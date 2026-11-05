//! `w3al facts`: list the facts of a save, or diff the facts of two saves.
use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use w3save::facts::{self, Fact, FactRecord, FactTable};
use w3save::{payload, Container};

pub(crate) fn load(path: &Path) -> Result<FactTable> {
    let file = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let c = Container::read(&file)?;
    let map = payload::read_map(&c.payload, c.data_start)?;
    facts::parse_payload(&c.payload, &map).with_context(|| format!("facts of {}", path.display()))
}

fn fmt_records(f: &Fact) -> String {
    f.records.iter().map(fmt_record).collect()
}

fn fmt_record(r: &FactRecord) -> String {
    format!("[v={} t={:.1} x={:.1}]", r.value, r.time, r.expiry)
}

fn fmt_fact(f: &Fact) -> String {
    format!("c1={} {}", f.c1, fmt_records(f))
}

fn wanted(name: &str, grep: Option<&str>) -> bool {
    grep.is_none_or(|g| name.to_lowercase().contains(&g.to_lowercase()))
}

pub fn facts_cmd(sav: &Path, diff: Option<&Path>, grep: Option<&str>) -> Result<ExitCode> {
    let a = load(sav)?;
    let Some(other) = diff else {
        println!("{} facts in {}", a.len(), sav.display());
        println!("name  c1  records [v=value t=time x=expiry]  (c1: see docs/research.md)");
        let mut shown = 0usize;
        for f in a.facts().iter().filter(|f| wanted(&f.name, grep)) {
            println!("{} {}", f.name, fmt_fact(f));
            shown += 1;
        }
        if grep.is_some() {
            println!("{shown} match(es)");
        }
        let issues = a.check_ids();
        for i in &issues {
            println!("[warn ] {}", i.message);
        }
        return Ok(ExitCode::SUCCESS);
    };

    let b = load(other)?;
    println!("A: {} facts ({})", a.len(), sav.display());
    println!("B: {} facts ({})", b.len(), other.display());
    let (mut added, mut removed, mut changed) = (Vec::new(), Vec::new(), Vec::new());
    let in_a: HashMap<&str, &Fact> = a.facts().iter().map(|f| (f.name.as_str(), f)).collect();
    for f in b.facts().iter().filter(|f| wanted(&f.name, grep)) {
        match in_a.get(f.name.as_str()) {
            None => added.push(f),
            Some(old) if *old != f => changed.push((*old, f)),
            _ => {}
        }
    }
    for f in a.facts().iter().filter(|f| wanted(&f.name, grep)) {
        if b.get(&f.name).is_none() {
            removed.push(f);
        }
    }
    // File order is the game's hash order and differs between saves; sort for a stable listing.
    added.sort_by(|x, y| x.name.cmp(&y.name));
    removed.sort_by(|x, y| x.name.cmp(&y.name));
    changed.sort_by(|x, y| x.1.name.cmp(&y.1.name));
    println!("facts: added={} removed={} changed={}", added.len(), removed.len(), changed.len());
    for f in added {
        println!("+ {} {}", f.name, fmt_fact(f));
    }
    for f in removed {
        println!("- {} {}", f.name, fmt_fact(f));
    }
    for (old, new) in changed {
        println!("~ {} {} -> {}", new.name, fmt_fact(old), fmt_fact(new));
    }
    Ok(ExitCode::SUCCESS)
}
