//! "Packed strings": the length-prefixed strings used for fact identifiers and names.
//!
//! Layout (from the plan; confirmed by Phase 0 round-trip of real facts):
//!   byte 0: bit 7 = single-byte (ANSI) characters, bit 6 = "more length bytes follow",
//!           bits 0..5 = low 6 bits of the length (in characters).
//!   next bytes: 7 more length bits each, bit 7 = "another length byte follows".
//!   body: `len` bytes (ANSI) or `len` UTF-16LE code units (2 bytes each).
//!
//! The 5.0.0c bug re-encoded Latin-1 identifiers as UTF-8 on every save. Writing
//! therefore picks ANSI whenever every char fits in Latin-1, and never "widens".
use crate::error::{Error, Result};
use crate::reader::Reader;

/// Upper bound on a decoded length. The 5.0.0c bug produced identifiers up to 512x
/// longer than normal; anything past this is corruption, not data.
const MAX_LEN: usize = 1 << 20;

pub fn read(r: &mut Reader) -> Result<String> {
    let b0 = r.u8()?;
    let ansi = b0 & 0x80 != 0;
    let mut len = (b0 & 0x3F) as usize;
    if b0 & 0x40 != 0 {
        let mut shift = 6u32;
        loop {
            let b = r.u8()?;
            if shift > 28 {
                return Err(Error::Payload("packed string length overflow".into()));
            }
            len |= ((b & 0x7F) as usize) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
    }
    if len > MAX_LEN {
        return Err(Error::Payload(format!("packed string length {len} is implausible")));
    }
    if ansi {
        // Latin-1: each byte is the code point of the same value.
        Ok(r.bytes(len)?.iter().map(|&b| b as char).collect())
    } else {
        let raw = r.bytes(len * 2)?;
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units).map_err(|e| Error::Payload(format!("bad UTF-16 in packed string: {e}")))
    }
}

pub fn write(out: &mut Vec<u8>, s: &str) {
    let ansi = s.chars().all(|c| (c as u32) <= 0xFF);
    let len = s.chars().count();
    let mut first = (len & 0x3F) as u8;
    if ansi {
        first |= 0x80;
    }
    let mut rest = len >> 6;
    if rest > 0 {
        first |= 0x40;
    }
    out.push(first);
    while rest > 0 {
        let mut b = (rest & 0x7F) as u8;
        rest >>= 7;
        if rest > 0 {
            b |= 0x80;
        }
        out.push(b);
    }
    if ansi {
        out.extend(s.chars().map(|c| c as u32 as u8));
    } else {
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(s: &str) {
        let mut buf = Vec::new();
        write(&mut buf, s);
        let mut r = Reader::new(&buf);
        assert_eq!(read(&mut r).unwrap(), s);
        assert_eq!(r.remaining(), 0, "no trailing bytes for {s:?}");
    }

    #[test]
    fn short_ansi() {
        roundtrip("");
        roundtrip("q103_botchling_saved");
    }

    #[test]
    fn length_continuation_boundaries() {
        // 63 fits in the first byte; 64 needs one extension byte; 8192 (= 64 + 128 * 64) needs two.
        for n in [63usize, 64, 65, 8191, 8192, 70_000] {
            roundtrip(&"a".repeat(n));
        }
    }

    #[test]
    fn latin1_stays_single_byte() {
        let mut buf = Vec::new();
        write(&mut buf, "caf\u{e9}");
        assert_eq!(buf[0] & 0x80, 0x80, "Latin-1 text must stay ANSI");
        assert_eq!(buf.len(), 1 + 4);
        roundtrip("caf\u{e9}");
    }

    #[test]
    fn non_latin1_uses_utf16() {
        let mut buf = Vec::new();
        write(&mut buf, "\u{141}\u{f3}d\u{17a}");
        assert_eq!(buf[0] & 0x80, 0);
        assert_eq!(buf.len(), 1 + 4 * 2);
        roundtrip("\u{141}\u{f3}d\u{17a}");
    }

    #[test]
    fn truncated_is_an_error_not_a_panic() {
        let mut buf = Vec::new();
        write(&mut buf, "hello");
        buf.pop();
        assert!(read(&mut Reader::new(&buf)).is_err());
    }
}
