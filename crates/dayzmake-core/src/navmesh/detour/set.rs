//! The `TESM` tile mesh set file (spec 9.1, 9.2, 9.4, 9.6) and the 64-bit polygon reference
//! (spec 8.1).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

use crate::error::ErrorExt;
use crate::navmesh::bytes::Reader;
use crate::navmesh::detour::tile::TileData;

/// The set file magic: the bytes `TESM`.
pub const SET_MAGIC: i32 = i32::from_le_bytes(*b"TESM");
/// The set version DayZ 1.30 accepts; the only one written.
pub const SET_VERSION: i32 = 9;
/// The set version of DayZ 1.29 and earlier. Same layout; accepted when reading.
pub const SET_VERSION_OLD: i32 = 7;

const SET_HEADER_LEN: usize = 48;
const RECORD_HEADER_LEN: usize = 16;
const EMPTY_SLOT: u32 = 0xFFFF_FFFF;
/// Reads of a record blob grow the buffer in steps of at most this size.
const READ_CHUNK: usize = 1 << 20;

const SALT_MASK: u32 = 0xFFFF;
const TILE_MASK: u32 = 0x0FFF_FFFF;
const POLY_MASK: u32 = 0x000F_FFFF;

/// Packs a 64-bit polygon reference: `salt << 48 | tile << 20 | poly` (16, 28 and 20 bits; wider
/// values are masked).
#[must_use]
pub fn encode_poly_ref(salt: u32, tile: u32, poly: u32) -> u64 {
    (u64::from(salt & SALT_MASK) << 48)
        | (u64::from(tile & TILE_MASK) << 20)
        | u64::from(poly & POLY_MASK)
}

/// Splits a polygon reference into `(salt, tile, poly)`.
#[must_use]
pub fn decode_poly_ref(r: u64) -> (u32, u32, u32) {
    let field = |shift: u32, mask: u32| u32::try_from((r >> shift) & u64::from(mask)).unwrap_or(0);
    (
        field(48, SALT_MASK),
        field(20, TILE_MASK),
        field(0, POLY_MASK),
    )
}

/// `dtNavMeshParams` as stored in the set header.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct NavMeshParams {
    /// Origin (0, 0, 0).
    pub orig: [f32; 3],
    /// Tile width, metres.
    pub tile_width: f32,
    /// Tile height (depth), metres.
    pub tile_height: f32,
    /// Number of tile slots.
    pub max_tiles: i32,
    /// Polygon limit per tile (`1 << 20`).
    pub max_polys: i32,
}

/// One tile record of a set file (spec 9.2).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct SetTile {
    /// `salt << 48 | slot << 20`.
    pub tile_ref: u64,
    /// The padding word of the record: 0 in shipped maps, preserved on rewrite.
    pub padding: u32,
    /// The Detour tile.
    pub data: TileData,
}

/// A `.nm` file: the mesh parameters and the tile records in file order.
///
/// Reading is sequential, as the game does it: the offset table is skipped (its size comes from
/// `maxTiles`) and its entries are not used, and `numTiles` records follow back to back. Writing
/// always produces version [`SET_VERSION`] and recomputes the offset table.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct NavMeshSet {
    /// The set version. Reading normalises version 7 to [`SET_VERSION`]; writing ignores this field.
    pub version: i32,
    /// Grid width in tiles.
    pub tiles_x: i32,
    /// Grid depth in tiles.
    pub tiles_z: i32,
    /// The mesh parameters.
    pub params: NavMeshParams,
    /// The tile records, in file order.
    pub tiles: Vec<SetTile>,
}

fn file_truncated() -> io::Error {
    error!("navmesh file truncated")
}

/// Reads `len` bytes, growing the buffer in chunks, so that a length field larger than the file
/// cannot make the reader allocate more than the file holds.
fn read_chunked(r: &mut impl Read, len: usize) -> io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    while buf.len() < len {
        let start = buf.len();
        let end = start.saturating_add(READ_CHUNK).min(len);
        buf.resize(end, 0);
        let Some(chunk) = buf.get_mut(start..end) else {
            return Ok(None);
        };
        match r.read_exact(chunk) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    Ok(Some(buf))
}

