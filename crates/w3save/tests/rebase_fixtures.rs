//! Fixture-gated: `Document::rebase` (PHASE0.md step 8, part 2). Skipped (passes) when
//! `W3AL_FIXTURES` is unset. Fixtures are copies of real saves and are never committed.
//!
//! 1. On every file: `RB` entry `i` is (NM name, absolute offset) of the `i`-th top-level `BS`
//!    (the claim `rebase` relies on), and `rebase` on an unedited document changes nothing.
//! 2. On a spread of saves with a facts section: grow the facts blob (several sizes), rebase,
//!    re-parse with `Document::parse` + `payload::read_map`, and check that every table entry,
//!    every `RB` entry, the NM/RB pointers and the footer land on the same logical thing as
//!    before, shifted by exactly the growth for everything behind the facts block. A shrink
//!    (remove a fact) is checked the same way, and so is the container round trip.
use std::path::{Path, PathBuf};

use w3save::{payload, Container, Document, Kind, Region};

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

fn fixtures() -> Option<Vec<PathBuf>> {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return None;
    };
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    Some(files)
}

/// (offset, size) of the node with table index `i` is unchanged / shifted / grown by the
/// facts edit, depending on where it sits relative to the facts SS.
struct Edit {
    ss_start: u32,
    ss_end: u32,
}

/// What an entry of the old table must look like in the new one.
fn expect_entry(old: payload::VarEntry, e: &Edit, delta: i64) -> payload::VarEntry {
    let (o, s) = (old.offset, old.size);
    let end = o + s;
    let shift = |x: u32| (x as i64 + delta) as u32;
    if o >= e.ss_end {
        // fully behind the edited section: moves, keeps its size
        payload::VarEntry { offset: shift(o), size: s }
    } else if o <= e.ss_start && end >= e.ss_end {
        // the SS itself or an ancestor (the `facts` BS): same start, size follows the edit
        payload::VarEntry { offset: o, size: shift(s) }
    } else {
        // in front of it (nothing in a real save sits inside the SS: it has no children)
        assert!(end <= e.ss_start, "entry {o}+{s} straddles the facts section");
        old
    }
}

fn check_rebased(path: &Path, old_payload: &[u8], doc: &Document, base: u32, delta: i64, label: &str) {
    let ctx = format!("{} [{label}]", path.display());
    let old_map = payload::read_map(old_payload, base).unwrap();
    let old_rb = payload::read_rb(old_payload, &old_map).unwrap();
    // facts section (old coordinates)
    let body = w3save::facts::find_facts_blob(old_payload, &old_map).unwrap();
    let e = Edit { ss_start: base + body.start as u32 - 6, ss_end: base + body.end as u32 };

    let new_payload = doc.to_bytes();
    assert_eq!(new_payload.len() as i64, old_payload.len() as i64 + delta, "{ctx}: size");
    let new_map = payload::read_map(&new_payload, base).unwrap_or_else(|err| panic!("{ctx}: read_map: {err}"));
    assert!(new_map.notes.is_empty(), "{ctx}: {:?}", new_map.notes);
    assert_eq!(new_map.variables, doc.map.variables, "{ctx}: in-memory map equals the written one");

    // Variable table, entry by entry.
    assert_eq!(new_map.variables.len(), old_map.variables.len(), "{ctx}");
    for (i, (old, new)) in old_map.variables.iter().zip(&new_map.variables).enumerate() {
        assert_eq!(*new, expect_entry(*old, &e, delta), "{ctx}: table entry {i}");
    }
    // RB: same names, offsets move with the blocks.
    let new_rb = payload::read_rb(&new_payload, &new_map).unwrap();
    assert_eq!(new_rb.len(), old_rb.len(), "{ctx}: RB count");
    for (i, (old, new)) in old_rb.iter().zip(&new_rb).enumerate() {
        assert_eq!(new.name, old.name, "{ctx}: RB[{i}] name");
        let want = expect_entry(payload::VarEntry { offset: old.offset, size: 0 }, &e, delta).offset;
        assert_eq!(new.offset, want, "{ctx}: RB[{i}] offset");
    }
    // Pointer block, footer, name table.
    let shift = |x: u32| (x as i64 + delta) as u32;
    assert_eq!(new_map.rb_offset, shift(old_map.rb_offset), "{ctx}: RB pointer");
    assert_eq!(new_map.nm_offset, shift(old_map.nm_offset), "{ctx}: NM pointer");
    assert_eq!(new_map.variable_table_offset, shift(old_map.variable_table_offset), "{ctx}: footer");
    assert_eq!(new_map.pre_table_bytes, old_map.pre_table_bytes, "{ctx}");
    assert_eq!(
        payload::read_names(&new_payload, &new_map).unwrap(),
        payload::read_names(old_payload, &old_map).unwrap(),
        "{ctx}: names"
    );

    // The typed tree of the result: parses, round-trips, table == tree, and only the facts BS
    // and the two gaps differ from the original tree.
    let again = Document::parse(&new_payload, base).unwrap_or_else(|err| panic!("{ctx}: parse: {err}"));
    assert_eq!(again.to_bytes(), new_payload, "{ctx}: typed round trip");
    assert_eq!(again.table(), new_map.variables, "{ctx}: table == tree");
    let orig = Document::parse(old_payload, base).unwrap();
    assert_eq!(orig.regions.len(), again.regions.len(), "{ctx}: region count");
    let (mut var_diff, mut gap_diff) = (0, 0);
    for (a, b) in orig.regions.iter().zip(&again.regions) {
        match (a, b) {
            (Region::Variable { node: x, .. }, Region::Variable { node: y, .. }) => var_diff += (x != y) as usize,
            (Region::Gap(x), Region::Gap(y)) => gap_diff += (x != y) as usize,
            _ => panic!("{ctx}: region kinds changed"),
        }
    }
    assert_eq!(var_diff, 1, "{ctx}: only the `facts` BS differs");
    assert_eq!(gap_diff, 2, "{ctx}: only the RB gap and the tail gap differ");
}

