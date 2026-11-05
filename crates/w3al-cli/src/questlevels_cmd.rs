//! `w3al quest-levels`: the game's recommended quest levels (journal GUID, `.journal` resource, level).
//! Numbers only, read from the local install; nothing is written anywhere.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use w3al_core::export;

pub fn quest_levels_cmd(game_dir: Option<&Path>) -> Result<()> {
    let dir: PathBuf = game_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("W3AL_INSTALL").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(export::DEFAULT_GAME_DIR));
    let rows = w3graph::names::quest_levels(&dir).with_context(|| format!("reading the quest-level tables of {}", dir.display()))?;
    println!("guid\tlevel\tresource\tbase_name");
    for r in &rows {
        let lv = r.level.map_or_else(|| "-".to_owned(), |l| l.to_string());
        println!("{}\t{}\t{}\t{}", r.guid, lv, r.resource, r.base_name);
    }
    let with = rows.iter().filter(|r| r.level.is_some()).count();
    eprintln!("{} quests, {} with a level, {} without", rows.len(), with, rows.len() - with);
    Ok(())
}
