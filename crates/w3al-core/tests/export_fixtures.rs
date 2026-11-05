//! Fixture-gated (skipped when `W3AL_FIXTURES` is unset): the product export path (questbook TOML ->
//! `resolve` -> `export`) with graph + journal + fact effects, on real saves
//! (docs/findings/export-wiring.md).
//!
//! Environment (all folders are searched recursively for `<stem>.sav`; nothing is written there):
//! * `W3AL_FIXTURES` - template and lukwraith donors (`C:\w3-extract`);
//! * `W3AL_DONORS`   - more donor saves, e.g. the user's donor S (`C:\w3-fixtures\donor_S`); optional;
//! * `W3AL_GATES`    - the gate saves built by `w3al journal-set` and passed in game (`C:\w3-out`);
//!   optional: without it the exports still run and verify, but are not compared to the gates.
//!
//! Test questbooks are in `tests/data/gate_*` (built from the gate delta files; not real content).
//! Output goes to a fresh folder under the system temp dir.
use std::path::{Path, PathBuf};

use w3al_core::export::{self, ExportError, ExportOptions};
use w3al_core::graph_effect;
use w3al_core::run::{QuestEntry, Status};
use w3al_core::{resolve, validate, Quest, Questbook, Run};
use w3save::graph;
use w3save::{Container, Document};

const T21: &str = "ManualSave_fd3e1_7ea4d800_3f4cf08"; // lukwraith 21, before Napp
const D22: &str = "ManualSave_8559a_7ea4d800_4038706"; // lukwraith 22, Napp brought to the smith
const DS: &str = "ManualSave_8559a_7ea51000_3ad835"; // donor S (the user's play from 21, Napp spared)
const GATE_G1: &str = "gate_G/ManualSave_8559a_7ea50c00_50f4f2b.sav";
const GATE_R1: &str = "gate_R/ManualSave_8559a_7ea50c00_54b3041.sav";

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

fn fixtures_set() -> bool {
    std::env::var_os("W3AL_FIXTURES").is_some()
}

/// First (sorted) `<stem>.sav` under `W3AL_FIXTURES` or `W3AL_DONORS`.
fn find_save(stem: &str) -> Option<PathBuf> {
    let mut files = Vec::new();
    for var in ["W3AL_FIXTURES", "W3AL_DONORS"] {
        if let Some(d) = std::env::var_os(var) {
            saves(Path::new(&d), &mut files);
        }
    }
    files.sort();
    let want = format!("{stem}.sav");
    files.into_iter().find(|f| f.file_name().is_some_and(|n| n.to_string_lossy() == want))
}

fn gate(rel: &str) -> Option<PathBuf> {
    let p = Path::new(&std::env::var_os("W3AL_GATES")?).join(rel);
    p.is_file().then_some(p)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("w3al-export-fx-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn quests(dir: &str) -> Vec<Quest> {
    let book = Questbook::load_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(dir)).unwrap();
    let errors: Vec<_> = validate::lint(&book).into_iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{dir}: {errors:?}");
    book.quests
}

fn run_all(book: &Questbook) -> Run {
    let mut run = Run::default();
    for q in &book.quests {
        run.quests.insert(q.id.clone(), QuestEntry { status: Status::Completed, answers: Default::default() });
    }
    run
}

/// Folders of these donor saves, or `None` (skip) when one is not on this machine.
fn donor_dirs(stems: &[&str]) -> Option<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for s in stems {
        let Some(p) = find_save(s) else {
            eprintln!("skip: donor {s}.sav not found (W3AL_FIXTURES / W3AL_DONORS)");
            return None;
        };
        dirs.push(p.parent().unwrap().to_owned());
    }
    Some(dirs)
}

fn load(p: &Path) -> Document {
    let c = Container::read(&std::fs::read(p).unwrap()).unwrap();
    Document::parse(&c.payload, c.data_start).unwrap()
}

