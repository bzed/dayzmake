//! A bounds-checked little-endian cursor over a byte slice, shared by the navmesh readers.

use std::io;

/// A bounds-checked little-endian cursor over a byte slice.
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    /// The bytes not yet read.
    pub(crate) fn rest(&self) -> &'a [u8] {
        self.buf
    }

    pub(crate) fn remaining(&self) -> usize {
        self.buf.len()
    }

    pub(crate) fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let (head, tail) = self
            .buf
            .split_first_chunk::<N>()
            .ok_or_else(|| error!("message truncated"))?;
        self.buf = tail;
        Ok(*head)
    }

    pub(crate) fn u8(&mut self) -> io::Result<u8> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    pub(crate) fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub(crate) fn f32(&mut self) -> io::Result<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    pub(crate) fn f32x3(&mut self) -> io::Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    /// A count read from the payload as `usize`, checked against `limit` and against the bytes left
    /// (`count * element_len`) before anything is allocated for it.
    pub(crate) fn count(
        &mut self,
        what: &str,
        limit: usize,
        element_len: usize,
    ) -> io::Result<usize> {
        let n =
            usize::try_from(self.u32()?).map_err(|_| error!("implausible {what} {}", u32::MAX))?;
        if n > limit {
            return Err(error!("implausible {what} {n}"));
        }
        self.check_available(n, element_len)?;
        Ok(n)
    }

    pub(crate) fn check_available(&self, n: usize, element_len: usize) -> io::Result<()> {
        let bytes = n
            .checked_mul(element_len)
            .ok_or_else(|| error!("message truncated"))?;
        if bytes > self.remaining() {
            return Err(error!("message truncated"));
        }
        Ok(())
    }
}
