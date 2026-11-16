//! Functions for working with PAA files.
//!
//! Format (as used by DayZ / Arma 3, verified against engine-loaded files):
//!
//! ```text
//! u16  magic (compression: 0xff01 DXT1, 0xff02 DXT2, 0xff03 DXT3,
//!             0xff04 DXT4, 0xff05 DXT5, 0x8888 RGBA, ...)
//! taggs: 8-byte signature (e.g. "GGATSFFO") + u32 data size + data,
//!        repeated until a zero byte (which is the low byte of the
//!        palette length following them)
//! u16  palette size in bytes (+ palette data if non-zero)
//! mipmaps: u16 width (bit 0x8000 = LZO compressed) + u16 height +
//!          3-byte (little endian) data size + data, repeated until a
//!          zero width
//! u16 x3 zero terminator
//! ```

use std::io::{Error, Read, Seek, SeekFrom, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
#[cfg(feature = "image")]
use image::{DynamicImage, ImageBuffer, Rgba};
use linked_hash_map::LinkedHashMap;
use lzo1x;
use texpresso::Format;

use crate::error::printable;
#[cfg(feature = "image")]
use texpresso::{Algorithm, Params};

const OFFS_TAGG: &str = "GGATSFFO"; // mipmap offsets (16 u32, zero padded)

/// Largest mipmap width or height accepted when reading.
const MAX_PAA_DIMENSION: usize = 16384;

/// Most mipmaps accepted when reading (real PAAs have at most 16: 16384 down to 1 pixel is 15).
const MAX_MIPMAPS: usize = 16;

/// Most mipmap bytes accepted in total when reading, after LZO decompression. A tiny file can
/// otherwise expand to 256 times its size.
const MAX_PAA_TOTAL: usize = 256 << 20;

/// Largest tagg accepted when reading.
const MAX_TAGG_SIZE: usize = 1 << 20;

/// Reads the data of a tagg without trusting its size.
fn read_tagg_data<I: Read>(input: &mut I, name: &str) -> Result<Vec<u8>, Error> {
    let tagg_size = input.read_u32::<LittleEndian>()? as usize;
    if tagg_size > MAX_TAGG_SIZE {
        return Err(error!(
            "PAA tagg '{}' is too large: {tagg_size} bytes.",
            printable(name)
        ));
    }
    let mut data = Vec::new();
    input.take(tagg_size as u64).read_to_end(&mut data)?;
    if data.len() != tagg_size {
        return Err(error!("Truncated PAA tagg '{}'.", printable(name)));
    }
    Ok(data)
}

/// Decompresses an LZO mipmap. The decompressed size is the format's usual compressed (block)
/// size for the mipmap dimensions.
fn decompress_mipmap(
    compression: &str,
    mip_data: &[u8],
    w: u16,
    h: u16,
    budget: usize,
) -> Result<Vec<u8>, Error> {
    let real_w = (w & 0x7FFF) as usize;
    let real_h = h as usize;
    let decompressed_size = match compression {
        "DXT1" => Format::Bc1.compressed_size(real_w, real_h),
        "DXT2" | "DXT3" => Format::Bc2.compressed_size(real_w, real_h),
        "DXT4" | "DXT5" => Format::Bc3.compressed_size(real_w, real_h),
        _ => {
            return Err(error!(
                "LZO compression is only supported for DXT formats, not '{}'",
                compression
            ));
        }
    };
    if decompressed_size > mip_data.len().saturating_mul(256) {
        return Err(error!(
            "PAA mipmap claims an implausible size: {decompressed_size} bytes from {} bytes.",
            mip_data.len()
        ));
    }
    if decompressed_size > budget {
        return Err(error!(
            "PAA mipmaps are too large: more than {MAX_PAA_TOTAL} bytes in total."
        ));
    }
    let mut decompressed_data = vec![0; decompressed_size];
    lzo1x::decompress(mip_data, &mut decompressed_data)
        .map_err(|e| error!("Failed to decompress LZO-compressed mipmap: {:?}", e))?;
    Ok(decompressed_data)
}

/// Reads one mipmap of `size` stored bytes (LZO compressed if bit 15 of the width is set).
/// `budget` is what is left of the total mipmap size allowed for the file.
fn read_mipmap<I: Read>(
    input: &mut I,
    compression: &str,
    (w, h): (u16, u16),
    size: usize,
    budget: usize,
) -> Result<Vec<u8>, Error> {
    // the size field is not trusted for preallocation
    let mut mip_data = Vec::new();
    input.take(size as u64).read_to_end(&mut mip_data)?;
    if mip_data.len() != size {
        return Err(error!("Truncated PAA mipmap."));
    }
    if (w & 0x8000) != 0 {
        decompress_mipmap(compression, &mip_data, w, h, budget)
    } else if size > budget {
        Err(error!(
            "PAA mipmaps are too large: more than {MAX_PAA_TOTAL} bytes in total."
        ))
    } else {
        Ok(mip_data)
    }
}

/// The size of an uncompressed mipmap of a DXT texture, `None` for other formats.
fn raw_mipmap_size(compression: &str, width: usize, height: usize) -> Option<usize> {
    match compression {
        "DXT1" => Some(Format::Bc1.compressed_size(width, height)),
        "DXT2" | "DXT3" => Some(Format::Bc2.compressed_size(width, height)),
        "DXT4" | "DXT5" => Some(Format::Bc3.compressed_size(width, height)),
        _ => None,
    }
}

/// A format a PAA can be created in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PaaFormat {
    /// DXT1 (BC1), no or 1-bit alpha.
    Dxt1,
    /// DXT3 (BC2), explicit 4-bit alpha.
    Dxt3,
    /// DXT5 (BC3), interpolated alpha.
    Dxt5,
    /// Uncompressed 8-bit RGBA.
    Rgba,
}

