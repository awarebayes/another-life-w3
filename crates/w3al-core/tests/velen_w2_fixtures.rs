//! Act-2 prototype, Velen decision window W2 (docs/plan/prototype-velen.md): the real questbook quests
//! `for_the_advancement_of_learning` (Keira: Kaer Morhen / Radovid / killed) and `ghosts_of_the_past` (Letho:
//! invited / elsewhere), exported from the template T_keira through the product path (questbook -> `resolve`
//! -> `export::export_with`) for all 3 x 2 combinations.
//!
//! The first test needs no saves. The others are fixture-gated (skipped when a save is missing). Environment
//! (folders are searched recursively for `<stem>.sav`; nothing is written there):
//! * `W3AL_FIXTURES` - one folder (e.g. `C:\w3-fixtures`);
//! * `W3AL_DONORS`   - a path list (`;`-separated on Windows), e.g. `C:\w3-fixtures\session_V1;C:\w3-fixtures\session_V2`;
//! * `W3AL_GATES`    - the gate folder (`C:\w3-out`), optional: KM + invited is compared with gate X1.
//!
//! Output goes to a fresh folder under the system temp dir and is removed afterwards.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use w3al_core::export::{self, ExportOptions};
use w3al_core::run::{QuestEntry, Status};
use w3al_core::{resolve, validate, Effect, Questbook, Run};
use w3save::graph::{self, GraftOp};
use w3save::journal;
use w3save::{Container, Document, Severity};

const TEMPLATE: &str = "ManualSave_568ef_7ea51000_923cb5"; // T_keira (user re-save of the Nexus pack)
const KEIRA_Q: &str = "for_the_advancement_of_learning";
const LETHO_Q: &str = "ghosts_of_the_past";
/// (option id, effects key, donor stem)
const KEIRA: [(&str, &str, &str); 3] = [
    ("kaer_morhen", "to_kaer_morhen", "ManualSave_568ef_7ea51000_94abd5"),
    ("radovid", "to_radovid", "ManualSave_568ef_7ea51000_97cdc9"),
    ("killed", "killed", "ManualSave_52584_7ea51000_96b66e"),
];
const LETHO: [(&str, &str, &str); 2] = [
    ("kaer_morhen", "invited", "ManualSave_52584_7ea51000_a4841f"),
    ("elsewhere", "elsewhere", "ManualSave_52584_7ea51000_9d341b"),
];
const GATE_X1: &str = "gate_X/ManualSave_568ef_7ea51000_9248b5.sav";
/// Blocks the W2 effects touch (besides `facts`); every other top-level node must stay the template's.
const TOUCHED: [&str; 4] = ["questSystem", "community", "CJournalManager", "CWitcherJournalManager"];

fn book() -> Questbook {
    Questbook::load_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../questbook")).unwrap()
}

fn effect<'a>(book: &'a Questbook, quest: &str, key: &str) -> &'a Effect {
    &book.get(quest).unwrap().effects[key]
}

/// The questbook side, without saves: both quests lint clean, every option carries a captured graph +
/// journal + facts effect from the expected donor, and no Keira option overlaps a Letho option (the
/// composition rule export applies: equal or nested op paths from different donors are refused).
#[test]
fn w2_quests_are_wired() {
    let book = book();
    let errors: Vec<_> = validate::lint(&book).into_iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    for (quest, opts) in [(KEIRA_Q, &KEIRA[..]), (LETHO_Q, &LETHO[..])] {
        let q = book.get(quest).unwrap_or_else(|| panic!("{quest} missing"));
        assert_eq!(q.choices.len(), 1);
        // Options built from donors come first; later options (Letho `abandoned`) are synthesized from the graphs.
        let ids: Vec<_> = q.choices[0].options.iter().map(|o| (o.id.as_str(), o.effects.as_deref())).collect();
        assert_eq!(ids[..opts.len()], opts.iter().map(|(o, k, _)| (*o, Some(*k))).collect::<Vec<_>>()[..]);
        for o in &q.choices[0].options[opts.len()..] {
            let key = o.effects.as_deref().unwrap_or_else(|| panic!("{quest}/{} has no effects", o.id));
            assert!(effect(&book, quest, key).synth.is_some(), "{quest}/{key}: extra options must be synthesized");
        }
        for (_, key, donor) in opts {
            let e = effect(&book, quest, key);
            assert!(!e.placeholder && !e.facts.is_empty() && e.has_journal_edit(), "{quest}/{key}");
            assert!(e.source.contains(TEMPLATE) && e.source.contains(donor), "{quest}/{key}: {}", e.source);
            let g = e.graph.as_ref().unwrap();
            assert_eq!(g.donor.as_deref(), Some(*donor));
            e.graph_ops().unwrap();
            e.journal_edit().unwrap();
        }
    }
    let paths = |e: &Effect| -> Vec<String> {
        e.graph_ops().unwrap().unwrap().into_iter().filter(|o| !matches!(o.op, GraftOp::Recount { .. })).map(|o| o.op.path().to_owned()).collect()
    };
    let nested = |a: &str, b: &str| a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"));
    for (_, k, _) in KEIRA {
        for (_, l, _) in LETHO {
            for a in paths(effect(&book, KEIRA_Q, k)) {
                for b in paths(effect(&book, LETHO_Q, l)) {
                    assert!(!nested(&a, &b), "{k} / {l} overlap: {a} vs {b}");
                }
            }
        }
    }
}

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

