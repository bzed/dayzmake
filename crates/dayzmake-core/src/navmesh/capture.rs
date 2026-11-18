//! `.dzgeo` capture files (spec 3.8): the world params and the tiles of a run, so a run can be
//! replayed without a server.
//!
//! Layout, little-endian: `DZGC`, `u32 version = 1`, a block (`u32 size`, bytes) with the world params
//! payload (without the protocol version), then one block per tile (the message-5 payload).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use crate::error::ErrorExt;
use crate::navmesh::client::{MAX_FRAME, read_chunked};
use crate::navmesh::wire::{TileInput, WorldParams};

const MAGIC: &[u8; 4] = b"DZGC";
const VERSION: u32 = 1;

/// A capture read from a `.dzgeo` file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Capture {
    params: WorldParams,
    tiles: Vec<TileInput>,
}

/// Reads a `u32` or reports `None` at a clean or partial end of the stream.
fn read_u32_opt(r: &mut impl Read) -> io::Result<Option<u32>> {
    let mut buf = [0u8; 4];
    let mut got = 0;
    while got < buf.len() {
        let Some(rest) = buf.get_mut(got..) else {
            break;
        };
        match r.read(rest) {
            Ok(0) => return Ok(None),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Some(u32::from_le_bytes(buf)))
}

/// Reads one block. `None` when the stream ends before the block is complete.
fn read_block(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let Some(size) = read_u32_opt(r)? else {
        return Ok(None);
    };
    if size > MAX_FRAME {
        return Err(error!("invalid capture block size {size}"));
    }
    let len = usize::try_from(size).map_err(|_| error!("invalid capture block size {size}"))?;
    match read_chunked(r, len) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(e),
    }
}

impl Capture {
    /// Reads a capture. A capture cut short keeps every complete tile; a partial trailing block is
    /// ignored. A tile captured twice keeps the later copy; tiles are sorted by `(z, x)`.
    ///
    /// # Errors
    ///
    /// "not a capture file" (bad magic), "unsupported capture version N", "capture truncated" (cut
    /// before the world block is complete), "invalid capture block size N" (above 1 GiB), and the
    /// errors of [`WorldParams::parse`] and [`TileInput::parse`] for a complete but malformed block.
    pub fn read(r: &mut impl Read) -> io::Result<Self> {
        let mut magic = Vec::new();
        r.take(4).read_to_end(&mut magic)?;
        if magic != MAGIC {
            return Err(error!("not a capture file"));
        }
        let truncated = || error!("capture truncated");
        let version = read_u32_opt(r)?.ok_or_else(truncated)?;
        if version != VERSION {
            return Err(error!("unsupported capture version {version}"));
        }
        let block = read_block(r)?.ok_or_else(truncated)?;
        let params = WorldParams::parse(&block).prepend_error("capture world params")?;
        let mut tiles: BTreeMap<(i32, i32), TileInput> = BTreeMap::new();
        let mut index = 0usize;
        while let Some(block) = read_block(r)? {
            let tile =
                TileInput::parse(&block).prepend_error(format!("capture tile record {index}"))?;
            tiles.insert((tile.z, tile.x), tile);
            index += 1;
        }
        Ok(Self {
            params,
            tiles: tiles.into_values().collect(),
        })
    }

    /// The world params.
    #[must_use]
    pub fn params(&self) -> &WorldParams {
        &self.params
    }

    /// The tiles, sorted by `(z, x)`.
    #[must_use]
    pub fn tiles(&self) -> &[TileInput] {
        &self.tiles
    }

    /// The captured tile `(x, z)`.
    #[must_use]
    pub fn tile(&self, x: i32, z: i32) -> Option<&TileInput> {
        self.tiles
            .binary_search_by_key(&(z, x), |t| (t.z, t.x))
            .ok()
            .and_then(|i| self.tiles.get(i))
    }
}

/// Where one tile record of a capture file lies.
#[derive(Debug, Clone, Copy)]
struct Record {
    /// Offset of the payload (after the block's size field).
    offset: u64,
    /// Payload size.
    size: u32,
}

/// A capture file that is indexed, not loaded: opening reads the world params and the `(x, z)` of every
/// record (8 bytes each, skipping the rest by seeking), and [`CaptureFile::tile`] reads one record on
/// demand. Memory is proportional to the number of records plus the tiles in flight, so a range of a
/// multi-gigabyte capture needs only the tiles of that range. The file format is the one of [`Capture`].
#[derive(Debug)]
pub struct CaptureFile {
    params: WorldParams,
    index: BTreeMap<(i32, i32), Record>,
    file: Mutex<File>,
}

