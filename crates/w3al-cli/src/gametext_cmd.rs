//! `w3al game-text`: print a journal quest's (or character's) texts from the local install, for checking
//! only. Nothing is written anywhere.
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use w3al_core::export;

pub fn game_text_cmd(key: &str, lang: &str, game_dir: Option<&Path>) -> Result<()> {
    let dir: PathBuf = game_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("W3AL_INSTALL").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(export::DEFAULT_GAME_DIR));
    let t = w3graph::names::journal_texts(&dir, lang).with_context(|| format!("reading the journals/strings of {}", dir.display()))?;
    let k = key.to_ascii_lowercase().replace('/', "\\");
    let hit = |guid: &str, res: &str, base: &str| {
        let stem = res.rsplit('\\').next().unwrap_or("").trim_end_matches(".journal").to_ascii_lowercase();
        (k.len() >= 4 && guid.starts_with(&k)) || res.to_ascii_lowercase() == k || stem == k || base.to_ascii_lowercase().contains(&k)
    };
    let mut n = 0;
    for q in t.quests.iter().filter(|q| hit(&q.guid, &q.resource, &q.base_name)) {
        n += 1;
        println!("quest {} ({})\n  {}\n  title: {}", q.guid, q.base_name, q.resource, q.title);
        for d in &q.descriptions {
            println!("  description [{}] {}: {}", d.index, d.guid, d.text);
        }
        for o in &q.objectives {
            println!("  objective {}: {}", o.guid, o.title);
        }
    }
    for c in t.characters.iter().filter(|c| hit(&c.guid, &c.resource, &c.base_name)) {
        n += 1;
        println!("character {} ({})\n  {}\n  name: {}", c.guid, c.base_name, c.resource, c.name);
        for d in &c.descriptions {
            println!("  description [{}] {}: {}", d.index, d.guid, d.text);
        }
    }
    if n == 0 {
        bail!("no journal quest or character matches {key:?} (a GUID prefix of 4+ hex digits, a .journal path or file stem, or part of the English working name)");
    }
    Ok(())
}