/// Our export against a gate save: the four quest-state blocks byte for byte, the NM table, the fact
/// table ignoring record times, and every other top-level block byte for byte. Returns whether the
/// whole decoded payload is identical too (informational: fact times are stamped at build time).
fn assert_reproduces(ours: &Path, gate: &Path) -> bool {
    let (a, b) = (load(ours), load(gate));
    assert_eq!(a.names, b.names, "NM name table");
    for name in ["questSystem", "community", "CJournalManager", "CWitcherJournalManager"] {
        let (x, y) = (graph::top_block(&a, name).unwrap(), graph::top_block(&b, name).unwrap());
        assert!(x.to_bytes() == y.to_bytes(), "{name} differs from {}", gate.display());
    }
    let (fa, fb) = (a.facts_table().unwrap(), b.facts_table().unwrap());
    assert_eq!(fa.len(), fb.len());
    for (x, y) in fa.facts().iter().zip(fb.facts()) {
        assert_eq!((&x.name, x.c1, x.records.len()), (&y.name, y.c1, y.records.len()));
        for (r, s) in x.records.iter().zip(&y.records) {
            assert_eq!((r.value, r.expiry.to_bits()), (s.value, s.expiry.to_bits()), "fact {}", x.name);
        }
    }
    let (ta, tb): (Vec<_>, Vec<_>) = (a.top_nodes().collect(), b.top_nodes().collect());
    assert_eq!(ta.len(), tb.len());
    for (x, y) in ta.iter().zip(&tb) {
        let name = graph::node_name(&a, x);
        if name != "facts" {
            assert!(x.to_bytes() == y.to_bytes(), "top-level {name} differs");
        }
    }
    a.to_bytes() == b.to_bytes()
}

fn export(quests: Vec<Quest>, template: &Path, opts: &ExportOptions, out: &Path) -> Result<export::ExportResult, ExportError> {
    let book = Questbook::from_quests(quests);
    let res = resolve(&book, &run_all(&book));
    assert!(!res.has_errors(), "{:?}", res.diagnostics);
    export::export_with(template, out, &res, opts)
}