#[test]
fn rb_entries_are_the_top_level_blocks_and_rebase_is_a_noop() {
    let Some(files) = fixtures() else { return };
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let c = Container::read(&bytes).unwrap();
        let Ok(mut doc) = Document::parse(&c.payload, c.data_start) else { continue };
        // The claim: RB[i] = (name, absolute offset) of the i-th top-level BS.
        let rb = payload::read_rb(&c.payload, &doc.map).unwrap();
        let mut at = 0usize;
        let mut blocks = Vec::new();
        for r in &doc.regions {
            if let Region::Variable { node, .. } = r {
                if let Kind::Block(b) = &node.kind {
                    blocks.push((b.name, c.data_start + at as u32));
                }
            }
            at += r.len();
        }
        let got: Vec<_> = rb.iter().map(|e| (e.name, e.offset)).collect();
        if got != blocks {
            failures.push(format!("{}: RB is not the top-level BS list", f.display()));
            continue;
        }
        if let Err(e) = doc.rebase() {
            failures.push(format!("{}: rebase: {e}", f.display()));
            continue;
        }
        if doc.to_bytes() != c.payload {
            failures.push(format!("{}: rebase changed an unedited document", f.display()));
        }
        checked += 1;
    }
    eprintln!("{checked} of {} saves: RB == top-level BS list, rebase is a byte-exact no-op", files.len());
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn growing_and_shrinking_the_facts_blob_moves_every_pointer() {
    let Some(files) = fixtures() else { return };
    // A spread of saves that have a recognised facts section (old-format saves have none).
    let mut candidates = Vec::new();
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let c = Container::read(&bytes).unwrap();
        if let Ok(doc) = Document::parse(&c.payload, c.data_start) {
            if doc.facts_table().is_ok() {
                candidates.push(f.clone());
            }
        }
    }
    let step = (candidates.len() / 8).max(1);
    let picked: Vec<_> = candidates.iter().step_by(step).take(10).collect();
    assert!(picked.len() >= 5 || candidates.len() < 5, "fewer than 5 usable saves in the fixture folder");

    for f in &picked {
        let bytes = std::fs::read(f).unwrap();
        let c = Container::read(&bytes).unwrap();
        let base = c.data_start;
        let table = Document::parse(&c.payload, base).unwrap().facts_table().unwrap();

        // Growth: 1 new fact (+~20 bytes), many new facts with long names (+kB), extra
        // records on an existing fact. The raw tree edit goes through `edit_facts`.
        for (label, n_new, name_len) in [("1 fact", 1usize, 12usize), ("500 facts", 500, 40), ("5000 long", 5000, 100)] {
            let mut doc = Document::parse(&c.payload, base).unwrap();
            doc.edit_facts(|t| {
                for i in 0..n_new {
                    let name = format!("{:0<width$}", format!("w3al_rebase_test_{i}_"), width = name_len);
                    t.set_value(&name, 1, 1.0)?;
                }
                Ok(())
            })
            .unwrap();
            let delta = doc.to_bytes().len() as i64 - c.payload.len() as i64;
            assert!(delta > 0, "{label}");
            check_rebased(f, &c.payload, &doc, base, delta, label);
            assert_eq!(doc.facts_table().unwrap().len(), table.len() + n_new);

            // The result survives the container: write, read, same payload, same facts.
            let mut c2 = c.clone();
            c2.payload = doc.to_bytes();
            let file = c2.write().unwrap();
            let back = Container::read(&file).unwrap();
            assert_eq!(back.payload, c2.payload, "{}: container round trip", f.display());
        }

        // Shrink: drop 3 facts.
        let mut doc = Document::parse(&c.payload, base).unwrap();
        let names: Vec<String> = table.facts().iter().take(3).map(|f| f.name.clone()).collect();
        doc.edit_facts(|t| {
            for n in &names {
                t.remove(n);
            }
            Ok(())
        })
        .unwrap();
        let delta = doc.to_bytes().len() as i64 - c.payload.len() as i64;
        assert!(delta < 0);
        check_rebased(f, &c.payload, &doc, base, delta, "remove 3 facts");

        // Zero growth through the same path: set a fact to its current single-record value is
        // not byte-identical (time changes), but an identical table must be.
        let mut doc = Document::parse(&c.payload, base).unwrap();
        doc.edit_facts(|_| Ok(())).unwrap();
        assert_eq!(doc.to_bytes(), c.payload, "{}: no-op edit_facts", f.display());
    }
    eprintln!("growth, shrink and no-op checked on {} saves: {}", picked.len(), picked.iter().map(|p| p.file_name().unwrap().to_string_lossy()).collect::<Vec<_>>().join(", "));
}
