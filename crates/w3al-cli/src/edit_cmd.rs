//! `w3al edit-fact`: Phase 0 development tool. Builds a NEW save from a template with a few
//! facts changed, writes it with its sidecars (never overwriting), then re-reads the written
//! file and checks it. Used to make the in-game gate saves (PHASE0.md step 9).
use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use w3al_core::export;
use w3save::facts::FactTable;
use w3save::{Container, Document, Edits, Severity};

use crate::facts_cmd;

pub struct EditArgs<'a> {
    pub template: &'a Path,
    /// `NAME=VALUE`, applied with `Document::apply` (FactsSet semantics).
    pub sets: &'a [String],
    /// Facts to remove from the table (`FactsRemove`); each must exist and must not also be in `sets`.
    pub removes: &'a [String],
    /// Make every fact that differs from this save's exactly equal to it (copy or remove).
    pub match_facts_of: Option<&'a Path>,
    pub stem: Option<&'a str>,
    pub out_dir: &'a Path,
}

fn parse_sets(sets: &[String]) -> Result<Vec<(String, i64)>> {
    sets.iter()
        .map(|s| {
            // Fact names may contain anything but are never numeric at the end, so split on the last `=`.
            let (name, value) = s.rsplit_once('=').with_context(|| format!("--set {s:?}: expected NAME=VALUE"))?;
            let value = value.trim().parse::<i64>().with_context(|| format!("--set {s:?}: value is not an integer"))?;
            Ok((name.to_owned(), value))
        })
        .collect()
}

pub fn edit_fact_cmd(a: &EditArgs) -> Result<ExitCode> {
    let sets = parse_sets(a.sets)?;
    if sets.is_empty() && a.removes.is_empty() && a.match_facts_of.is_none() {
        println!("note: no --set, --remove or --match-facts-of: this is an unedited repack");
    }
    for r in a.removes {
        if sets.iter().any(|(n, _)| n == r) {
            bail!("--remove {r:?} is also in --set; pick one");
        }
    }
    if let Some(s) = a.stem {
        export::check_stem(s)?;
    }
    let other = a.match_facts_of.map(facts_cmd::load).transpose()?;
    let template_bytes = std::fs::read(a.template).with_context(|| format!("reading {}", a.template.display()))?;
    let before = table_of(&template_bytes)?;

    let mut restored: Vec<String> = Vec::new();
    let bytes = export::build_save_from_document(&template_bytes, |doc| {
        if let Some(o) = &other {
            doc.edit_facts(|t| {
                for n in t.differing(o) {
                    t.match_fact(&n, o)?;
                    restored.push(n);
                }
                Ok(())
            })?;
        }
        if !a.removes.is_empty() {
            doc.edit_facts(|t| {
                for n in a.removes {
                    if t.remove(n).is_none() {
                        return Err(w3save::Error::Payload(format!("--remove {n:?}: the template has no such fact")));
                    }
                }
                Ok(())
            })?;
        }
        doc.apply(&Edits { facts: sets.clone(), ..Default::default() })
    })?;

    let stem = a.stem.map_or_else(|| export::default_stem(a.template, a.out_dir), str::to_owned);
    let written = export::write_save_set_as(a.template, a.out_dir, &stem, &bytes)?;
    println!("wrote {}", written.sav.display());
    for s in &written.sidecars {
        println!("      {}", s.display());
    }

    let touched: BTreeSet<&str> = restored
        .iter()
        .map(String::as_str)
        .chain(a.removes.iter().map(String::as_str))
        .chain(sets.iter().map(|s| s.0.as_str()))
        .collect();
    println!(
        "template {} facts, payload {} bytes; touched {}: {}",
        before.len(),
        Container::read(&template_bytes)?.payload.len(),
        touched.len(),
        touched.iter().take(12).copied().collect::<Vec<_>>().join(", ")
    );
    verify(&written.sav, &before, &sets, a.removes, other.as_ref(), &touched)?;
    Ok(ExitCode::SUCCESS)
}

fn table_of(save: &[u8]) -> Result<FactTable> {
    let c = Container::read(save)?;
    Ok(Document::parse(&c.payload, c.data_start)?.facts_table()?)
}

/// Re-read the file we just wrote, independently of the in-memory result.
fn verify(
    path: &Path,
    before: &FactTable,
    sets: &[(String, i64)],
    removes: &[String],
    other: Option<&FactTable>,
    touched: &BTreeSet<&str>,
) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("re-reading {}", path.display()))?;
    let mut problems: Vec<String> = Vec::new();

    let c = Container::read(&bytes)?;
    let doc = Document::parse(&c.payload, c.data_start)?;
    if doc.to_bytes() != c.payload {
        problems.push("typed re-parse does not serialize back to the payload".into());
    }
    if doc.table() != doc.map.variables {
        problems.push("variable table recomputed from the tree differs from the one on disk".into());
    }
    let info = w3save::inspect(&bytes)?;
    for i in info.issues.iter().filter(|i| i.severity == Severity::Error) {
        problems.push(format!("check-save: {}", i.message));
    }
    let after = doc.facts_table()?;
    let mut sums = std::collections::HashMap::new();
    for (name, value) in sets {
        sums.insert(name.as_str(), *value);
    }
    for (name, value) in &sums {
        match after.get(name) {
            Some(f) if f.sum() == *value && f.records.len() == 1 && f.records[0].is_permanent() => {}
            other => problems.push(format!("fact {name:?}: expected one permanent record with value {value}, found {other:?}")),
        }
    }
    if let Some(o) = other {
        for n in after.differing(o) {
            if !sums.contains_key(n.as_str()) {
                problems.push(format!("fact {n:?} still differs from the --match-facts-of save"));
            }
        }
    }
    for n in removes {
        if after.get(n).is_some() {
            problems.push(format!("fact {n:?} was to be removed but is still in the table"));
        }
    }
    for n in before.differing(&after) {
        if !touched.contains(n.as_str()) {
            problems.push(format!("fact {n:?} changed but was not asked for"));
        }
    }
    println!(
        "re-read: {} facts ({:+}), payload {} bytes, {} chunk(s), typed re-parse + table check {}",
        after.len(),
        after.len() as i64 - before.len() as i64,
        c.payload.len(),
        c.records.len(),
        if problems.is_empty() { "PASS" } else { "FAIL" }
    );
    if problems.is_empty() {
        println!("VERIFIED: only the requested facts changed; the file parses; no check-save errors");
    } else {
        for p in &problems {
            println!("PROBLEM: {p}");
        }
    }
    if problems.is_empty() && info.issues.is_empty() {
        println!("check-save: no problems found");
    }
    if let Some(first) = problems.first() {
        bail!("verification failed: {first}");
    }
    Ok(())
}