impl CaptureFile {
    /// Opens and indexes a capture. A capture cut short keeps every complete record; a partial trailing
    /// block is ignored. A tile captured twice keeps the later copy. The records are not parsed here: a
    /// malformed record is reported by [`CaptureFile::tile`].
    ///
    /// # Errors
    ///
    /// The errors of [`Capture::read`] for the header and the world block, "invalid capture block size N",
    /// and "message truncated" for a record too short to hold its coordinates.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut r = BufReader::new(file);
        let mut magic = [0u8; 4];
        let truncated = || error!("capture truncated");
        if r.read_exact(&mut magic).is_err() || &magic != MAGIC {
            return Err(error!("not a capture file"));
        }
        let version = read_u32_opt(&mut r)?.ok_or_else(truncated)?;
        if version != VERSION {
            return Err(error!("unsupported capture version {version}"));
        }
        let block = read_block(&mut r)?.ok_or_else(truncated)?;
        let params = WorldParams::parse(&block).prepend_error("capture world params")?;
        // 4 (magic) + 4 (version) + 4 (size) + the world block.
        let mut pos = 12u64 + u64::try_from(block.len()).map_err(|_| truncated())?;
        let mut index = BTreeMap::new();
        let mut n = 0usize;
        while let Some(size) = read_u32_opt(&mut r)? {
            if size > MAX_FRAME {
                return Err(error!("invalid capture block size {size}"));
            }
            let start = pos + 4;
            let end = start + u64::from(size);
            if end > len {
                break;
            }
            if size < 8 {
                return Err(error!("capture tile record {n}: message truncated"));
            }
            let mut coords = [0u8; 8];
            r.read_exact(&mut coords)
                .map_err(|_| error!("capture tile record {n}: message truncated"))?;
            let x = i32::from_le_bytes([coords[0], coords[1], coords[2], coords[3]]);
            let z = i32::from_le_bytes([coords[4], coords[5], coords[6], coords[7]]);
            r.seek_relative(i64::from(size) - 8)?;
            index.insert(
                (z, x),
                Record {
                    offset: start,
                    size,
                },
            );
            pos = end;
            n += 1;
        }
        Ok(Self {
            params,
            index,
            file: Mutex::new(r.into_inner()),
        })
    }

    /// The world params.
    #[must_use]
    pub fn params(&self) -> &WorldParams {
        &self.params
    }

    /// The number of tiles in the capture.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether the capture holds no tile.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The bounding range `[x0, z0, x1, z1]` of the tiles, `None` for an empty capture.
    #[must_use]
    pub fn bounds(&self) -> Option<[i32; 4]> {
        let mut keys = self.index.keys();
        let &(z, x) = keys.next()?;
        let mut r = [x, z, x, z];
        for &(z, x) in keys {
            r = [r[0].min(x), r[1].min(z), r[2].max(x), r[3].max(z)];
        }
        Some(r)
    }

    /// Reads and parses tile `(x, z)`; `None` when the capture lacks it.
    ///
    /// # Errors
    ///
    /// Read errors, and the errors of [`TileInput::parse`] for a malformed record.
    pub fn tile(&self, x: i32, z: i32) -> io::Result<Option<TileInput>> {
        let Some(rec) = self.index.get(&(z, x)) else {
            return Ok(None);
        };
        let len = usize::try_from(rec.size).map_err(|_| error!("invalid capture block size"))?;
        let bytes = {
            let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
            file.seek(SeekFrom::Start(rec.offset))?;
            read_chunked(&mut *file, len)
        }
        .prepend_error(format!("capture tile ({x},{z})"))?;
        TileInput::parse(&bytes)
            .map(Some)
            .prepend_error(format!("capture tile ({x},{z})"))
    }
}

/// Writes a capture, flushing after every tile so a crashed run keeps its prefix.
#[derive(Debug)]
pub struct CaptureWriter<W: Write> {
    w: W,
}

fn write_block(w: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let size = u32::try_from(bytes.len())
        .ok()
        .filter(|n| *n <= MAX_FRAME)
        .ok_or_else(|| error!("capture block too large"))?;
    let mut out = Vec::with_capacity(bytes.len() + 4);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(bytes);
    w.write_all(&out)?;
    w.flush()
}

impl<W: Write> CaptureWriter<W> {
    /// Writes the header and the world params block.
    ///
    /// # Errors
    ///
    /// Fails when writing fails.
    pub fn new(mut w: W, params: &WorldParams) -> io::Result<Self> {
        w.write_all(MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        write_block(&mut w, &params.encode())?;
        Ok(Self { w })
    }

    /// Appends one tile and flushes.
    ///
    /// # Errors
    ///
    /// Fails when writing fails or the tile is larger than 1 GiB.
    pub fn add(&mut self, tile: &TileInput) -> io::Result<()> {
        write_block(&mut self.w, &tile.encode())
    }
}
