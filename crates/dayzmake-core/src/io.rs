use std::io;
use std::io::{Read, Write};

/// Extensions on `Read` for the binary formats dayzmake reads.
pub(crate) trait ReadExt: Read {
    /// Reads a NUL-terminated string, decoded as UTF-8.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure or invalid UTF-8.
    fn read_cstring(&mut self) -> io::Result<String>;
    /// Reads a NUL-terminated string as raw bytes.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure.
    fn read_cstring_bytes(&mut self) -> io::Result<Vec<u8>>;
    /// Reads a BI compressed integer (7 bits per byte, high bit = continuation).
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure or an encoding longer than 5 bytes.
    fn read_compressed_int(&mut self) -> io::Result<u32>;
}

impl<T: Read> ReadExt for T {
    fn read_cstring(&mut self) -> io::Result<String> {
        let bytes = self.read_cstring_bytes()?;
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn read_cstring_bytes(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes: Vec<u8> = Vec::new();
        let mut buf = [0u8; 1];
        loop {
            self.read_exact(&mut buf)?;
            if buf[0] == 0 {
                break;
            }
            bytes.push(buf[0]);
        }

        Ok(bytes)
    }

    fn read_compressed_int(&mut self) -> io::Result<u32> {
        let mut result: u32 = 0;

        let mut buf = [0u8; 1];
        for i in 0..5 {
            self.read_exact(&mut buf)?;
            let b: u32 = buf[0].into();
            result |= (b & 0x7f) << (i * 7);

            if b < 0x80 {
                return Ok(result);
            }
        }

        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid compressed integer",
        ))
    }
}

/// Extensions on `Write` for the binary formats dayzmake writes.
pub(crate) trait WriteExt: Write {
    /// Writes a NUL-terminated string.
    ///
    /// # Errors
    ///
    /// Returns an `InvalidInput` error if the string contains a NUL byte (it would end the
    /// string early and shift every following field), and an error on I/O failure.
    fn write_cstring<S: AsRef<[u8]>>(&mut self, s: S) -> io::Result<()>;
    /// Writes a BI compressed integer, returning the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure.
    fn write_compressed_int(&mut self, x: u32) -> io::Result<usize>;
}

impl<T: Write> WriteExt for T {
    fn write_cstring<S: AsRef<[u8]>>(&mut self, s: S) -> io::Result<()> {
        let bytes = s.as_ref();
        if bytes.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "string contains a NUL byte",
            ));
        }
        self.write_all(bytes)?;
        self.write_all(b"\0")?;
        Ok(())
    }

    fn write_compressed_int(&mut self, x: u32) -> io::Result<usize> {
        let mut temp = x;
        let mut len = 0;

        while temp > 0x7f {
            // `0x80 | (temp & 0x7f)` is at most 0xff, so the conversion cannot truncate
            let byte = u8::try_from(0x80 | (temp & 0x7f)).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid compressed integer")
            })?;
            self.write_all(&[byte])?;
            len += 1;
            temp &= !0x7f;
            temp >>= 7;
        }

        let last = u8::try_from(temp).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid compressed integer")
        })?;
        self.write_all(&[last])?;
        Ok(len + 1)
    }
}

/// Number of bytes `write_compressed_int` writes for `x`.
#[must_use]
pub(crate) fn compressed_int_len(x: u32) -> usize {
    let mut temp = x;
    let mut len = 0;

    while temp > 0x7f {
        len += 1;
        temp &= !0x7f;
        temp >>= 7;
    }

    len + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_cstring_rejects_an_embedded_nul() {
        let mut out = Vec::new();
        let err = out.write_cstring("a\0b").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        out.write_cstring("ab").unwrap();
        assert_eq!(out, b"ab\0");
    }
}
