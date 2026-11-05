//! Fixture-gated (skipped when `W3AL_FIXTURES` is unset): the quest-log reader and editor on real saves.
//!
//! * every 5.x save: strict `journal::read`, `Size` == entries, flags rule, unique keys, post-order table indices;
//! * consecutive numbered saves of one playthrough (`NN. title` folders, the lukwraith set): the game only
//!   appends entries and keeps the counters' relative order, and `apply(diff(A, B))` on A reproduces B's quest log;
//! * lukwraith 21 -> 22 (mq0005 completed): the numbers in docs/findings/quest-state.md, and the edited 21 has
//!   byte-identical `CJournalManager` and `CWitcherJournalManager` blocks to the game's own 22;
//! * a single status flip on 21 changes two bytes and re-parses as expected.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use w3save::journal::{self, StatusChange};
use w3save::{payload, Container, Document};

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
    let dir = std::env::var_os("W3AL_FIXTURES")?;
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    Some(files)
}

fn load(p: &Path) -> (Vec<u8>, Document) {
    let c = Container::read(&std::fs::read(p).unwrap()).unwrap();
    let d = Document::parse(&c.payload, c.data_start).unwrap();
    (c.payload, d)
}

fn is_5x(d: &Document) -> bool {
    matches!(d.map.header_ints, [66 | 67, 29, 164])
}

