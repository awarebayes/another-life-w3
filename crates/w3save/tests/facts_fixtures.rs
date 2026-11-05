//! Fixture-gated: locate and parse the facts blob of every real save under `W3AL_FIXTURES`,
//! and check that `parse` + `to_bytes` reproduces the `SS` body byte for byte, that the
//! `SBDF` count equals the parsed fact count, and that the inferred `c1` rule holds.
//! Skipped (passes) when the variable is unset. Fixtures are copies of real saves and are
//! never committed.
//!
//! Old game versions (pre-5.x, e.g. the saves in `C:\w3-fixtures`) may use a different
//! layout. Set `W3AL_FACTS_ALLOW_UNSUPPORTED=1` to report those as a count instead of failing.
use std::path::{Path, PathBuf};

use w3save::facts::{self, FactTable};
use w3save::{payload, Container};

fn saves(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            saves(&p, out);
        } else if p.extension().is_some_and(|x| x == "sav") {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct Totals {
    facts: usize,
    c1_mismatch: usize,
}

fn check(f: &Path, totals: &mut Totals) -> w3save::Result<usize> {
    let c = Container::read(&std::fs::read(f)?)?;
    let map = payload::read_map(&c.payload, c.data_start)?;
    let range = facts::find_facts_blob(&c.payload, &map)?;
    let body = &c.payload[range.clone()];
    let table = FactTable::parse(body)?;
    let declared = u32::from_le_bytes(body[20..24].try_into().unwrap()) as usize;
    if declared != table.len() {
        return Err(w3save::Error::Payload(format!("SBDF count {declared} != parsed {}", table.len())));
    }
    if table.to_bytes() != body {
        return Err(w3save::Error::Payload("parse + to_bytes is not byte-exact".into()));
    }
    // The enclosing SS variable can be rebuilt exactly, too.
    if table.to_ss_variable() != c.payload[range.start - 6..range.end] {
        return Err(w3save::Error::Payload("to_ss_variable differs from the original variable".into()));
    }
    // The 5.0.0c damage check must be quiet on real, undamaged fact names.
    if let Some(i) = table.check_ids().first() {
        return Err(w3save::Error::Payload(format!("check_ids: {}", i.message)));
    }
    for fact in table.facts() {
        totals.facts += 1;
        if fact.c1 as usize != fact.timed_count() {
            totals.c1_mismatch += 1;
        }
    }
    Ok(table.len())
}

#[test]
fn facts_round_trip_byte_exact_on_every_fixture() {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let allow_unsupported = std::env::var_os("W3AL_FACTS_ALLOW_UNSUPPORTED").is_some();
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    let (mut totals, mut passed) = (Totals::default(), 0usize);
    let mut failures = Vec::new();
    for f in &files {
        match check(f, &mut totals) {
            Ok(n) if n > 0 => passed += 1,
            Ok(_) => failures.push(format!("{}: zero facts", f.display())),
            Err(e) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!(
        "facts: {passed}/{} saves ok, {} facts, {} with c1 != timed-record count",
        files.len(),
        totals.facts,
        totals.c1_mismatch
    );
    if allow_unsupported {
        for f in &failures {
            eprintln!("unsupported: {f}");
        }
    } else {
        assert!(failures.is_empty(), "{} of {} saves failed:\n{}", failures.len(), files.len(), failures.join("\n"));
        assert_eq!(totals.c1_mismatch, 0, "the c1 = timed-record-count rule broke");
    }
}