impl std::str::FromStr for PaaFormat {
    type Err = Error;

    /// Parses `dxt1`, `dxt3`, `dxt5` or `rgba`, ignoring case.
    fn from_str(s: &str) -> Result<Self, Error> {
        match s.to_lowercase().as_str() {
            "dxt1" => Ok(PaaFormat::Dxt1),
            "dxt3" => Ok(PaaFormat::Dxt3),
            "dxt5" => Ok(PaaFormat::Dxt5),
            "rgba" => Ok(PaaFormat::Rgba),
            _ => Err(error!("Unsupported PAA compression type for creation: {s}")),
        }
    }
}

impl PaaFormat {
    /// The compression name the format has in a PAA.
    #[cfg(feature = "image")]
    fn name(self) -> &'static str {
        match self {
            PaaFormat::Dxt1 => "DXT1",
            PaaFormat::Dxt3 => "DXT3",
            PaaFormat::Dxt5 => "DXT5",
            PaaFormat::Rgba => "RGBA",
        }
    }
}

/// One mipmap: its stored bytes and whether they are an LZO stream.
#[derive(Debug)]
struct Mipmap {
    data: Vec<u8>,
    lzo: bool,
}

/// A PAA texture.
#[derive(Debug)]
pub struct Paa {
    /// Tagg sections in file order (name → data).
    taggs: LinkedHashMap<String, Vec<u8>>,
    /// Palette data, if the texture is paletted.
    palette: Option<Vec<u8>>,
    /// Mipmap data, largest first.
    mipmaps: Vec<Mipmap>,
    /// Width of the largest mipmap.
    width: u16,
    /// Height of the largest mipmap.
    height: u16,
    /// Compression format name (e.g. "DXT5").
    compression: String,
}

impl Paa {
    /// Width of the largest mipmap in pixels.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Height of the largest mipmap in pixels.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Compression format name as read from the file, e.g. `"DXT5"`.
    #[must_use]
    pub fn compression(&self) -> &str {
        &self.compression
    }

    /// Number of mipmaps.
    #[must_use]
    pub fn mipmap_count(&self) -> usize {
        self.mipmaps.len()
    }

    /// The data of the tagg called `name`, if the file has one.
    #[must_use]
    pub fn tagg(&self, name: &str) -> Option<&[u8]> {
        self.taggs.get(name).map(Vec::as_slice)
    }

