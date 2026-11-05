//! Layer 1: the `SNFHFZLC` outer container.
//!
//! ```text
//! 0   "SNFHFZLC"
//! 8   u32 chunk count
//! 12  u32 data start (3,084 on every observed 5.x save)
//! 16  count x { u32 compressed size, u32 decompressed size, u32 end offset }
//!     zero padding up to `data start`
//!     raw LZ4 blocks (no frame header), each decoding to 1 MiB except the last
//! ```
//! Compressing with `lz4_flex` will not reproduce the game's compressed bytes, so
//! "round trip" means: decompress -> serialize -> compare *decompressed payloads*.
use serde::Serialize;

use crate::diag::Issue;
use crate::error::{Error, Result};
use crate::reader::Reader;

pub const MAGIC: &[u8; 8] = b"SNFHFZLC";
pub const CHUNK_SIZE: usize = 1 << 20;
pub const TABLE_START: usize = 16;
pub const RECORD_SIZE: usize = 12;
/// Observed on every 5.x save: room for 255 records (16 + 255*12 = 3,076).
pub const DEFAULT_DATA_START: u32 = 3084;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChunkRecord {
    pub compressed_size: u32,
    pub decompressed_size: u32,
    pub end_offset: u32,
}

#[derive(Debug, Clone)]
pub struct Container {
    pub data_start: u32,
    /// Records exactly as read. Informational only; `write` rebuilds them.
    pub records: Vec<ChunkRecord>,
    /// Bytes between the end of the chunk table and `data_start`, kept verbatim in
    /// case the game stores anything there. Observed to be zeros so far.
    pub header_padding: Vec<u8>,
    /// Plan: "the last record uses a zero end sentinel". Remembered per file.
    pub zero_last_end: bool,
    /// Concatenated decompressed chunks.
    pub payload: Vec<u8>,
    /// Compressed bytes left after the last chunk (should be 0).
    pub trailing_bytes: usize,
}

impl Container {
    /// Wrap a payload for writing, with the observed 5.x defaults.
    pub fn from_payload(payload: Vec<u8>) -> Self {
        Self {
            data_start: DEFAULT_DATA_START,
            records: Vec::new(),
            header_padding: Vec::new(),
            zero_last_end: true,
            payload,
            trailing_bytes: 0,
        }
    }

    pub fn read(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < TABLE_START || &bytes[..8] != MAGIC {
            return Err(Error::BadMagic);
        }
        let mut r = Reader::at(bytes, 8)?;
        let count = r.u32()? as usize;
        let data_start = r.u32()? as usize;

        let table_end = count
            .checked_mul(RECORD_SIZE)
            .and_then(|n| n.checked_add(TABLE_START))
            .filter(|&e| e <= bytes.len())
            .ok_or_else(|| Error::Truncated(format!("chunk table of {count} records exceeds the file")))?;
        if data_start > bytes.len() {
            return Err(Error::Truncated(format!(
                "data start {data_start} is past the end of the file ({} bytes)",
                bytes.len()
            )));
        }

        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            records.push(ChunkRecord {
                compressed_size: r.u32()?,
                decompressed_size: r.u32()?,
                end_offset: r.u32()?,
            });
        }

        // If the table overran `data_start` (the 5.0.0c bug) there is no padding; the
        // overflow is reported by `diagnostics`, not by failing the read, so
        // `check-save` can still describe the damaged file.
        let header_padding = if data_start >= table_end {
            bytes[table_end..data_start].to_vec()
        } else {
            Vec::new()
        };

        let mut cursor = data_start;
        let mut payload = Vec::new();
        for (i, rec) in records.iter().enumerate() {
            let end = cursor
                .checked_add(rec.compressed_size as usize)
                .filter(|&e| e <= bytes.len())
                .ok_or_else(|| Error::Truncated(format!("chunk {i} runs past the end of the file")))?;
            let block = lz4_flex::block::decompress(&bytes[cursor..end], rec.decompressed_size as usize)
                .map_err(|e| Error::Lz4 {
                    chunk: i,
                    msg: e.to_string(),
                })?;
            if block.len() != rec.decompressed_size as usize {
                return Err(Error::Lz4 {
                    chunk: i,
                    msg: format!("decoded {} bytes, table says {}", block.len(), rec.decompressed_size),
                });
            }
            payload.extend_from_slice(&block);
            cursor = end;
        }

