use std::env;
use std::path::PathBuf;

pub const QUESTBOOK_ENV: &str = "W3AL_QUESTBOOK";
pub const FIXTURES_ENV: &str = "W3AL_FIXTURES";

/// Find the questbook directory: explicit override, env var, then the usual places
/// for a dev checkout and an installed app.
pub fn locate_questbook(extra: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = env::var(QUESTBOOK_ENV) {
        candidates.push(p.into());
    }
    candidates.extend(extra);
    if let Ok(cwd) = env::current_dir() {
        candidates.push(cwd.join("questbook"));
        candidates.push(cwd.join("..").join("questbook"));
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("questbook"));
            candidates.push(dir.join("resources").join("questbook"));
            // target/debug/<exe> inside a dev checkout
            candidates.push(dir.join("../../questbook"));
            candidates.push(dir.join("../../../questbook"));
        }
    }
    candidates.into_iter().find(|p| p.is_dir())
}