fn roots() -> Vec<PathBuf> {
    let mut r: Vec<PathBuf> = std::env::var_os("W3AL_FIXTURES").map(PathBuf::from).into_iter().collect();
    if let Some(d) = std::env::var_os("W3AL_DONORS") {
        r.extend(std::env::split_paths(&d));
    }
    r
}

/// First (sorted) `<stem>.sav` under the roots.
fn find_save(stem: &str) -> Option<PathBuf> {
    let mut files = Vec::new();
    for r in roots() {
        saves(&r, &mut files);
    }
    files.sort();
    let want = format!("{stem}.sav");
    files.into_iter().find(|f| f.file_name().is_some_and(|n| n.to_string_lossy() == want))
}

/// Template, the folders of the five donors (deduplicated), or `None` (skip) when one is missing.
fn inputs() -> Option<(PathBuf, Vec<PathBuf>)> {
    if roots().is_empty() {
        eprintln!("skip: W3AL_FIXTURES / W3AL_DONORS not set");
        return None;
    }
    let mut dirs = BTreeSet::new();
    for stem in KEIRA.iter().chain(&LETHO).map(|(_, _, d)| *d) {
        let Some(p) = find_save(stem) else {
            eprintln!("skip: {stem}.sav not found under W3AL_FIXTURES / W3AL_DONORS");
            return None;
        };
        dirs.insert(p.parent().unwrap().to_owned());
    }
    let Some(t) = find_save(TEMPLATE) else {
        eprintln!("skip: template {TEMPLATE}.sav not found");
        return None;
    };
    Some((t, dirs.into_iter().collect()))
}

fn load(p: &Path) -> Document {
    let c = Container::read(&std::fs::read(p).unwrap()).unwrap();
    Document::parse(&c.payload, c.data_start).unwrap()
}

