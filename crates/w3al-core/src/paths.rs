use std::env;
use std::path::{Path, PathBuf};

pub const QUESTBOOK_ENV: &str = "W3AL_QUESTBOOK";
pub const FIXTURES_ENV: &str = "W3AL_FIXTURES";

/// Questbook candidates relative to the folder holding the executable: next to it (portable zip and the
/// installer, which put `questbook/` and `templates/` beside `w3al.exe` and the app), a `resources` subfolder,
/// and the dev layout (`target/<profile>/w3al` inside a checkout).
pub fn exe_dir_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        exe_dir.join("questbook"),
        exe_dir.join("resources").join("questbook"),
        exe_dir.join("../../questbook"),
        exe_dir.join("../../../questbook"),
    ]
}

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
            candidates.extend(exe_dir_candidates(dir));
        }
    }
    candidates.into_iter().find(|p| p.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questbook_is_found_next_to_the_executable() {
        let root = env::temp_dir().join(format!("w3al-paths-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("questbook")).unwrap();
        let found = exe_dir_candidates(&root).into_iter().find(|p| p.is_dir());
        assert_eq!(found, Some(root.join("questbook")));
        // the installer layout also works: resources/questbook when nothing sits next to the exe
        std::fs::remove_dir_all(root.join("questbook")).unwrap();
        std::fs::create_dir_all(root.join("resources").join("questbook")).unwrap();
        let found = exe_dir_candidates(&root).into_iter().find(|p| p.is_dir());
        assert_eq!(found, Some(root.join("resources").join("questbook")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
