//! `.dzgeo` capture files (spec 3.8): the world params and the tiles of a run, so a run can be
//! replayed without a server.
//!
//! Layout, little-endian: `DZGC`, `u32 version = 1`, a block (`u32 size`, bytes) with the world params
//! payload (without the protocol version), then one block per tile (the message-5 payload).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::error::ErrorExt;
use crate::navmesh::client::{MAX_FRAME, read_chunked};
use crate::navmesh::wire::{TileInput, WorldParams};
use crate::warning::{Warning, WarningSink};

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

/// The most tiles a [`CaptureFile`] indexes (the size of the largest plan of the generator).
const MAX_INDEXED: usize = 1 << 22;

/// What a [`CaptureFile`] can be read from: a file, or anything seekable.
trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// Options of [`CaptureFile::open`] and [`CaptureFile::from_reader`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CaptureFileOptions {
    /// Index only the tiles in this range `[x0, z0, x1, z1]` (clipped to the world like the range of
    /// [`generate`](crate::navmesh::generate::generate); a negative upper bound means the end of the grid).
    /// `None`: every tile of the world grid. Tiles outside the range read as absent.
    pub range: Option<[i32; 4]>,
}

impl CaptureFileOptions {
    /// Options that index the whole capture.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Index only the tiles in `range`.
    #[must_use]
    pub fn range(mut self, range: Option<[i32; 4]>) -> Self {
        self.range = range;
        self
    }
}

/// A capture that is indexed, not loaded: opening reads the world params and the `(x, z)` of every
/// record (8 bytes each, skipping the rest by seeking), and [`CaptureFile::tile`] reads one record on
/// demand. Memory is proportional to the number of indexed tiles (at most the range) plus the tiles in
/// flight, so a range of a multi-gigabyte capture needs only the tiles of that range. The format is the
/// one of [`Capture`]; use [`Capture::read`] only for small captures that you want in memory.
pub struct CaptureFile {
    params: WorldParams,
    index: BTreeMap<(i32, i32), Record>,
    file: Mutex<Box<dyn ReadSeek>>,
    path: Option<PathBuf>,
    /// The part of the grid that was indexed (`None`: the requested range was empty).
    indexed: Option<[i32; 4]>,
}

impl std::fmt::Debug for CaptureFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureFile")
            .field("tiles", &self.index.len())
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// The part of the grid `range` selects, as inclusive bounds; `None` when it is empty.
fn clip(range: Option<[i32; 4]>, tiles_x: i32, tiles_z: i32) -> Option<[i32; 4]> {
    let [rx0, rz0, rx1, rz1] = range.unwrap_or([0, 0, -1, -1]);
    let x1 = if rx1 < 0 {
        tiles_x.saturating_sub(1)
    } else {
        rx1.min(tiles_x.saturating_sub(1))
    };
    let z1 = if rz1 < 0 {
        tiles_z.saturating_sub(1)
    } else {
        rz1.min(tiles_z.saturating_sub(1))
    };
    let (x0, z0) = (rx0.max(0), rz0.max(0));
    (x0 <= x1 && z0 <= z1).then_some([x0, z0, x1, z1])
}

impl CaptureFile {
    /// Opens and indexes the capture file at `path`; see [`CaptureFile::from_reader`].
    ///
    /// # Errors
    ///
    /// The file cannot be opened, and the errors of [`CaptureFile::from_reader`].
    pub fn open(
        path: &Path,
        options: &CaptureFileOptions,
        warnings: &mut dyn WarningSink,
    ) -> io::Result<Self> {
        let mut capture = Self::from_reader(File::open(path)?, options, warnings)?;
        capture.path = Some(path.to_path_buf());
        Ok(capture)
    }

