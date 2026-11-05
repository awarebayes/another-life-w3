use crate::error::{Error, Result};

/// Bounds-checked little-endian cursor. Every short read is a `Truncated` error,
/// never a panic, so malformed saves can't crash the parser (see the fuzzing item
/// in the plan).
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn at(buf: &'a [u8], pos: usize) -> Result<Self> {
        if pos > buf.len() {
            return Err(Error::Truncated(format!(
                "seek to {pos} but only {} bytes available",
                buf.len()
            )));
        }
        Ok(Self { buf, pos })
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or_else(|| {
                Error::Truncated(format!(
                    "need {n} bytes at {} but only {} remain",
                    self.pos,
                    self.remaining()
                ))
            })?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    /// Consume `tag` or fail with a message naming both sides.
    pub fn expect_tag(&mut self, tag: &[u8]) -> Result<()> {
        let at = self.pos;
        let got = self.bytes(tag.len())?;
        if got != tag {
            return Err(Error::Payload(format!(
                "expected tag {:?} at {at}, found {:?}",
                String::from_utf8_lossy(tag),
                String::from_utf8_lossy(got)
            )));
        }
        Ok(())
    }
}
