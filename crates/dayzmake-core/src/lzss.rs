//! Bohemia's LZSS variant, used by "Cprs" PBO entries and by the 16-bit PAA formats.

use std::io::Error;

/// Decompresses a stream: groups of 8 items led by a flag byte (LSB first), bit set = literal byte,
/// bit clear = 2-byte back reference (12 bit offset, 4 bit length - 3). The window starts out filled
/// with spaces.
///
/// Returns the `original_size` output bytes and the number of input bytes consumed; what follows
/// them (a 4 byte checksum, in both users) is left to the caller. `what` names the data in error
/// messages ("PBO entry", "PAA mipmap").
pub(crate) fn decompress(
    data: &[u8],
    original_size: usize,
    what: &str,
) -> Result<(Vec<u8>, usize), Error> {
    let truncated = || error!("Truncated compressed {what}.");
    // a flag byte and 8 two byte references expand to at most 8 * 18 bytes out of 17 in
    let max_size = data.len().saturating_mul(9);
    if original_size > max_size {
        return Err(error!(
            "Compressed {what} claims an implausible size: {original_size} bytes from {} bytes.",
            data.len()
        ));
    }
    let mut out: Vec<u8> = Vec::with_capacity(original_size.min(max_size));
    let mut pos = 0;

    while out.len() < original_size {
        let flags = *data.get(pos).ok_or_else(truncated)?;
        pos += 1;

        for bit in 0..8 {
            if out.len() >= original_size {
                break;
            }

            if flags & (1 << bit) != 0 {
                out.push(*data.get(pos).ok_or_else(truncated)?);
                pos += 1;
            } else {
                let b1 = *data.get(pos).ok_or_else(truncated)? as usize;
                let b2 = *data.get(pos + 1).ok_or_else(truncated)? as usize;
                pos += 2;

                let offset = b1 | ((b2 & 0xF0) << 4);
                let length = (b2 & 0x0F) + 3;
                if offset == 0 {
                    return Err(error!("Invalid back reference in compressed {what}."));
                }

                for _ in 0..length {
                    let byte = out
                        .len()
                        .checked_sub(offset)
                        .and_then(|i| out.get(i))
                        .copied()
                        .unwrap_or(b' ');
                    out.push(byte);
                }
            }
        }
    }

    out.truncate(original_size);
    Ok((out, pos))
}

/// The checksum that follows an LZSS stream: the sum of the output bytes, each as a signed byte.
pub(crate) fn checksum(data: &[u8]) -> i32 {
    data.iter().fold(0i32, |sum, &b| {
        sum.wrapping_add(i32::from(i8::from_le_bytes([b])))
    })
}