        Ok(Self {
            data_start: data_start as u32,
            zero_last_end: records.last().is_some_and(|r| r.end_offset == 0),
            records,
            header_padding,
            payload,
            trailing_bytes: bytes.len() - cursor,
        })
    }

    /// Serialize. Never emits a table that overruns `data_start`.
    pub fn write(&self) -> Result<Vec<u8>> {
        let blocks: Vec<(usize, Vec<u8>)> = self
            .payload
            .chunks(CHUNK_SIZE)
            .map(|c| (c.len(), lz4_flex::block::compress(c)))
            .collect();

        let count = blocks.len();
        let data_start = self.data_start as usize;
        let table_end = TABLE_START + count * RECORD_SIZE;
        if table_end > data_start {
            return Err(Error::TableOverflow {
                count,
                needed: table_end,
                data_start,
            });
        }

        let mut out = Vec::with_capacity(data_start + blocks.iter().map(|b| b.1.len()).sum::<usize>());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&self.data_start.to_le_bytes());

        // HYPOTHESIS: `end offset` is absolute (data_start + cumulative compressed
        // size). `diagnostics` compares this against the records read from a real file.
        let mut end = data_start;
        for (i, (dec_len, comp)) in blocks.iter().enumerate() {
            end += comp.len();
            let end_field = if self.zero_last_end && i + 1 == count { 0 } else { end as u32 };
            out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            out.extend_from_slice(&(*dec_len as u32).to_le_bytes());
            out.extend_from_slice(&end_field.to_le_bytes());
        }

        let pad_len = data_start - table_end;
        if self.header_padding.len() == pad_len {
            out.extend_from_slice(&self.header_padding);
        } else {
            out.resize(out.len() + pad_len, 0);
        }
        for (_, comp) in &blocks {
            out.extend_from_slice(comp);
        }
        Ok(out)
    }

    /// Structural checks shared by `w3al check-save`. Includes the 5.0.0c signatures.
    pub fn diagnostics(&self) -> Vec<Issue> {
        let mut out = Vec::new();
        let needed = TABLE_START + self.records.len() * RECORD_SIZE;
        if needed > self.data_start as usize {
            out.push(Issue::error(format!(
                "chunk table ({} records, {needed} bytes) overruns the data start {}: 5.0.0c damage signature",
                self.records.len(),
                self.data_start
            )));
        }
        if self.records.len() > 32 {
            out.push(Issue::warn(format!(
                "{} chunks; a healthy 5.x save has 5-6 (5.0.0c bug produced 266)",
                self.records.len()
            )));
        }
        if self.payload.len() > 32 << 20 {
            out.push(Issue::warn(format!(
                "decoded payload is {} MiB; a healthy 5.x save is ~5 MiB",
                self.payload.len() >> 20
            )));
        }
        if self.data_start != DEFAULT_DATA_START {
            out.push(Issue::warn(format!(
                "data start is {}, every observed 5.x save has {DEFAULT_DATA_START}",
                self.data_start
            )));
        }
        if self.trailing_bytes != 0 {
            out.push(Issue::warn(format!("{} unexpected bytes after the last chunk", self.trailing_bytes)));
        }
        if self.header_padding.iter().any(|&b| b != 0) {
            out.push(Issue::info("header padding is not all zeros; kept verbatim on write"));
        }
        // Check the "end offset is absolute" hypothesis against what the game wrote.
        let mut end = self.data_start as u64;
        for (i, rec) in self.records.iter().enumerate() {
            end += rec.compressed_size as u64;
            let last = i + 1 == self.records.len();
            if rec.end_offset as u64 != end && !(last && rec.end_offset == 0) {
                out.push(Issue::warn(format!(
                    "chunk {i}: end offset {} != data_start + cumulative size {end}; the 'absolute end offset' hypothesis is wrong",
                    rec.end_offset
                )));
                break;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload(len: usize) -> Vec<u8> {
        // Compressible but not constant, so LZ4 does real work.
        (0..len).map(|i| ((i * 7) % 251) as u8 ^ (i >> 9) as u8).collect()
    }

    #[test]
    fn roundtrip_multi_chunk() {
        let payload = sample_payload(CHUNK_SIZE * 2 + 12_345);
        let file = Container::from_payload(payload.clone()).write().unwrap();
        assert_eq!(&file[..8], MAGIC);

        let back = Container::read(&file).unwrap();
        assert_eq!(back.payload, payload);
        assert_eq!(back.records.len(), 3);
        assert_eq!(back.data_start, DEFAULT_DATA_START);
        assert!(back.zero_last_end);
        assert!(back.diagnostics().is_empty(), "{:?}", back.diagnostics());

        // A second pass must be stable (compressed bytes are ours both times).
        assert_eq!(back.write().unwrap(), file);
    }

    #[test]
    fn empty_payload() {
        let file = Container::from_payload(Vec::new()).write().unwrap();
        assert_eq!(file.len(), DEFAULT_DATA_START as usize);
        assert!(Container::read(&file).unwrap().payload.is_empty());
    }

    #[test]
    fn rejects_bad_magic_and_truncation() {
        assert!(matches!(Container::read(b"nope"), Err(Error::BadMagic)));
        let mut file = Container::from_payload(sample_payload(5000)).write().unwrap();
        file.truncate(file.len() - 10);
        assert!(Container::read(&file).is_err());
    }

    #[test]
    fn writer_refuses_table_overflow() {
        // 256 chunks need 16 + 256*12 = 3,088 > 3,084: exactly the 5.0.0c failure.
        let payload = vec![0u8; CHUNK_SIZE * 256];
        let err = Container::from_payload(payload).write().unwrap_err();
        assert!(matches!(err, Error::TableOverflow { count: 256, .. }));
    }

    #[test]
    fn check_flags_overflow_on_read() {
        // A damaged file can't be built byte-wise (the table overlaps the chunk data
        // by definition), so exercise `diagnostics` on the parsed shape directly.
        let mut c = Container::from_payload(Vec::new());
        c.records = vec![
            ChunkRecord {
                compressed_size: 10,
                decompressed_size: 10,
                end_offset: 0,
            };
            260
        ];
        assert!(c
            .diagnostics()
            .iter()
            .any(|i| i.message.contains("5.0.0c damage signature")));
    }
}