    /// Indexes a capture read from `reader`. A capture cut short keeps every complete record; a partial
    /// trailing block is ignored. A tile captured twice keeps the later copy. Records outside the world
    /// grid are ignored (one note says how many); records outside `options.range` are skipped. The
    /// records are not parsed here: a malformed record is reported by [`CaptureFile::tile`].
    ///
    /// # Errors
    ///
    /// The errors of [`Capture::read`] for the header and the world block, "invalid capture block size
    /// N", "message truncated" for a record too short to hold its coordinates, "capture has more
    /// records than the world has tiles" and "too many tiles to index".
    pub fn from_reader<R: Read + Seek + Send + 'static>(
        reader: R,
        options: &CaptureFileOptions,
        warnings: &mut dyn WarningSink,
    ) -> io::Result<Self> {
        let mut reader: Box<dyn ReadSeek> = Box::new(reader);
        let len = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut r = BufReader::new(reader);
        let truncated = || error!("capture truncated");
        let mut magic = [0u8; 4];
        if r.read_exact(&mut magic).is_err() || &magic != MAGIC {
            return Err(error!("not a capture file"));
        }
        let version = read_u32_opt(&mut r)?.ok_or_else(truncated)?;
        if version != VERSION {
            return Err(error!("unsupported capture version {version}"));
        }
        let block = read_block(&mut r)?.ok_or_else(truncated)?;
        let params = WorldParams::parse(&block).prepend_error("capture world params")?;
        let (tiles_x, tiles_z) = (params.world.tiles_x, params.world.tiles_z);
        let wanted = clip(options.range, tiles_x, tiles_z);
        let max_records = u64::try_from(i64::from(tiles_x) * i64::from(tiles_z)).unwrap_or(0);
        // 4 (magic) + 4 (version) + 4 (size) + the world block.
        let mut pos = u64::try_from(block.len())
            .ok()
            .and_then(|n| n.checked_add(12))
            .ok_or_else(truncated)?;
        let mut index = BTreeMap::new();
        let (mut n, mut outside) = (0u64, 0u64);
        while let Some(size) = read_u32_opt(&mut r)? {
            if size > MAX_FRAME {
                return Err(error!("invalid capture block size {size}"));
            }
            let start = pos.checked_add(4).ok_or_else(truncated)?;
            let end = start.checked_add(u64::from(size)).ok_or_else(truncated)?;
            if end > len {
                break;
            }
            if size < 8 {
                return Err(error!("capture tile record {n}: message truncated"));
            }
            n += 1;
            if n > max_records {
                return Err(error!(
                    "capture has more records than the world has tiles ({max_records})"
                ));
            }
            let mut coords = [0u8; 8];
            r.read_exact(&mut coords)
                .map_err(|_| error!("capture tile record {}: message truncated", n - 1))?;
            let x = i32::from_le_bytes([coords[0], coords[1], coords[2], coords[3]]);
            let z = i32::from_le_bytes([coords[4], coords[5], coords[6], coords[7]]);
            r.seek_relative(i64::from(size) - 8)?;
            pos = end;
            if !(0..tiles_x).contains(&x) || !(0..tiles_z).contains(&z) {
                outside += 1;
            } else if wanted
                .is_some_and(|[x0, z0, x1, z1]| (x0..=x1).contains(&x) && (z0..=z1).contains(&z))
            {
                index.insert(
                    (z, x),
                    Record {
                        offset: start,
                        size,
                    },
                );
                if index.len() > MAX_INDEXED {
                    return Err(error!(
                        "too many tiles to index (limit {MAX_INDEXED}); give a range"
                    ));
                }
            }
        }
        if outside > 0 {
            warnings.warn(Warning::note(format!(
                "capture: {outside} records outside the world grid were ignored"
            )));
        }
        Ok(Self {
            params,
            index,
            file: Mutex::new(r.into_inner()),
            path: None,
            indexed: wanted,
        })
    }

    /// The part of the world grid that was indexed, `[x0, z0, x1, z1]` inclusive: the range the file was
    /// opened with, clipped to the grid. `None` when that range was empty.
    #[must_use]
    pub fn indexed_range(&self) -> Option<[i32; 4]> {
        self.indexed
    }

    /// Whether every tile of `range` (clipped to the grid like the range of a run; `None` is the whole
    /// grid) was indexed, so that a tile this capture does not return is really missing from the file.
    #[must_use]
    pub fn covers(&self, range: Option<[i32; 4]>) -> bool {
        let (tiles_x, tiles_z) = (self.params.world.tiles_x, self.params.world.tiles_z);
        match (clip(range, tiles_x, tiles_z), self.indexed) {
            (Some([x0, z0, x1, z1]), Some([ix0, iz0, ix1, iz1])) => {
                x0 >= ix0 && z0 >= iz0 && x1 <= ix1 && z1 <= iz1
            }
            (None, _) => true,
            (Some(_), None) => false,
        }
    }

    /// The path of the file this was opened from; `None` for [`CaptureFile::from_reader`].
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The world params.
    #[must_use]
    pub fn params(&self) -> &WorldParams {
        &self.params
    }

    /// The number of tiles indexed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether no tile is indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The bounding range `[x0, z0, x1, z1]` of the indexed tiles, `None` when there is none.
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

    /// Reads and parses tile `(x, z)`; `None` when the capture (or the indexed range) lacks it.
    ///
    /// # Errors
    ///
    /// A read error keeps its kind (the file may be unreadable at the moment); a malformed record is
    /// [`io::ErrorKind::InvalidData`] with the errors of [`TileInput::parse`].
    pub fn tile(&self, x: i32, z: i32) -> io::Result<Option<TileInput>> {
        let Some(rec) = self.index.get(&(z, x)) else {
            return Ok(None);
        };
        let len = usize::try_from(rec.size).map_err(|_| error!("invalid capture block size"))?;
        let bytes = {
            let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
            file.seek(SeekFrom::Start(rec.offset))
                .and_then(|_| read_chunked(&mut *file, len))
        }
        .map_err(|e| io::Error::new(e.kind(), format!("capture tile ({x},{z}): {e}")))?;
        TileInput::parse(&bytes).map(Some).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("capture tile ({x},{z}): {e}"),
            )
        })
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
