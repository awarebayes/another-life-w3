//! `POTATO70` bundles: the archives under `content\content0\bundles` and `dlc\*\content\bundles`.
//!
//! Layout (read from the 5.01 install, checked on every bundle of it): 32-byte header
//! (`POTATO70`, u32 at 16 = size of the table of contents), then the table: 304-byte entries
//! (`name[256]` NUL-padded, 16-byte hash, u32 at 272 = offset of the data, pad, u32 at 280 = size,
//! u32 at 284 = stored size, u32 at 288 = crc, u32 at 292 = compression). Compression 0 = stored,
//! 1 = zlib; the LZ4 codes (4, 5) never occur in the quest-graph entries but are decoded anyway.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::quest::Error;

#[derive(Debug, Clone)]
pub struct Entry {
    /// Path inside the bundle, backslash-separated (`quests\part_1\part_1.w2phase`).
    pub name: String,
    pub offset: u64,
    pub size: u32,
    pub stored: u32,
    pub compression: u32,
}

/// The table of contents of one bundle.
pub fn toc(path: &Path) -> Result<Vec<Entry>, Error> {
    let mut f = File::open(path).map_err(|e| Error::Io(path.display().to_string(), e.to_string()))?;
    let mut h = [0u8; 32];
    f.read_exact(&mut h).map_err(|e| Error::Io(path.display().to_string(), e.to_string()))?;
    if &h[0..8] != b"POTATO70" {
        return Err(Error::Format(format!("{}: not a POTATO70 bundle", path.display())));
    }
    let size = u32::from_le_bytes([h[16], h[17], h[18], h[19]]) as usize;
    let mut t = vec![0u8; size];
    f.read_exact(&mut t).map_err(|e| Error::Io(path.display().to_string(), e.to_string()))?;
    Ok(t.chunks_exact(304)
        .map(|c| {
            let u = |o: usize| u32::from_le_bytes([c[o], c[o + 1], c[o + 2], c[o + 3]]);
            Entry {
                name: c[0..256].iter().take_while(|&&b| b != 0).map(|&b| b as char).collect(),
                offset: u64::from(u(272)),
                size: u(280),
                stored: u(284),
                compression: u(292),
            }
        })
        .collect())
}

/// The decoded bytes of one entry.
pub fn extract(path: &Path, e: &Entry) -> Result<Vec<u8>, Error> {
    let io = |x: std::io::Error| Error::Io(path.display().to_string(), x.to_string());
    let mut f = File::open(path).map_err(io)?;
    f.seek(SeekFrom::Start(e.offset)).map_err(io)?;
    let mut z = vec![0u8; e.stored as usize];
    f.read_exact(&mut z).map_err(io)?;
    match e.compression {
        0 => Ok(z),
        1 => {
            let mut out = Vec::with_capacity(e.size as usize);
            flate2::read::ZlibDecoder::new(&z[..]).read_to_end(&mut out).map_err(io)?;
            Ok(out)
        }
        4 | 5 => lz4_flex::block::decompress(&z, e.size as usize).map_err(|x| Error::Format(format!("{}: lz4: {x}", e.name))),
        c => Err(Error::Format(format!("{}: unknown compression {c}", e.name))),
    }
}

/// Every `.bundle` under an install: `content\content0\bundles` (base game) and
/// `dlc\<name>\content\bundles` (DLCs), in a stable order.
pub fn bundles_of_install(install: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![install.join("content").join("content0").join("bundles")];
    if let Ok(rd) = std::fs::read_dir(install.join("dlc")) {
        let mut dlcs: Vec<PathBuf> = rd.flatten().map(|d| d.path().join("content").join("bundles")).collect();
        dlcs.sort();
        dirs.extend(dlcs);
    }
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(&d) {
            let mut files: Vec<PathBuf> = rd.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|x| x == "bundle")).collect();
            files.sort();
            out.extend(files);
        }
    }
    out
}

/// Is this a quest-graph resource (`.w2phase` or `.w2quest`)?
pub fn is_quest_graph(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".w2phase") || n.ends_with(".w2quest")
}