    /// Reads a PAA texture.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not a supported PAA file or is malformed.
    pub fn read<I: Read + Seek>(input: &mut I) -> Result<Paa, Error> {
        // The top-level read function. We seek to the beginning to ensure consistent state,
        // as this might be called on a stream that has been partially read (e.g. by another
        // part of the program trying to determine file type).
        input.seek(SeekFrom::Start(0))?;

        let magic_number = input.read_u16::<LittleEndian>()?;
        let compression = match magic_number {
            0xff01 => "DXT1",
            0xff02 => "DXT2",
            0xff03 => "DXT3",
            0xff04 => "DXT4",
            0xff05 => "DXT5",
            0x8888 => "RGBA",
            0x4444 => return Err(error!("Unsupported PAA format: RGBA4444 (0x4444)")),
            0x1555 => return Err(error!("Unsupported PAA format: RGBA5551 (0x1555)")),
            0x8080 => return Err(error!("Unsupported PAA format: GRAYwAlpha (0x8080)")),
            _ => return Err(error!("Invalid PAA magic number: 0x{:04x}", magic_number)),
        }
        .to_string();

        let mut taggs = LinkedHashMap::new();
        let mut mipmaps = Vec::new();
        let mut width = 0;
        let mut height = 0;
        let mut total = 0usize;

        // Taggs. The section is terminated by a zero byte, which is not
        // consumed: it is the low byte of the palette size that follows.
        loop {
            let mut next_byte = [0u8; 1];
            if input.read(&mut next_byte)? == 0 {
                break;
            }
            if next_byte[0] == 0 {
                input.seek(SeekFrom::Current(-1))?;
                break;
            }
            input.seek(SeekFrom::Current(-1))?;

            let mut tagg_name_bytes = [0u8; 8];
            input.read_exact(&mut tagg_name_bytes)?;
            let tagg_name = String::from_utf8_lossy(&tagg_name_bytes)
                .trim_end_matches('\0')
                .to_string();

            let data = read_tagg_data(input, &tagg_name)?;

            taggs.insert(tagg_name, data);
        }

        let palette_len = input.read_u16::<LittleEndian>()? as usize;
        let palette = if palette_len > 0 {
            let mut pal_data = vec![0; palette_len];
            input.read_exact(&mut pal_data)?;
            Some(pal_data)
        } else {
            None
        };

        // Mipmaps
        loop {
            let w = match input.read_u16::<LittleEndian>() {
                Ok(0) | Err(_) => break,
                Ok(w) => w,
            };
            let h = input.read_u16::<LittleEndian>()?;

            let mut size_bytes = [0u8; 3];
            input.read_exact(&mut size_bytes)?;
            let size = (size_bytes[0] as usize)
                | ((size_bytes[1] as usize) << 8)
                | ((size_bytes[2] as usize) << 16);

            if usize::from(w & 0x7FFF) > MAX_PAA_DIMENSION || usize::from(h) > MAX_PAA_DIMENSION {
                return Err(error!(
                    "PAA mipmap dimensions are too large: {} x {}.",
                    w & 0x7FFF,
                    h
                ));
            }

            if width == 0 {
                width = w & 0x7FFF;
            }
            if height == 0 {
                height = h;
            }

            if mipmaps.len() >= MAX_MIPMAPS {
                return Err(error!("PAA has more than {MAX_MIPMAPS} mipmaps."));
            }
            let budget = MAX_PAA_TOTAL.saturating_sub(total);
            let mip = read_mipmap(input, &compression, (w, h), size, budget)?;
            total += mip.len();
            mipmaps.push(Mipmap {
                data: mip,
                lzo: false,
            });
        }

        if mipmaps.is_empty() {
            return Err(error!("PAA contains no mipmap data."));
        }

        Ok(Paa {
            taggs,
            palette,
            mipmaps,
            width,
            height,
            compression,
        })
    }

