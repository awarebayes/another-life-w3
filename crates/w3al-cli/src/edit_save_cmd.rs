//! `w3al edit-plan <sav>` and `w3al edit --save <sav> ...`: edit mode on one of the player's own saves
//! (w3al_core::edit). The opened save is only read; the result is a NEW save in `--out-dir`.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use w3al_core::edit::{Changes, Engine, Plan};
use w3al_core::export::ExportOptions;
use w3al_core::templates::{self, Registry};
use w3al_core::{Questbook, Run};

use crate::chain_cmd::questbook_dir;

pub struct EngineArgs {
    pub questbook: Option<PathBuf>,
    pub template_dirs: Vec<PathBuf>,
    pub opts: ExportOptions,
    pub cache: Option<PathBuf>,
}

/// The scan cache next to the other app data (`%LOCALAPPDATA%\w3al`), else the temp folder.
pub fn default_cache() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("w3al").join("edit-scan-cache.json")
}

pub fn load_engine(a: &EngineArgs) -> Result<Engine> {
    let qb = questbook_dir(a.questbook.clone())?;
    let book = Questbook::load_dir(&qb)?;
    let reg = Registry::load_for_questbook(&qb).map_err(anyhow::Error::msg)?;
    let dirs = if a.template_dirs.is_empty() { templates::search_dirs(&Run::default(), Path::new("."), &qb) } else { a.template_dirs.clone() };
    let cache = a.cache.clone().unwrap_or_else(default_cache);
    Engine::load(book, reg, dirs, a.opts.clone(), Some(&cache), &|m| eprintln!("... {m}")).map_err(anyhow::Error::msg)
}

fn print_plan(p: &Plan) {
    println!("save: {}", p.stem);
    if let Some(t) = &p.is_template {
        println!("this save is the registered template {t}");
    }
    println!("position (the template whose fixed rules apply): {}", p.position.as_deref().unwrap_or("-"));
    println!("\nwindows:");
    for w in &p.windows {
        println!(
            "  {:<22} line {:>2}  {:<11} {}{}",
            w.id,
            w.line,
            w.state,
            w.buildable.map_or(String::new(), |b| format!("[dry build {}] ", if b { "ok" } else { "FAILS" })),
            if w.why.is_empty() { String::new() } else { format!("({})", w.why) }
        );
    }
    println!("\nquests:");
    for q in &p.quests {
        println!("  {:<34} {:<11} {}", q.quest, q.status, q.note);
        for c in &q.choices {
            let opts = c.options.iter().map(|o| if o.allowed { o.id.clone() } else { format!("({})", o.id) }).collect::<Vec<_>>().join(" | ");
            println!(
                "      {}.{} = {} [{}; {}] options: {}{}",
                q.quest,
                c.id,
                c.current.as_deref().unwrap_or("?"),
                c.source,
                c.mode,
                opts,
                c.reason.as_ref().map_or(String::new(), |r| format!("\n          why not: {r}"))
            );
        }
    }
    match (&p.character, &p.character_error) {
        (Some(c), _) => println!("\ncharacter: level {} (max {}), crowns {:?}, {} item records", c.level, c.max_level, c.crowns, c.items),
        (None, Some(e)) => println!("\ncharacter: not readable ({e})"),
        _ => {}
    }
    for n in &p.notes {
        println!("note: {n}");
    }
}

pub fn edit_plan(sav: &Path, a: &EngineArgs, json: bool) -> Result<ExitCode> {
    let engine = load_engine(a)?;
    let bytes = std::fs::read(sav).with_context(|| format!("reading {}", sav.display()))?;
    let stem = sav.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let plan = engine.plan(&bytes, stem, true).map_err(anyhow::Error::msg)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    } else {
        print_plan(&plan);
    }
    Ok(ExitCode::SUCCESS)
}

/// `quest.choice=option` -> (quest, choice, option).
pub fn parse_set(s: &str) -> Result<(String, String, String)> {
    let (qc, o) = s.split_once('=').with_context(|| format!("--set {s:?}: want quest.choice=option"))?;
    let (q, c) = qc.split_once('.').with_context(|| format!("--set {s:?}: want quest.choice=option"))?;
    if q.is_empty() || c.is_empty() || o.is_empty() {
        bail!("--set {s:?}: want quest.choice=option");
    }
    Ok((q.to_owned(), c.to_owned(), o.to_owned()))
}

pub struct EditArgs<'a> {
    pub save: &'a Path,
    pub sets: &'a [String],
    pub level: Option<u32>,
    pub crowns: Option<u32>,
    pub gear: Option<&'a str>,
    pub items: &'a [String],
    pub no_ngplus: bool,
    pub skill_points: Option<u32>,
    pub out_dir: &'a Path,
    pub stem: Option<&'a str>,
}

pub fn edit(e: &EditArgs, a: &EngineArgs) -> Result<ExitCode> {
    let mut ch = Changes { level: e.level, crowns: e.crowns, items: e.items.to_vec(), no_new_game_plus: e.no_ngplus, skill_points: e.skill_points, ..Changes::default() };
    match e.gear {
        None => {}
        Some("recommended") => ch.gear = true,
        Some(other) => bail!("--gear {other:?}: only `recommended` exists"),
    }
    let mut answers: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for s in e.sets {
        let (q, c, o) = parse_set(s)?;
        answers.entry(q).or_default().insert(c, o);
    }
    ch.answers = answers;
    let engine = load_engine(a)?;
    let out = engine.apply(e.save, e.out_dir, &ch, e.stem).map_err(|x| anyhow::anyhow!("{x}"))?;
    for n in &out.notes {
        println!("  {n}");
    }
    println!("wrote {}", out.sav.display());
    for s in &out.sidecars {
        println!("      {}", s.display());
    }
    if !out.rewritten.is_empty() {
        println!("rewritten decisions: {}", out.rewritten.join(", "));
    }
    if !out.applied.is_empty() {
        println!("applied quests of the next window: {}", out.applied.join(", "));
    }
    println!("{}", out.verified);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_syntax() {
        assert_eq!(parse_set("the_last_wish.yennefer=love").unwrap(), ("the_last_wish".into(), "yennefer".into(), "love".into()));
        assert!(parse_set("nope").is_err());
        assert!(parse_set("a.b=").is_err());
    }
}