impl NavMeshSet {
    /// Reads a set file. Rejects a bad magic, a version other than 7 or 9, `numTiles > maxTiles`,
    /// negative counts, a record with `tileRef == 0` or `dataSize <= 0`, a record running past the
    /// end of the input, and any tile blob that [`TileData::parse`] rejects (spec 9.4).
    ///
    /// The input is consumed up to the end of the last record; nothing is allocated from a length
    /// field before the input has delivered that many bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for any of the rejections above, or when the reader fails.
    pub fn read(r: &mut impl Read) -> io::Result<Self> {
        let mut head = [0u8; SET_HEADER_LEN];
        r.read_exact(&mut head).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                file_truncated()
            } else {
                e
            }
        })?;
        let mut h = Reader::new(&head);
        let magic = h.i32()?;
        if magic != SET_MAGIC {
            return Err(error!("Invalid header format (bad magic)"));
        }
        let version = h.i32()?;
        if version != SET_VERSION && version != SET_VERSION_OLD {
            return Err(error!("Invalid version {version}"));
        }
        let num_tiles = h.i32()?;
        let tiles_x = h.i32()?;
        let tiles_z = h.i32()?;
        let params = NavMeshParams {
            orig: h.f32x3()?,
            tile_width: h.f32()?,
            tile_height: h.f32()?,
            max_tiles: h.i32()?,
            max_polys: h.i32()?,
        };
        let max_tiles = u32::try_from(params.max_tiles)
            .map_err(|_| error!("negative maxTiles {}", params.max_tiles))?;
        let num_tiles =
            u32::try_from(num_tiles).map_err(|_| error!("negative numTiles {num_tiles}"))?;
        if num_tiles > max_tiles {
            return Err(error!("numTiles {num_tiles} exceeds maxTiles {max_tiles}"));
        }

        // The offset table is not needed: the records follow back to back. Skip it without
        // allocating; a file shorter than the table is rejected.
        let table = u64::from(max_tiles) * 4;
        let skipped = io::copy(&mut r.by_ref().take(table), &mut io::sink())?;
        if skipped != table {
            return Err(file_truncated());
        }

        let mut tiles = Vec::new();
        for i in 0..num_tiles {
            let mut rec = [0u8; RECORD_HEADER_LEN];
            r.read_exact(&mut rec).map_err(|e| {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    error!("tile record {i} runs past the end of the file")
                } else {
                    e
                }
            })?;
            let mut rh = Reader::new(&rec);
            let tile_ref = rh.u64()?;
            let data_size = rh.i32()?;
            let padding = rh.u32()?;
            if tile_ref == 0 {
                return Err(error!("tile record {i} has tileRef 0"));
            }
            let size = usize::try_from(data_size)
                .ok()
                .filter(|&n| n > 0)
                .ok_or_else(|| error!("tile record {i} has dataSize {data_size}"))?;
            let blob = read_chunked(r, size)?
                .ok_or_else(|| error!("tile record {i} runs past the end of the file"))?;
            let data = TileData::parse(&blob).prepend_error(format!("tile record {i}:"))?;
            tiles.push(SetTile {
                tile_ref,
                padding,
                data,
            });
        }
        Ok(Self {
            version: SET_VERSION,
            tiles_x,
            tiles_z,
            params,
            tiles,
        })
    }

    /// Writes the file: version [`SET_VERSION`], the offset table of spec 9.1 (offsets relative to
    /// the first record, `0xFFFFFFFF` for empty slots) and the records in `tiles` order.
    ///
    /// # Errors
    ///
    /// Returns an error, before writing anything, for a negative `maxTiles`, more tiles than
    /// `maxTiles`, a `tile_ref` of 0, a tile whose slot is `>= maxTiles` or used twice, or offsets
    /// above `0xFFFFFFFE`; or when the writer fails.
    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let max_tiles = u32::try_from(self.params.max_tiles)
            .map_err(|_| error!("negative maxTiles {}", self.params.max_tiles))?;
        let too_many = || error!("{} tiles exceed maxTiles {max_tiles}", self.tiles.len());
        let num_tiles = u32::try_from(self.tiles.len()).map_err(|_| too_many())?;
        if num_tiles > max_tiles {
            return Err(too_many());
        }
        let mut offsets: BTreeMap<u32, u32> = BTreeMap::new();
        let mut sizes = Vec::with_capacity(self.tiles.len());
        let mut offset: u64 = 0;
        for t in &self.tiles {
            if t.tile_ref == 0 {
                return Err(error!("tile with tileRef 0"));
            }
            let slot = decode_poly_ref(t.tile_ref).1;
            if slot >= max_tiles {
                return Err(error!("tile slot {slot} is not below maxTiles {max_tiles}"));
            }
            let size = t
                .data
                .blob_len()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or_else(|| error!("tile in slot {slot} is too large"))?;
            let at = u32::try_from(offset)
                .ok()
                .filter(|&o| o != EMPTY_SLOT)
                .ok_or_else(|| error!("tile offset {offset} does not fit the offset table"))?;
            if offsets.insert(slot, at).is_some() {
                return Err(error!("two tiles in slot {slot}"));
            }
            sizes.push(size);
            offset +=
                u64::try_from(RECORD_HEADER_LEN).unwrap_or(16) + u64::from(size.unsigned_abs());
        }

        let mut head = Vec::with_capacity(SET_HEADER_LEN);
        head.extend_from_slice(&SET_MAGIC.to_le_bytes());
        head.extend_from_slice(&SET_VERSION.to_le_bytes());
        head.extend_from_slice(&num_tiles.to_le_bytes());
        head.extend_from_slice(&self.tiles_x.to_le_bytes());
        head.extend_from_slice(&self.tiles_z.to_le_bytes());
        for f in self.params.orig {
            head.extend_from_slice(&f.to_le_bytes());
        }
        head.extend_from_slice(&self.params.tile_width.to_le_bytes());
        head.extend_from_slice(&self.params.tile_height.to_le_bytes());
        head.extend_from_slice(&self.params.max_tiles.to_le_bytes());
        head.extend_from_slice(&self.params.max_polys.to_le_bytes());
        w.write_all(&head)?;

        // The table, in chunks: maxTiles can be large even when few tiles are present.
        let mut chunk = Vec::with_capacity(16 * 1024);
        for slot in 0..max_tiles {
            let v = offsets.get(&slot).copied().unwrap_or(EMPTY_SLOT);
            chunk.extend_from_slice(&v.to_le_bytes());
            if chunk.len() >= 16 * 1024 {
                w.write_all(&chunk)?;
                chunk.clear();
            }
        }
        w.write_all(&chunk)?;

        for (t, size) in self.tiles.iter().zip(sizes) {
            let blob = t.data.to_bytes();
            if i32::try_from(blob.len()).ok() != Some(size) {
                return Err(error!("tile blob size changed while writing"));
            }
            w.write_all(&t.tile_ref.to_le_bytes())?;
            w.write_all(&size.to_le_bytes())?;
            w.write_all(&t.padding.to_le_bytes())?;
            w.write_all(&blob)?;
        }
        Ok(())
    }

    /// Writes the file atomically (spec 9.6, ruling 11): into a uniquely named temporary file in the
    /// target's directory, then `sync_all`, then a rename over `path`. An interrupted or failed
    /// write leaves the previous file untouched and removes the temporary file.
    ///
    /// # Errors
    ///
    /// Returns an error when [`NavMeshSet::write`] fails or the file cannot be created, synced or
    /// renamed.
    pub fn write_atomic(&self, path: &Path) -> io::Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| error!("{} has no file name", path.display()))?
            .to_string_lossy()
            .into_owned();
        let dir = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let pid = std::process::id();
        let mut n = 0u32;
        let (tmp, file) = loop {
            let tmp = dir.join(format!(".{name}.{pid}.{n}.tmp"));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
            {
                Ok(f) => break (tmp, f),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n < 1000 => n += 1,
                Err(e) => return Err(e),
            }
        };
        let result = (|| {
            let mut w = BufWriter::new(file);
            self.write(&mut w)?;
            let file = w.into_inner().map_err(io::IntoInnerError::into_error)?;
            file.sync_all()?;
            fs::rename(&tmp, path)
        })();
        if result.is_err() {
            // Best effort: the original error is what the caller needs.
            let _ = fs::remove_file(&tmp);
        }
        result.prepend_error(format!("writing {}:", path.display()))
    }
}