    /// Decodes the largest mipmap into an image.
    ///
    /// # Errors
    ///
    /// Returns an error if the PAA has no image data, the format is unsupported
    /// or the data does not fit the declared dimensions.
    #[cfg_attr(docsrs, doc(cfg(feature = "image")))]
    #[cfg(feature = "image")]
    pub fn to_dynamic_image(&self) -> Result<DynamicImage, Error> {
        let Some(first) = self.mipmaps.first() else {
            return Err(error!("PAA has no image data"));
        };
        let decompressed_mipmap;
        let main_mipmap: &Vec<u8> = if first.lzo {
            decompressed_mipmap = decompress_mipmap(
                &self.compression,
                &first.data,
                self.width,
                self.height,
                MAX_PAA_TOTAL,
            )?;
            &decompressed_mipmap
        } else {
            &first.data
        };
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        // the decoded RGBA image is 4 bytes per pixel; hold it to the same bound as the mipmaps
        // (at most 8192 x 8192 pixels)
        if width.saturating_mul(height).saturating_mul(4) > MAX_PAA_TOTAL {
            return Err(error!(
                "PAA is too large to decode: {width} x {height} pixels."
            ));
        }

        let format = match self.compression.as_str() {
            "DXT1" => Format::Bc1,
            "DXT2" | "DXT3" => Format::Bc2,
            "DXT4" | "DXT5" => Format::Bc3,
            "PAL" => {
                let palette = self
                    .palette
                    .as_ref()
                    .ok_or_else(|| error!("Paletted PAA without palette data"))?;
                let mut rgba_data = Vec::with_capacity(width * height * 4);
                for &index in main_mipmap {
                    let i = usize::from(index) * 4;
                    // Palette is BGRA
                    let Some(&[blue, green, red, alpha]) = palette.get(i..i + 4) else {
                        return Err(error!("Palette index out of bounds"));
                    };
                    rgba_data.push(red);
                    rgba_data.push(green);
                    rgba_data.push(blue);
                    rgba_data.push(alpha);
                }
                let img = Self::rgba_image(width, height, rgba_data)?;
                return Ok(DynamicImage::ImageRgba8(img));
            }
            "RGBA" => {
                if main_mipmap.len() == width * height * 4 {
                    let img = Self::rgba_image(width, height, main_mipmap.clone())?;
                    return Ok(DynamicImage::ImageRgba8(img));
                }
                return Err(error!("Invalid uncompressed RGBA data size"));
            }
            _ => {
                return Err(error!(
                    "Unsupported PAA compression format: {}",
                    self.compression
                ));
            }
        };

        if width == 0 || height == 0 {
            return Err(error!("PAA has no image data"));
        }
        // the block decoder indexes the data by the dimensions, so it must hold every block
        if main_mipmap.len() < format.compressed_size(width, height) {
            return Err(error!(
                "PAA mipmap holds {} bytes, but {width} x {height} {} needs {}.",
                main_mipmap.len(),
                self.compression,
                format.compressed_size(width, height)
            ));
        }
        let mut decompressed = vec![0u8; width * height * 4];
        format.decompress(main_mipmap, width, height, &mut decompressed);

        let img = Self::rgba_image(width, height, decompressed)?;
        Ok(DynamicImage::ImageRgba8(img))
    }

    /// Builds an RGBA image, checking that the data fits the dimensions.
    ///
    /// # Errors
    ///
    /// Returns an error if `data` does not hold `width * height` pixels.
    #[cfg(feature = "image")]
    fn rgba_image(
        width: usize,
        height: usize,
        data: Vec<u8>,
    ) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>, Error> {
        let dims = u32::try_from(width).ok().and_then(|w| {
            u32::try_from(height)
                .ok()
                .and_then(|h| ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(w, h, data))
        });
        dims.ok_or_else(|| error!("Image data does not match dimensions"))
    }

