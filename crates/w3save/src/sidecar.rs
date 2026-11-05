//! A save is a set, not one file: `.sav` + `.png` (+ `.json` on 5.x, `.req` on 4.04).
//! The exact 5.01 set is a Phase 0 finding; until then we copy whatever sits next to
//! the template so the new save appears in the Load list with the same thumbnail,
//! which is what the closed Nexus editor does.
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;

const SIDECAR_EXTS: [&str; 3] = ["png", "json", "req"];

/// Sidecar files that exist next to `sav` (same stem, known extensions).
pub fn find(sav: &Path) -> Vec<PathBuf> {
    SIDECAR_EXTS
        .iter()
        .map(|e| sav.with_extension(e))
        .filter(|p| p.is_file())
        .collect()
}

/// Copy `template`'s sidecars to sit beside `new_sav`, renamed to its stem.
/// Never overwrites: an existing file makes the whole call fail.
///
/// UNVERIFIED: whether the game validates the `.json` against the `.sav`, and which
/// of the three hex ids in `ManualSave_<a>_<b>_<c>` it keys on (research unknown 2).
pub fn copy_for(template: &Path, new_sav: &Path) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for src in find(template) {
        let dst = new_sav.with_extension(src.extension().unwrap_or_default());
        let mut out = fs::OpenOptions::new().write(true).create_new(true).open(&dst)?;
        std::io::copy(&mut fs::File::open(&src)?, &mut out)?;
        written.push(dst);
    }
    Ok(written)
}