#[test]
fn journal_reads_strictly_on_every_5x_fixture() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (mut ok, mut entries, mut old) = (0usize, 0usize, 0usize);
    let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
    let mut depths: BTreeMap<usize, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for f in &files {
        let (_, d) = load(f);
        if !is_5x(&d) {
            old += 1;
            continue;
        }
        let r = (|| -> Result<(), String> {
            if !d.indices_are_post_order() {
                return Err("table indices are not post-order".into());
            }
            let j = journal::read(&d).map_err(|e| e.to_string())?;
            if j.size as usize != j.entries.len() {
                return Err(format!("Size {} != {}", j.size, j.entries.len()));
            }
            if j.counters_size.is_some_and(|s| s as usize != j.counters.len()) {
                return Err("counter Size".into());
            }
            if j.duplicate_keys() != 0 {
                return Err(format!("{} duplicate keys", j.duplicate_keys()));
            }
            for e in &j.entries {
                let n = e.path.len();
                if !(1..=journal::MAX_PATH_STEPS).contains(&n) || e.path.iter().enumerate().any(|(i, s)| s.flags != u8::from(i + 1 == n)) {
                    return Err(format!("entry {} breaks the flags/depth rule", e.label()));
                }
                *statuses.entry(e.status.clone()).or_default() += 1;
                *depths.entry(n).or_default() += 1;
            }
            entries += j.entries.len();
            Ok(())
        })();
        match r {
            Ok(()) => ok += 1,
            Err(e) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!("journal: {ok} 5.x saves read, {entries} entries, statuses {statuses:?}, path depths {depths:?}; {old} older saves skipped");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Saves in `NN. title` folders, grouped by playthrough folder, in folder-number order.
fn numbered(files: &[PathBuf]) -> BTreeMap<PathBuf, Vec<(u32, PathBuf)>> {
    let mut sets: BTreeMap<PathBuf, Vec<(u32, PathBuf)>> = BTreeMap::new();
    for f in files {
        let Some(folder) = f.parent() else { continue };
        let name = folder.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let Some((num, _)) = name.split_once(". ") else { continue };
        let Ok(n) = num.parse::<u32>() else { continue };
        sets.entry(folder.parent().unwrap().to_owned()).or_default().push((n, f.clone()));
    }
    for v in sets.values_mut() {
        v.sort();
    }
    sets
}

#[test]
fn consecutive_saves_are_append_only_and_diffs_replay() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (mut pairs, mut active_exact, mut witcher_exact, mut rehashed, mut unmodelled, mut new_names) = (0usize, 0, 0, 0, 0, 0);
    let mut failures = Vec::new();
    for (set, list) in numbered(&files) {
        for w in list.windows(2) {
            let ((na, fa), (nb, fb)) = (&w[0], &w[1]);
            let (_, mut da) = load(fa);
            let (_, db) = load(fb);
            if !is_5x(&da) {
                continue;
            }
            pairs += 1;
            let (ja, jb) = (journal::read(&da).unwrap(), journal::read(&db).unwrap());
            let d = journal::diff(&ja, &jb);
            let tag = format!("{} {na:02}->{nb:02}", set.display());
            // The game never removes or reorders entries.
            if !d.append_only || !d.removed.is_empty() || !d.counters_removed.is_empty() {
                failures.push(format!("{tag}: append_only={} removed={}", d.append_only, d.removed.len()));
                continue;
            }
            let mut edit = d.to_edit().unwrap();
            // 01 -> 02: the first save has no `JS_Success` name yet (the game appends it, so do we).
            edit.allow_new_names = true;
            if !edit.is_empty() {
                match journal::apply(&mut da, &edit) {
                    Ok(r) => new_names += r.names_added.len(),
                    Err(e) => {
                        failures.push(format!("{tag}: apply failed: {e}"));
                        continue;
                    }
                }
            }
            let mut got = journal::read(&da).unwrap();
            let mut want = jb.clone();
            if !d.counters_order_kept {
                // The game re-hashed its counter map (all counters moved); compare them as a set.
                rehashed += 1;
                got.counters.sort_by_key(|c| c.guid);
                want.counters.sort_by_key(|c| c.guid);
            }
            if got != want {
                failures.push(format!("{tag}: replayed journal differs from B"));
                continue;
            }
            let same = |n: &str| journal::top_block_bytes(&da, n) == journal::top_block_bytes(&db, n);
            let names_same = journal::read(&da).is_ok() && edit.allow_new_names && d.size.0 > 0 && da.name_index("JS_Success") == db.name_index("JS_Success");
            if same("CJournalManager") {
                active_exact += 1;
            } else if names_same {
                failures.push(format!("{tag}: CJournalManager not byte-exact although the status names have the same indices"));
            }
            if same("CWitcherJournalManager") {
                witcher_exact += 1;
            } else if !journal::unmodelled_changes(&da, &db).is_empty() {
                unmodelled += 1;
            } else if d.counters_order_kept {
                failures.push(format!("{tag}: CWitcherJournalManager not byte-exact and no unmodelled table changed"));
            }
            // The edited document is a consistent save.
            let q = da.to_bytes();
            let again = Document::parse(&q, da.base).unwrap();
            assert_eq!(again.table(), payload::read_map(&q, da.base).unwrap().variables, "{tag}");
            assert!(again.indices_are_post_order(), "{tag}");
        }
    }
    eprintln!(
        "journal replay: {pairs} consecutive 5.x pairs; CJournalManager byte-exact {active_exact}; \
         CWitcherJournalManager byte-exact {witcher_exact}, {unmodelled} differ only in unmodelled side tables, \
         {rehashed} pairs where the game re-hashed the counters; {new_names} NM name(s) appended"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn find(files: &[PathBuf], suffix: &str) -> Option<PathBuf> {
    files.iter().find(|f| f.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.ends_with(suffix))).cloned()
}

/// lukwraith 21 (`ManualSave_fd3e1_7ea4d800_3f4cf08`, facing Napp) and 22 (`ManualSave_8559a_7ea4d800_4038706`).
fn pair_21_22() -> Option<(PathBuf, PathBuf)> {
    let files = fixtures()?;
    let a = find(&files, "_3f4cf08.sav");
    let b = find(&files, "_4038706.sav");
    if a.is_none() || b.is_none() {
        eprintln!("lukwraith saves 21/22 not under W3AL_FIXTURES; skipping");
    }
    Some((a?, b?))
}

#[test]
fn lukwraith_21_to_22_matches_the_findings_and_replays_byte_exact() {
    let Some((fa, fb)) = pair_21_22() else { return };
    let (pa, mut da) = load(&fa);
    let (_, db) = load(&fb);
    let (ja, jb) = (journal::read(&da).unwrap(), journal::read(&db).unwrap());
    let d = journal::diff(&ja, &jb);
    assert_eq!(d.size, (208, 214));
    assert_eq!(d.changed.len(), 5);
    assert!(d.changed.iter().all(|c| c.before.status == "JS_Active" && c.after.status == "JS_Success" && c.before.unread == c.after.unread));
    assert!(d.changed.iter().any(|c| c.after.label() == "minorquests/mq0005dwarfsmith"), "the quest-level entry flips");
    assert_eq!(d.added.len(), 6);
    assert_eq!(d.added.iter().filter(|(_, e)| e.label().contains("mq0005dwarfsmith")).count(), 2);
    assert!(d.append_only);
    assert_eq!(d.counters_added.len(), 1);
    assert_eq!(d.counters_added[0].0, 27, "the game put the new counter in hash order, not at the end");
    assert!(d.counters_order_kept && d.counters_changed.is_empty());
    assert_eq!(journal::guid_hex(&d.tracked_quest.unwrap().1.unwrap()), "7c198a768adc3046b34b5a730b5b53c9");

    let report = journal::apply(&mut da, &d.to_edit().unwrap()).unwrap();
    assert_eq!((report.status_set, report.appended, report.counters_appended, report.tracking_set), (5, 6, 1, 2));
    // An entry is BS entry + BS path + 3 VL per step + VL status + VL unread; a counter is BS + 2 VL.
    let new_nodes = d.added.iter().map(|(_, e)| 4 + 3 * e.path.len()).sum::<usize>() + 3;
    assert_eq!(report.nodes.1, report.nodes.0 + new_nodes);
    for name in ["CJournalManager", "CWitcherJournalManager"] {
        assert_eq!(journal::top_block_bytes(&da, name), journal::top_block_bytes(&db, name), "{name} bytes differ from save 22");
    }
    // Everything outside the two journal blocks is still save 21's.
    let q = da.to_bytes();
    let again = Document::parse(&q, da.base).unwrap();
    assert_eq!(again.to_bytes(), q);
    assert_eq!(again.table(), payload::read_map(&q, da.base).unwrap().variables);
    let (_, orig) = load(&fa);
    for (x, y) in orig.top_nodes().zip(again.top_nodes()) {
        let name = match &x.kind {
            w3save::Kind::Block(b) => orig.name(b.name).unwrap_or("?").to_string(),
            _ => x.tag().to_string(),
        };
        if name != "CJournalManager" && name != "CWitcherJournalManager" {
            assert_eq!(x.to_bytes(), y.to_bytes(), "{name} changed");
        }
    }
    let grown = journal::top_block_bytes(&db, "CJournalManager").unwrap().len() + journal::top_block_bytes(&db, "CWitcherJournalManager").unwrap().len()
        - journal::top_block_bytes(&orig, "CJournalManager").unwrap().len()
        - journal::top_block_bytes(&orig, "CWitcherJournalManager").unwrap().len();
    assert_eq!(q.len(), pa.len() + grown + 8 * new_nodes, "payload grows by the blocks plus 8 table bytes per new node");
}

#[test]
fn lukwraith_21_single_flip_is_two_bytes() {
    let Some((fa, _)) = pair_21_22() else { return };
    let (pa, mut da) = load(&fa);
    let key = journal::read(&da).unwrap().entries[202].key();
    let edit = journal::JournalEdit {
        status: vec![StatusChange { key: key.clone(), from: Some("JS_Active".into()), to: "JS_Success".into(), unread: None }],
        ..Default::default()
    };
    journal::apply(&mut da, &edit).unwrap();
    let q = da.to_bytes();
    assert_eq!(q.len(), pa.len());
    let diffs: Vec<usize> = (0..q.len()).filter(|&i| q[i] != pa[i]).collect();
    assert!(!diffs.is_empty() && diffs.len() <= 2 && diffs.last().unwrap() - diffs[0] <= 1, "{diffs:?}");
    let j = journal::read(&Document::parse(&q, da.base).unwrap()).unwrap();
    let before = journal::read(&load(&fa).1).unwrap();
    for (i, (x, y)) in before.entries.iter().zip(&j.entries).enumerate() {
        if i == 202 {
            assert_eq!((x.status.as_str(), y.status.as_str(), y.key()), ("JS_Active", "JS_Success", key.clone()));
        } else {
            assert_eq!(x, y);
        }
    }
}