#[test]
fn g1_reproduces_gate_g() {
    if !fixtures_set() {
        eprintln!("skip: W3AL_FIXTURES not set");
        return;
    }
    let (Some(t21), Some(dirs)) = (find_save(T21), donor_dirs(&[D22])) else { return };
    let out = scratch("g1");
    let opts = ExportOptions { stem: Some("G1_export".into()), donor_dirs: dirs, ..Default::default() };
    let r = export(quests("gate_g1"), &t21, &opts, &out).unwrap();
    assert_eq!(r.effects_applied, 1);
    assert!(r.notes[0].contains("36 NM names appended") && !r.notes[0].contains("REMAPPED"), "{:?}", r.notes);
    assert!(r.sidecars.iter().any(|s| s.extension().is_some_and(|e| e == "png")));
    match gate(GATE_G1) {
        Some(g) => {
            let whole = assert_reproduces(&r.sav, &g);
            eprintln!("G1: blocks reproduced; whole decoded payload identical: {whole}");
        }
        None => eprintln!("W3AL_GATES not set: G1 exported and verified, not compared"),
    }
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn r1_reproduces_gate_r_with_two_donors() {
    if !fixtures_set() {
        eprintln!("skip: W3AL_FIXTURES not set");
        return;
    }
    // mq0004's delta (donor lukwraith 23) only removes nodes, so only donor S must be on disk.
    let (Some(t21), Some(dirs)) = (find_save(T21), donor_dirs(&[DS])) else { return };
    let out = scratch("r1");
    let opts = ExportOptions { stem: Some("R1_export".into()), donor_dirs: dirs, ..Default::default() };
    let r = export(quests("gate_r1"), &t21, &opts, &out).unwrap();
    assert_eq!(r.effects_applied, 2);
    assert!(r.notes[0].starts_with("mq0004_done/done: graph 0 replaced, 9 removed") && r.notes[0].contains("0 NM names appended"), "{:?}", r.notes);
    assert!(r.notes[1].contains("6 NM names appended"), "{:?}", r.notes);
    match gate(GATE_R1) {
        Some(g) => {
            let whole = assert_reproduces(&r.sav, &g);
            eprintln!("R1: blocks reproduced; whole decoded payload identical: {whole}");
        }
        None => eprintln!("W3AL_GATES not set: R1 exported and verified, not compared"),
    }
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn overlapping_deltas_from_two_donors_are_refused() {
    if !fixtures_set() {
        return;
    }
    let (Some(t21), Some(dirs)) = (find_save(T21), donor_dirs(&[D22, DS])) else { return };
    let out = scratch("overlap");
    // Two branches of the same quest (brought vs spared) both rewrite the mq0005 scope.
    let mut qs = quests("gate_g1");
    qs.extend(quests("gate_r1").into_iter().filter(|q| q.id == "mq0005_spared"));
    let opts = ExportOptions { donor_dirs: dirs, ..Default::default() };
    let err = export(qs, &t21, &opts, &out).unwrap_err();
    assert!(matches!(err, ExportError::Effect(ref m) if m.contains("overlaps")), "{err}");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0, "nothing written");
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn guards_refuse_a_template_the_delta_was_not_captured_on() {
    if !fixtures_set() {
        return;
    }
    // G1's delta on save 22 (where the quest is already done): its `before` guards do not match.
    let (Some(t22), Some(dirs)) = (find_save(D22), donor_dirs(&[D22])) else { return };
    let out = scratch("guards");
    let opts = ExportOptions { donor_dirs: dirs, ..Default::default() };
    let err = export(quests("gate_g1"), &t22, &opts, &out).unwrap_err();
    assert!(matches!(err, ExportError::Effect(ref m) if m.contains("not the one the delta was captured from")), "{err}");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0, "nothing written");
    std::fs::remove_dir_all(&out).unwrap();
}

/// A second donor whose names do not extend the save's: refused unless remapping is allowed; then the
/// copied subtree verifies by resolved fingerprint. Offline only (HYPOTHESIS in game).
#[test]
fn second_donor_with_other_names_needs_remap() {
    if !fixtures_set() {
        return;
    }
    let (Some(t21), Some(dirs)) = (find_save(T21), donor_dirs(&[D22, DS])) else { return };
    let mut qs = quests("gate_g1");
    // One op of donor S that touches no node of G1's delta: S's epilogue thread, a sibling of G1's.
    let scope = "questSystem/quest[witcher3_quest.w2quest]/questThread/questThread[7b125918]/questThread/questThread[f0b52e98]/questThread/questThread[b4e73851]/questThread/questThread[38424c39]/questThread";
    let extra = format!(
        r##"id = "s_epilogue"
title = "t"
act = 1
order = 20
kind = "secondary"
region = "white_orchard"
level = 1
summary = "s"
done_effects = "done"
[effects.done]
source = "test: one insert from donor S"
[effects.done.graph]
donor = "{DS}"
ops = [
  {{ op = "insert", path = "{scope}/questThread[4f5f58fc]", after = "#f51dc20a" }},
  {{ op = "recount", path = "{scope}/numThreads", of = "questThread" }},
]
"##
    );
    qs.push(toml::from_str(&extra).unwrap());

    let out = scratch("remap");
    let opts = ExportOptions { donor_dirs: dirs, ..Default::default() };
    let err = export(qs.clone(), &t21, &opts, &out).unwrap_err();
    assert!(matches!(err, ExportError::Effect(ref m) if m.contains("not a prefix") && m.contains("allow_name_remap")), "{err}");
    let opts = ExportOptions { allow_name_remap: true, ..opts };
    let r = export(qs, &t21, &opts, &out).unwrap();
    assert!(r.notes[1].contains("REMAPPED"), "{:?}", r.notes);
    let (ours, donor) = (load(&r.sav), load(&find_save(DS).unwrap()));
    let p = format!("{scope}/questThread[4f5f58fc]");
    let fp = |d: &Document| graph::resolved_fingerprint(d, graph::node_at(d, &graph::locate(d, &p).unwrap().unwrap())).unwrap();
    assert_eq!(fp(&ours), fp(&donor));
    std::fs::remove_dir_all(&out).unwrap();
}

/// Every graph delta written for the in-game gates parses with the shared parser and passes lint rules.
#[test]
fn gate_delta_files_parse() {
    let Some(root) = std::env::var_os("W3AL_GATES") else {
        eprintln!("skip: W3AL_GATES not set");
        return;
    };
    let mut n = 0;
    for gate in std::fs::read_dir(&root).unwrap().flatten() {
        let Ok(rd) = std::fs::read_dir(gate.path().join("deltas")) else { continue };
        for f in rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "toml")) {
            let text = std::fs::read_to_string(&f).unwrap();
            for (key, g) in graph_effect::tables_from_toml(&text).unwrap_or_else(|e| panic!("{}: {e}", f.display())) {
                let ops = g.to_ops().unwrap_or_else(|e| panic!("{} {key}: {e}", f.display()));
                n += ops.len();
            }
        }
    }
    eprintln!("{n} gate delta ops parsed");
}