    /// Encodes an image as a PAA texture, building the mipmap chain down to 4x4.
    ///
    /// # Errors
    ///
    /// Returns an error for images too large for the format.
    #[cfg_attr(docsrs, doc(cfg(feature = "image")))]
    #[cfg(feature = "image")]
    pub fn from_dynamic_image(img: &DynamicImage, format: PaaFormat) -> Result<Paa, Error> {
        let mut current = img.to_rgba8();
        let compression = format.name().to_string();

        // Build the mipmap chain down to 4x4.
        let mut levels: Vec<ImageBuffer<Rgba<u8>, Vec<u8>>> = vec![current.clone()];
        loop {
            let (w, h) = current.dimensions();
            if w <= 4 || h <= 4 {
                break;
            }
            current = image::imageops::resize(
                &current,
                w / 2,
                h / 2,
                image::imageops::FilterType::Triangle,
            );
            levels.push(current.clone());
        }

        let params = Params {
            algorithm: Algorithm::RangeFit,
            weights: [1.0, 1.0, 1.0],
            weigh_colour_by_alpha: false,
        };

        let mut mipmaps = Vec::new();
        for level in &levels {
            let (w, h) = level.dimensions();
            let data = level.as_raw().clone();
            let mipmap = match compression.as_str() {
                "DXT1" => {
                    let mut out = vec![0; Format::Bc1.compressed_size(w as usize, h as usize)];
                    Format::Bc1.compress(&data, w as usize, h as usize, params, &mut out);
                    out
                }
                "DXT3" => {
                    let mut out = vec![0; Format::Bc2.compressed_size(w as usize, h as usize)];
                    Format::Bc2.compress(&data, w as usize, h as usize, params, &mut out);
                    out
                }
                "DXT5" => {
                    let mut out = vec![0; Format::Bc3.compressed_size(w as usize, h as usize)];
                    Format::Bc3.compress(&data, w as usize, h as usize, params, &mut out);
                    out
                }
                "RGBA" => data,
                _ => {
                    return Err(error!(
                        "Unsupported PAA compression type for creation: {compression}"
                    ));
                }
            };
            mipmaps.push(Mipmap {
                data: mipmap,
                lzo: false,
            });
        }

        let Some(first) = levels.first() else {
            return Err(error!("no mipmaps were built"));
        };
        let (width, height) = first.dimensions();
        let to_u16 =
            |dim: u32| u16::try_from(dim).map_err(|_| error!("image too large for PAA: {dim}px"));

        let mut paa = Paa {
            taggs: LinkedHashMap::new(),
            palette: None,
            mipmaps,
            width: to_u16(width)?,
            height: to_u16(height)?,
            compression,
        };
        // LZO compress large mipmaps like the engine's own tools do.
        paa.lzo_compress()?;
        Ok(paa)
    }

    /// LZO-compresses the mipmaps wider than 128 pixels of a DXT texture.
    ///
    /// A mipmap is replaced by its compressed form only if that is shorter, so the others stay raw
    /// (and [`Paa::write`] leaves them unmarked). Mipmaps that are already compressed, mipmaps of 128
    /// pixels or less and textures of other formats are left unchanged; that is not an error, and
    /// calling this twice has no further effect.
    ///
    /// # Errors
    ///
    /// Currently never fails; the result leaves room for limits on the compressed size.
    pub fn lzo_compress(&mut self) -> Result<(), Error> {
        let (mut width, mut height) = (usize::from(self.width), usize::from(self.height));
        let compression = self.compression.clone();
        for mipmap in &mut self.mipmaps {
            let raw = raw_mipmap_size(&compression, width, height);
            if let Some(raw) =
                raw.filter(|&raw| width > 128 && !mipmap.lzo && mipmap.data.len() == raw)
            {
                // The stream already ends with the LZO end marker (11 00 00); a byte after it makes
                // the game reject the mipmap.
                let compressed = lzo1x::compress(&mipmap.data, lzo1x::CompressLevel::default());
                if compressed.len() < raw {
                    mipmap.data = compressed;
                    mipmap.lzo = true;
                }
            }
            width = (width / 2).max(1);
            height = (height / 2).max(1);
        }
        Ok(())
    }

