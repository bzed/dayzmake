//! `.dzgeo` capture files (spec 3.8): the world params and the tiles of a run, so a run can be
//! replayed without a server.
//!
//! Layout, little-endian: `DZGC`, `u32 version = 1`, a block (`u32 size`, bytes) with the world params
//! payload (without the protocol version), then one block per tile (the message-5 payload).

use std::collections::BTreeMap;
use std::io::{self, Read, Write};

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
