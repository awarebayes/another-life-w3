use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not a Witcher 3 save container (expected `SNFHFZLC` magic)")]
    BadMagic,
    #[error("file is truncated: {0}")]
    Truncated(String),
    #[error("LZ4 failure in chunk {chunk}: {msg}")]
    Lz4 { chunk: usize, msg: String },
    /// The 5.0.0c recovery report hit this: 266 chunks overran the 3,084-byte data start.
    /// The writer refuses to emit such a file.
    #[error("chunk table does not fit before the data start: 16 + {count}*12 = {needed} > {data_start}")]
    TableOverflow {
        count: usize,
        needed: usize,
        data_start: usize,
    },
    #[error("payload: {0}")]
    Payload(String),
    /// A code path that needs evidence from a real 5.01 save. Never guess past this.
    #[error("not yet verified against a real 5.01 save: {0}")]
    Unverified(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