    /// Writes the PAA texture.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure or an unsupported compression type.
    #[expect(
        clippy::too_many_lines,
        reason = "one block per PAA section keeps the writer readable"
    )]
    pub fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        let magic_number: u16 = match self.compression.as_str() {
            "DXT1" => 0xff01,
            "DXT2" => 0xff02,
            "DXT3" => 0xff03,
            "DXT4" => 0xff04,
            "DXT5" => 0xff05,
            "RGBA" => 0x8888,
            _ => {
                return Err(error!(
                    "Cannot write PAA compression type '{}'.",
                    self.compression
                ));
            }
        };
        output.write_u16::<LittleEndian>(magic_number)?;

        for (name, data) in &self.taggs {
            let mut name_bytes = [0u8; 8];
            let name = name.as_bytes();
            if name.len() > 8 {
                return Err(error!(
                    "PAA tagg name '{}' is too long (max 8 bytes).",
                    String::from_utf8_lossy(name)
                ));
            }
            let (name_part, _) = name_bytes.split_at_mut(name.len());
            name_part.copy_from_slice(name);
            output.write_all(&name_bytes)?;
            let size = u32::try_from(data.len())
                .map_err(|_| error!("tagg data too large: {} bytes", data.len()))?;
            output.write_u32::<LittleEndian>(size)?;
            output.write_all(data)?;
        }

        // Mipmap offset tagg (GGATSFFO): 16 u32 offsets, zero padded.
        let num_mipmaps = self.mipmaps.len().min(16);
        let mut offset = 2u32; // magic
        for (_, data) in &self.taggs {
            let size = u32::try_from(data.len())
                .map_err(|_| error!("tagg data too large: {} bytes", data.len()))?;
            offset += 8 + 4 + size;
        }
        offset += 8 + 4 + 64; // offset tagg itself (padded to 16 u32)
        offset += 2; // palette size
        if let Some(palette) = &self.palette {
            let size = u32::try_from(palette.len())
                .map_err(|_| error!("palette too large: {} bytes", palette.len()))?;
            offset += size;
        }

        let mut offsets: Vec<u32> = Vec::new();
        let mut current_offset = offset;
        let mut current_width = self.width;
        let mut current_height = self.height;
        for mipmap in self.mipmaps.iter().take(num_mipmaps) {
            offsets.push(current_offset);
            let size = u32::try_from(mipmap.data.len())
                .map_err(|_| error!("mipmap too large: {} bytes", mipmap.data.len()))?;
            current_offset += size + 2 + 2 + 3;
            if current_width > 1 {
                current_width /= 2;
            }
            if current_height > 1 {
                current_height /= 2;
            }
        }
        while offsets.len() < 16 {
            offsets.push(0);
        }

        output.write_all(OFFS_TAGG.as_bytes())?;
        output.write_u32::<LittleEndian>((16 * 4) as u32)?;
        for offset in offsets {
            output.write_u32::<LittleEndian>(offset)?;
        }

        // Palette size. The zero doubles as the tagg section terminator.
        let palette_len = self.palette.as_ref().map_or(0, std::vec::Vec::len);
        let palette_len = u16::try_from(palette_len)
            .map_err(|_| error!("palette too large: {palette_len} bytes"))?;
        output.write_u16::<LittleEndian>(palette_len)?;
        if let Some(palette) = &self.palette {
            output.write_all(palette)?;
        }

        // Mipmaps
        let mut current_width = self.width;
        let mut current_height = self.height;
        for mipmap in &self.mipmaps {
            let mut w = current_width;
            if mipmap.lzo {
                w |= 0x8000; // LZO compressed
            }
            output.write_u16::<LittleEndian>(w)?;
            output.write_u16::<LittleEndian>(current_height)?;
            let len = u32::try_from(mipmap.data.len())
                .map_err(|_| error!("mipmap too large: {} bytes", mipmap.data.len()))?;
            // 3-byte little endian length
            output.write_all(&len.to_le_bytes()[..3])?;
            output.write_all(&mipmap.data)?;

            if current_width > 1 {
                current_width /= 2;
            }
            if current_height > 1 {
                current_height /= 2;
            }
        }

        // Terminator
        output.write_u16::<LittleEndian>(0)?;
        output.write_u16::<LittleEndian>(0)?;
        output.write_u16::<LittleEndian>(0)?;

        Ok(())
    }
}
