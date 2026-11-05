//! `w3al character`, `w3al character-set`: read and edit the player's level, crowns and items
//! (docs/findings/character-inventory.md).
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use w3al_core::export;
use w3al_core::start_kit::{self, StartKit};
use w3save::character;

use crate::graph_cmd::load_doc;

pub fn character_cmd(sav: &Path, items: bool) -> Result<ExitCode> {
    let d = load_doc(sav)?;
    let c = character::read(&d)?;
    println!("level {}  (lastCustomLevel {}, table {} rows)", c.level.level, c.level.last_custom_level, c.level.table.len());
    println!("xp total {}  (free {} + used {}; the level needs {:?})", c.level.total_xp(), c.level.xp.free, c.level.xp.used, c.level.exp_for_level(c.level.level));
    println!(
        "skill points free {} used {} (table up to this level: {:?})",
        c.level.skill.free,
        c.level.skill.used,
        c.level.skill_points_up_to(c.level.level)
    );
    println!("Lvl abilities {:?}", c.level_abilities);
    for s in &c.stats {
        println!("stat {} current {} max {}", s.name, s.current, s.max);
    }
    println!("crowns {:?}", c.crowns());
    println!("items {} (header: count {} next uid {} unknown {})", c.items.len(), c.header.count, c.header.next_uid, c.header.unknown);
    if items {
        for i in &c.items {
            println!("  {:<40} x{:<5} dur {}", i.name, i.quantity, i.durability);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Write a NEW save from a template with the start kit applied, then re-read it.
pub fn character_set_cmd(template: &Path, kit: &StartKit, stem: Option<&str>, out_dir: &Path) -> Result<ExitCode> {
    let bytes_in = std::fs::read(template).with_context(|| format!("reading {}", template.display()))?;
    let mut report = Vec::new();
    let bytes = export::build_save_from_document(&bytes_in, |doc| {
        report = start_kit::apply(doc, kit)?;
        Ok(())
    })?;
    let stem = match stem {
        Some(s) => s.to_owned(),
        None => export::default_stem(template, out_dir),
    };
    let written = export::write_save_set_as(template, out_dir, &stem, &bytes)?;
    for l in &report {
        println!("{l}");
    }
    println!("wrote {}", written.sav.display());
    let problems = start_kit::verify_bytes(&bytes_in, &std::fs::read(&written.sav)?, kit)?;
    if problems.is_empty() {
        println!("VERIFIED: re-read the written file; the start kit is in it, nothing else of the character changed");
        Ok(ExitCode::SUCCESS)
    } else {
        for p in &problems {
            eprintln!("PROBLEM: {p}");
        }
        Ok(ExitCode::FAILURE)
    }
}