fn run_for(keira: &str, letho: &str, donors: &[PathBuf]) -> Run {
    let mut run = Run { allow_name_remap: true, donors: donors.iter().map(|d| d.display().to_string()).collect(), ..Run::default() };
    for (q, c, o) in [(KEIRA_Q, "keira", keira), (LETHO_Q, "letho", letho)] {
        run.quests.insert(q.into(), QuestEntry { status: Status::Completed, answers: [(c.to_string(), o.to_string())].into() });
    }
    run
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("w3al-velen-w2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn guid_path(steps: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<journal::Guid> {
    steps.into_iter().map(|s| journal::parse_guid(s.as_ref()).unwrap()).collect()
}

/// Our own checks of an exported save, independent of export's verification: it re-parses and serializes
/// back, `check-save` has no errors, the template's names are a prefix of ours, top-level nodes outside the
/// touched blocks equal the template's, every copied node equals its donor's (resolved fingerprint, so
/// remapped copies count), every removed node is gone, and the journal and facts hold what was requested
/// (and what the donor holds).
fn verify_export(sav: &Path, template: &Document, effects: &[(&Effect, &Path)]) {
    let bytes = std::fs::read(sav).unwrap();
    let info = w3save::inspect(&bytes).unwrap();
    let errors: Vec<_> = info.issues.iter().filter(|i| i.severity == Severity::Error).collect();
    assert!(errors.is_empty(), "check-save: {errors:?}");
    let c = Container::read(&bytes).unwrap();
    let ours = Document::parse(&c.payload, c.data_start).unwrap();
    assert!(ours.to_bytes() == c.payload, "typed re-parse does not serialize back");
    assert!(ours.table() == ours.map.variables, "variable table");
    assert!(graph::names_prefix(template, &ours), "template names are not a prefix");

    let (tt, to): (Vec<_>, Vec<_>) = (template.top_nodes().collect(), ours.top_nodes().collect());
    assert_eq!(tt.len(), to.len());
    for (a, b) in tt.iter().zip(&to) {
        let name = graph::node_name(template, a);
        // `MANU` (the NM table) grows by appended names: checked by `names_prefix` above.
        if !TOUCHED.contains(&name.as_str()) && name != "facts" && name != "MANU" {
            assert!(a.to_bytes() == b.to_bytes(), "top-level {name} changed");
        }
    }

    let (tj, oj) = (journal::read(template).unwrap(), journal::read(&ours).unwrap());
    let facts = ours.facts_table().unwrap();
    for (e, donor_path) in effects {
        let donor = load(donor_path);
        for o in e.graph_ops().unwrap().unwrap() {
            let p = o.op.path();
            match &o.op {
                GraftOp::Replace { .. } | GraftOp::Insert { .. } => {
                    let fp = |d: &Document| graph::resolved_fingerprint(d, graph::node_at(d, &graph::locate(d, p).unwrap().unwrap())).unwrap();
                    assert_eq!(fp(&ours), fp(&donor), "copied node differs from the donor: {p}");
                }
                GraftOp::Remove { .. } | GraftOp::SwapRemove { .. } => assert!(graph::locate(&ours, p).unwrap().is_none(), "still there: {p}"),
                GraftOp::Recount { .. } => {}
            }
        }
        let dj = journal::read(&donor).unwrap();
        let status_of = |j: &journal::Journal, key: &[journal::Guid]| -> Vec<String> { j.find(key).into_iter().map(|i| j.entries[i].status.clone()).collect() };
        for s in &e.journal_status {
            let key = guid_path(&s.path);
            assert_eq!(status_of(&oj, &key), vec![s.to.clone()], "{:?}", s.label);
            assert_eq!(status_of(&dj, &key), vec![s.to.clone()], "donor {:?}", s.label);
        }
        for a in &e.journal_append {
            let key = guid_path(a.steps.iter().map(|s| &s.guid));
            assert!(tj.find(&key).is_empty(), "template already has {:?}", a.label);
            assert_eq!(status_of(&oj, &key), vec![a.status.clone()], "{:?}", a.label);
            assert_eq!(status_of(&dj, &key), vec![a.status.clone()], "donor {:?}", a.label);
        }
        for k in &e.journal_counters {
            let g = journal::parse_guid(&k.guid).unwrap();
            assert_eq!(oj.counters.iter().filter(|c| c.guid == g).map(|c| c.count).collect::<Vec<_>>(), vec![k.count], "counter {}", k.guid);
        }
        let df = donor.facts_table().unwrap();
        for (name, v) in &e.facts {
            assert_eq!(facts.get(name).map(|f| f.sum()), Some(*v), "fact {name}");
            assert_eq!(df.get(name).map(|f| f.sum()), Some(*v), "donor fact {name}");
        }
    }
}

/// All 3 x 2 combinations from T_keira through the real export path, each verified.
#[test]
fn all_six_w2_combinations_export_and_verify() {
    let Some((tpath, donors)) = inputs() else { return };
    let book = book();
    let template = load(&tpath);
    let out = scratch("six");
    let find = |stem: &str| find_save(stem).unwrap();
    for (i, (ko, kk, kd)) in KEIRA.iter().enumerate() {
        for (j, (lo, lk, ld)) in LETHO.iter().enumerate() {
            let run = run_for(ko, lo, &donors);
            let res = resolve(&book, &run);
            assert!(!res.has_errors(), "{:?}", res.diagnostics);
            assert_eq!(res.effects.len(), 2);
            // Distinct game-style names (one file per combination; `b`/`c` = a plausible later time).
            let stem = format!("ManualSave_568ef_7ea51000_c{}00b5", i * 2 + j + 1);
            let opts = ExportOptions { stem: Some(stem.clone()), ..ExportOptions::for_run(&run) };
            let r = export::export_with(&tpath, &out, &res, &opts).unwrap_or_else(|e| panic!("{ko} + {lo}: {e}"));
            assert_eq!(r.effects_applied, 2);
            assert!(r.notes[0].contains(kd) && !r.notes[0].contains("REMAPPED"), "{:?}", r.notes);
            assert!(r.notes[1].contains(ld) && r.notes[1].contains("REMAPPED"), "{:?}", r.notes);
            let (ke, le) = (effect(&book, KEIRA_Q, kk), effect(&book, LETHO_Q, lk));
            verify_export(&r.sav, &template, &[(ke, &find(kd)), (le, &find(ld))]);
            eprintln!("{ko} + {lo}: {} verified", r.sav.display());
        }
    }
    std::fs::remove_dir_all(&out).unwrap();
}

/// Path of q109's own thread (Keira's quest), which advances at the decision: the pre-decision trigger
/// `215f2719` (still in T_keira) leaves, `d5740aa4` and a branch block (KM / Radovid: block + thread
/// `f61db075`; killed: `c8874cb7`) arrive.
const Q109: &str = "questSystem/quest[witcher3_quest.w2quest]/questThread/questThread[7b125918]/questThread/questThread[f0b52e98]/questThread/questThread[117e4006]/questThread/questThread[3673c07f]/questThread/questThread[bb109b9f]";

/// For every Keira option (with Letho invited): q109's thread in our export equals the donor's (resolved
/// fingerprint of the whole thread), so the quest is past the decision the way the real branch save is.
#[test]
fn keira_q109_thread_matches_each_donor() {
    let Some((tpath, donors)) = inputs() else { return };
    let book = book();
    let out = scratch("q109");
    let thread = |d: &Document| -> Option<String> {
        let loc = graph::locate(d, Q109).unwrap()?;
        Some(graph::resolved_fingerprint(d, graph::node_at(d, &loc)).unwrap())
    };
    let child = |d: &Document, c: &str| graph::locate(d, &format!("{Q109}/questThread/{c}")).unwrap().is_some();
    let template = load(&tpath);
    assert!(child(&template, "questBlock[215f2719]"), "T_keira waits at the pre-decision trigger");
    for (i, (ko, _, kd)) in KEIRA.iter().enumerate() {
        let run = run_for(ko, "kaer_morhen", &donors);
        let res = resolve(&book, &run);
        let stem = format!("Q109_{i}");
        let r = export::export_with(&tpath, &out, &res, &ExportOptions { stem: Some(stem), ..ExportOptions::for_run(&run) }).unwrap();
        let (ours, donor) = (load(&r.sav), load(&find_save(kd).unwrap()));
        assert_eq!(thread(&ours), thread(&donor), "{ko}: q109 thread differs from donor {kd}");
        assert!(!child(&ours, "questBlock[215f2719]") && child(&ours, "questBlock[d5740aa4]"), "{ko}");
        let branch = if *ko == "killed" { "questBlock[c8874cb7]" } else { "questThread[f61db075]" };
        assert!(child(&ours, branch), "{ko}: {branch} missing");
    }
    std::fs::remove_dir_all(&out).unwrap();
}

/// Keira to Kaer Morhen + Letho invited is gate X1's content (X1 as rebuilt on 2026-11-05 with q109's thread
/// `bb109b9f`; the first X1, which left q109 at its pre-decision trigger, is in `gate_X\old_X1_flawed`): the
/// NM table and the four quest-state blocks are byte-identical to the gate save (built with `w3al
/// journal-set` from the same deltas), and so are the facts, including the removal of `keira_part1_dd_start`
/// (`facts_remove`, as the donors and the gate do).
#[test]
fn km_invited_reproduces_gate_x1() {
    let Some((tpath, donors)) = inputs() else { return };
    let Some(gate) = std::env::var_os("W3AL_GATES").map(|g| Path::new(&g).join(GATE_X1)).filter(|p| p.is_file()) else {
        eprintln!("skip: W3AL_GATES not set or {GATE_X1} missing");
        return;
    };
    let book = book();
    let run = run_for("kaer_morhen", "kaer_morhen", &donors);
    let res = resolve(&book, &run);
    let out = scratch("x1");
    let r = export::export_with(&tpath, &out, &res, &ExportOptions { stem: Some("X1_export".into()), ..ExportOptions::for_run(&run) }).unwrap();
    let (a, b, t) = (load(&r.sav), load(&gate), load(&tpath));
    assert_eq!(a.names, b.names, "NM name table");
    for name in TOUCHED {
        let (x, y) = (graph::top_block(&a, name).unwrap(), graph::top_block(&b, name).unwrap());
        assert!(x.to_bytes() == y.to_bytes(), "{name} differs from gate X1");
    }
    let (fa, fb, ft) = (a.facts_table().unwrap(), b.facts_table().unwrap(), t.facts_table().unwrap());
    let key = |f: &w3save::facts::Fact| (f.name.clone(), f.c1, f.records.iter().map(|r| (r.value, r.time.to_bits(), r.expiry.to_bits())).collect::<Vec<_>>());
    let ours: Vec<_> = fa.facts().iter().map(key).collect();
    let gate_facts: Vec<_> = fb.facts().iter().map(key).collect();
    assert!(ours == gate_facts, "facts differ from gate X1");
    assert!(ft.get("keira_part1_dd_start").is_some() && fa.get("keira_part1_dd_start").is_none(), "removed by facts_remove");
    std::fs::remove_dir_all(&out).unwrap();
}
