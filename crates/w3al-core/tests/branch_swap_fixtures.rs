//! Fixture-gated (skipped when `W3AL_FIXTURES` is unset or lacks the session V1 saves): build "Keira to
//! Radovid" from the template T_keira, the "Keira to Kaer Morhen" donor and an output swap read from
//! the quest graph (docs/findings/branch-synthesis.md), and compare it with the real Radovid donor.
//!
//! The swap values are identifiers from the 5.01 quest graph (`q109_wrapup.w2phase` scene block
//! `d24f649d`, `character_journal_entries.w2phase` phase `a536c75d`), not copied from the Radovid save.
//! The Radovid save is only the reference the result is compared with.
use std::path::{Path, PathBuf};

use w3al_core::branch_swap::{self, FactSwap, GuidSwap, OutputSwap, PathSwap};
use w3al_core::graph_effect::GuardedOp;
use w3save::graph::{self, GraftOp, NameMode};
use w3save::journal;
use w3save::{Container, Document, Edits};

const TK: &str = "ManualSave_568ef_7ea51000_923cb5"; // T_keira, before the decision
const KM: &str = "ManualSave_568ef_7ea51000_94abd5"; // donor: Keira to Kaer Morhen
const KR: &str = "ManualSave_568ef_7ea51000_97cdc9"; // reference: Keira to Radovid (played)

const Q: &str = "questSystem/quest[witcher3_quest.w2quest]/questThread/questThread[7b125918]/questThread";

fn keira_scope() -> String {
    format!("{Q}/questThread[988006f0]/questThread/questThread[37028f15]/questThread/questThread[143c1d62]/questThread/questThread[a536c75d]")
}

fn q109_scope() -> String {
    format!("{Q}/questThread[f0b52e98]/questThread/questThread[117e4006]/questThread/questThread[3673c07f]/questThread/questThread[bb109b9f]")
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

fn load(stem: &str) -> Option<Document> {
    let mut files = Vec::new();
    saves(Path::new(&std::env::var_os("W3AL_FIXTURES")?), &mut files);
    files.sort();
    let p = files.into_iter().find(|f| f.file_stem().and_then(|s| s.to_str()) == Some(stem))?;
    let c = Container::read(&std::fs::read(p).ok()?).ok()?;
    Document::parse(&c.payload, c.data_start).ok()
}

fn g(s: &str) -> String {
    s.to_owned()
}

/// The swap, as read from the graph (see the module doc and docs/findings/branch-synthesis.md).
fn keira_swap() -> OutputSwap {
    let qa = ["419f978ff624ed49a444903714632529", "ddd5713ddef70240beecc5f8d026e213", "5a77286675444c4caf1c78015fa765a5"];
    let ca = ["8efa62e10a415b499608256195453836", "80e826de895d434d903c39946c0246b3"];
    let path = |pre: &[&str], last: &str| pre.iter().map(|x| g(x)).chain([g(last)]).collect::<Vec<_>>();
    OutputSwap {
        source: Some("q109_wrapup d24f649d: Keira to KM -> Keira to Radovid".into()),
        block: vec![GuidSwap { from: g("f5290e77e94cfd4ca216b846c93d81ac"), to: g("c133524a87776c429ddf03180120419a"), note: None }],
        journal: vec![
            PathSwap { from: path(&qa, "7801de377aab6a4381fee5b713690733"), to: path(&qa, "fdcc61685ad3364fb611b82ed5a6f23d"), note: None },
            PathSwap { from: path(&ca, "efa7c8fb78127441aa2bb4058e749251"), to: path(&ca, "aed9149df836eb4e88a7fc4c8c530276"), note: None },
        ],
        fact: vec![FactSwap { from: g("q109_keira_to_km"), to: g("q109_keira_to_radovid"), note: None }],
    }
}

fn subtree(d: &Document, path: &str) -> Option<String> {
    graph::locate(d, path).ok().flatten().map(|l| graph::fingerprint(d, graph::node_at(d, &l)))
}

#[test]
fn keira_to_radovid_from_the_kaer_morhen_donor_equals_the_played_branch() {
    let (Some(tk), Some(km), Some(kr)) = (load(TK), load(KM), load(KR)) else {
        eprintln!("W3AL_FIXTURES not set or without the session V1 Keira saves; skipping");
        return;
    };
    let scopes = [keira_scope(), q109_scope()];

    // The KM delta in Keira's two scopes (character journal phase, q109's own thread).
    let changes = graph::diff_top(&tk, &km, "questSystem").unwrap().unwrap();
    let ops: Vec<GuardedOp> =
        graph::ops_from_diff(&changes, &scopes).into_iter().map(|op| GuardedOp { op, before: None, after: None }).collect();
    assert!(ops.iter().any(|o| o.op.path().ends_with("questBlock[f5290e77]")), "KM delta lacks Keira's KM wait");
    let edit = journal::diff(&journal::read(&tk).unwrap(), &journal::read(&km).unwrap()).to_edit().unwrap();
    let facts = vec![("q109_confronted_keira".to_owned(), 1), ("q109_keira_to_km".to_owned(), 1)];

    let s = keira_swap().apply(&ops, &edit, &facts).unwrap();
    assert_eq!(s.rekeys.len(), 1);
    let donor = branch_swap::synth_donor(&km, &s.rekeys).unwrap();
    let mut out = tk.clone();
    let plain: Vec<GraftOp> = s.ops.iter().map(|o| o.op.clone()).collect();
    graph::transplant_with(&mut out, &donor, &plain, NameMode::Prefix).unwrap();
    journal::apply(&mut out, &s.journal).unwrap();
    out.apply(&Edits { facts: s.facts.clone(), ..Default::default() }).unwrap();

    // Both scopes equal the played Radovid branch byte for byte (names resolved).
    for p in &scopes {
        assert!(subtree(&out, p).is_some(), "{p} missing");
        assert_eq!(subtree(&out, p), subtree(&kr, p), "{p} differs from the Radovid donor");
    }
    // ... while the KM donor's Keira scope does not (the swap did something).
    assert_ne!(subtree(&km, &keira_scope()), subtree(&kr, &keira_scope()));
    // The quest log equals the Radovid donor's (entries, order, statuses, counters, tracking).
    let (jo, jr) = (journal::read(&out).unwrap(), journal::read(&kr).unwrap());
    assert!(journal::diff(&jo, &jr).is_empty(), "journal differs from the Radovid donor");
    // Decision facts: Radovid set, KM absent, as in the donor.
    let (fo, fr) = (out.facts_table().unwrap(), kr.facts_table().unwrap());
    for n in ["q109_keira_to_radovid", "q109_keira_to_km", "q109_confronted_keira"] {
        assert_eq!(fo.get(n).map(|f| f.sum()), fr.get(n).map(|f| f.sum()), "fact {n}");
    }
}
