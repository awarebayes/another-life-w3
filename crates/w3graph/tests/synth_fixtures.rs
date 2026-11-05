//! Donor-free synthesis checked against saves a human really played (docs/plan/no-human-pipeline.md).
//! Needs the game's quest graphs (with the side resources: scenes, spawn sets, journals) and the
//! session V1 / V2 fixtures:
//!   W3AL_GRAPHS   = folder(s) of extracted .w2phase/.w2quest/.w2scene/.w2comm/.journal files, `;`-separated
//!                   (or W3AL_INSTALL = the game folder, read-only)
//!   W3AL_FIXTURES = C:\w3-fixtures (holds session_V1, session_V2, pack_501)
//! Skips (passes) when either is unset.
use std::collections::BTreeSet;
use std::path::PathBuf;

use w3graph::community;
use w3graph::frontier::{self, Frontier};
use w3graph::quest::{guid_hex, Library};
use w3graph::synth::{self, WalkSpec};
use w3graph::walk::Walk;
use w3save::graph;
use w3save::journal;
use w3save::{Container, Document};

fn load(path: &PathBuf) -> Document {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let c = Container::read(&bytes).unwrap();
    Document::parse(&c.payload, c.data_start).unwrap()
}

fn setup() -> Option<(Library, PathBuf)> {
    let fixtures = std::env::var("W3AL_FIXTURES").ok()?;
    let lib = if let Ok(dirs) = std::env::var("W3AL_GRAPHS") {
        let dirs: Vec<PathBuf> = dirs.split(';').filter(|s| !s.is_empty()).map(PathBuf::from).collect();
        Library::load_dirs(&dirs).unwrap()
    } else if let Ok(i) = std::env::var("W3AL_INSTALL") {
        Library::load_install(&PathBuf::from(i)).unwrap()
    } else {
        return None;
    };
    assert!(lib.blocks.len() > 100_000, "the quest graphs look incomplete: {} blocks", lib.blocks.len());
    Some((lib, PathBuf::from(fixtures)))
}

/// What `w3al synth-walk --through` / the questbook's `[effects.*.synth]` do: `synth::plan` with the
/// start blocks fired through their saved input, picks, scene picks, stops, then the template's fact
/// listeners. Refuses unresolved points.
fn walk(lib: &Library, template: &Document, tf: &Frontier, starts: &[&str], picks: &[&str], scene_picks: &[&str], stops: &[&str]) -> Walk {
    let v = |x: &[&str]| x.iter().map(|s| (*s).to_owned()).collect();
    let spec = WalkSpec { starts: v(starts), picks: v(picks), scene_picks: v(scene_picks), stops: v(stops), through: true, ..WalkSpec::default() };
    let (w, notes) = synth::plan(lib, template, tf, &spec).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    let unresolved = synth::unresolved(lib, &w);
    assert!(unresolved.is_empty(), "{unresolved:#?}");
    w
}

/// Changes between two saves' `questSystem` under the given scope keys (`questThread[xxxxxxxx]`).
fn scope_changes(a: &Document, b: &Document, scopes: &[&str]) -> Vec<String> {
    graph::diff_top(a, b, "questSystem")
        .unwrap()
        .unwrap_or_default()
        .iter()
        .filter(|c| scopes.iter().any(|s| c.path().contains(&format!("questThread[{s}]"))))
        .map(|c| c.path().to_owned())
        .collect()
}

/// The spawn-set differences between the synthesized save and the donor in the sets the walk
/// touched, minus the stubs the game itself creates on load for an active set / phase (HYPOTHESIS
/// gate K3: stub creation is left to the game, so a donor stub with no counterpart is expected).
fn community_gaps(synth: &Document, donor: &Document, w: &Walk) -> Vec<String> {
    let sets = community::touched_sets(&synth::spawn_actions(w));
    community::compare(synth, donor, &sets).into_iter().filter(|l| !l.starts_with("stub only in B")).collect()
}

#[test]
fn keira_to_kaer_morhen_is_reproduced_from_the_graph_alone() {
    let Some((lib, fx)) = setup() else { return };
    let v1 = fx.join("session_V1");
    let template = load(&v1.join("ManualSave_568ef_7ea51000_923cb5.sav"));
    let donor = load(&v1.join("ManualSave_568ef_7ea51000_94abd5.sav"));
    let tf = frontier::read(&template).unwrap();
    let w = walk(&lib, &template, &tf, &["215f2719"], &["d24f649d=Keira to KM"], &[], &["4c2a9b2d"]);
    assert_eq!(w.facts.get("q109_keira_to_km"), Some(&1));
    assert_eq!(w.removed_facts, vec!["keira_part1_dd_start".to_owned()]);

    // Prediction vs the played donor: nothing missed, nothing extra in the quest's files.
    let pred = frontier::predict(&lib, &w, &tf);
    let r = frontier::compare(&lib, &pred, &template, &donor).unwrap();
    assert_eq!((r.blocks.1, r.blocks.2), (0, 0), "{}", r.lines.join("\n"));
    assert_eq!((r.facts.1, r.facts.2), (0, 0), "{}", r.lines.join("\n"));
    assert_eq!((r.journal.1, r.journal.2), (0, 0), "{}", r.lines.join("\n"));

    // The synthesized save equals the donor in the quest's scopes, the journal (entries and objective
    // counters) and the decision facts; the spawn set took the new phase and lost the old phase's stub.
    let (synth, rep) = synth::apply(&lib, &w, &template).unwrap();
    assert_eq!(rep.journal.appended, 4);
    assert_eq!(rep.counters_added.len(), 1, "the new objective gets its counter");
    let changes = scope_changes(&synth, &donor, &["bb109b9f", "f61db075", "a536c75d", "973a4357"]);
    assert!(changes.is_empty(), "{changes:#?}");
    let d = journal::diff(&journal::read(&synth).unwrap(), &journal::read(&donor).unwrap());
    assert!(d.changed.is_empty() && d.added.is_empty() && d.removed.is_empty(), "{d:?}");
    assert!(d.counters_added.is_empty() && d.counters_changed.is_empty() && d.counters_removed.is_empty(), "counters: {d:?}");
    let sf = frontier::fact_sums(&synth).unwrap();
    let df = frontier::fact_sums(&donor).unwrap();
    for f in ["q109_keira_to_km", "q109_confronted_keira", "keira_part1_dd_stop", "keira_part1_dd_start"] {
        assert_eq!(sf.get(f), df.get(f), "{f}");
    }
    let keira = "quests/secondary_npcs/community_files/keira_metz.w2comm".to_owned();
    assert_eq!(rep.community.phases_set, vec![(keira.clone(), "q109_island_laboratory".to_owned(), "q109_island_attacking".to_owned())]);
    assert_eq!(rep.community.stubs_removed.len(), 1);
    assert!(community_gaps(&synth, &donor, &w).is_empty(), "{:?}", community::compare(&synth, &donor, &[keira]));
    // The written bytes re-read to the same tree.
    let re = Document::parse(&synth.to_bytes(), synth.base).unwrap();
    assert_eq!(re.to_bytes(), synth.to_bytes());
}

#[test]
fn keira_to_radovid_is_the_same_walk_with_another_pick() {
    let Some((lib, fx)) = setup() else { return };
    let v1 = fx.join("session_V1");
    let template = load(&v1.join("ManualSave_568ef_7ea51000_923cb5.sav"));
    let donor = load(&v1.join("ManualSave_568ef_7ea51000_97cdc9.sav"));
    let tf = frontier::read(&template).unwrap();
    let w = walk(&lib, &template, &tf, &["215f2719"], &["d24f649d=Keira to Radovid"], &[], &["4c2a9b2d"]);
    let pred = frontier::predict(&lib, &w, &tf);
    let r = frontier::compare(&lib, &pred, &template, &donor).unwrap();
    assert_eq!((r.blocks.1, r.blocks.2, r.facts.1, r.facts.2, r.journal.1, r.journal.2), (0, 0, 0, 0, 0, 0), "{}", r.lines.join("\n"));
    let (synth, rep) = synth::apply(&lib, &w, &template).unwrap();
    assert!(scope_changes(&synth, &donor, &["bb109b9f", "f61db075", "a536c75d", "973a4357"]).is_empty());
    let d = journal::diff(&journal::read(&synth).unwrap(), &journal::read(&donor).unwrap());
    assert!(d.changed.is_empty() && d.added.is_empty() && d.removed.is_empty(), "{d:?}");
    assert!(d.counters_added.is_empty() && d.counters_changed.is_empty(), "counters: {d:?}");
    assert_eq!(rep.community.phases_set.len(), 1);
    assert!(community_gaps(&synth, &donor, &w).is_empty());
}

#[test]
fn whispering_hillock_both_branches_from_the_graph() {
    let Some((lib, fx)) = setup() else { return };
    let v1 = fx.join("session_V1");
    let template = load(&v1.join("ManualSave_6255c_7ea51000_8f3325.sav"));
    let tf = frontier::read(&template).unwrap();
    // (donor, pick, stops, facts the walk must set, sets added, sets removed)
    type Case = (&'static str, &'static str, &'static [&'static str], &'static [&'static str], usize, usize);
    let cases: [Case; 2] = [
        ("ManualSave_52587_7ea51000_900fd9.sav", "q105_ritual_done_out", &["51f0f429", "0d3a7efd"], &["q105_evil_spirit_released", "q105_ritual_done"], 8, 2),
        // `q105_betray_spirit` is set inside the ritual scene file (section `#90`), not by a graph block.
        ("ManualSave_52587_7ea51000_91a5f1.sav", "q105_betrayed_out", &["1f5f6045"], &["q105_evil_spirit_destroyed", "q105_betray_spirit"], 2, 2),
    ];
    for (donor_file, pick, stops, facts, sets_added, sets_removed) in cases {
        let donor = load(&v1.join(donor_file));
        let w = walk(&lib, &template, &tf, &["c2549952", "c4d54c91"], &[&format!("ba20c454={pick}")], &[], stops);
        for f in facts {
            assert!(w.facts.get(*f).is_some_and(|v| *v > 0), "{pick}: {f}");
        }
        let pred = frontier::predict(&lib, &w, &tf);
        let r = frontier::compare(&lib, &pred, &template, &donor).unwrap();
        assert_eq!((r.blocks.1, r.blocks.2), (0, 0), "{pick}\n{}", r.lines.join("\n"));
        assert_eq!((r.journal.1, r.journal.2), (0, 0), "{pick}\n{}", r.lines.join("\n"));
        assert_eq!(r.facts.2, 0, "{pick}: extra facts\n{}", r.lines.join("\n"));
        // Facts the graph and the scenes cannot give: script side effects, play noise.
        let known_gaps: BTreeSet<&str> = ["q105_new_heart_killed", "SignToggled", "traveling_merchant_01_homestead", "traveling_merchant_01_gustfields"].into();
        let misses: Vec<&String> = r.lines.iter().filter(|l| l.starts_with("  MISS ") && !l.contains(" removed")).collect();
        for m in &misses {
            let name = m.split_whitespace().nth(1).unwrap();
            assert!(known_gaps.contains(name), "{pick}: unexpected fact miss {m}");
        }
        let (synth, rep) = synth::apply(&lib, &w, &template).unwrap();
        let d = journal::diff(&journal::read(&synth).unwrap(), &journal::read(&donor).unwrap());
        assert!(d.changed.is_empty() && d.added.is_empty() && d.removed.is_empty(), "{pick}: {d:?}");
        assert!(d.counters_added.is_empty() && d.counters_changed.is_empty(), "{pick} counters: {d:?}");
        assert_eq!(rep.counters_added.len(), 1, "{pick}");
        // The quest's own scope: every GUID the donor waits at is there (the scripted-actions tree
        // state of a block that already ran in the donor is the known difference).
        let sf = frontier::read(&synth).unwrap();
        let df = frontier::read(&donor).unwrap();
        for b in df.blocks.iter().filter(|b| b.path.contains("questThread[0edad8e4]")) {
            assert!(sf.find(&b.guid).is_some(), "{pick}: donor waits at {} which the synthesis lacks", guid_hex(&b.guid));
        }
        for b in sf.blocks.iter().filter(|b| b.path.contains("questThread[0edad8e4]")) {
            assert!(df.find(&b.guid).is_some(), "{pick}: synthesis waits at {} which the donor lacks", guid_hex(&b.guid));
        }
        // Spawn sets: the sets the walk activates / deactivates match the donor (phases and presence);
        // the evil heart's stub and the burrows NPC's stub left with their sets.
        assert_eq!((rep.community.sets_added.len(), rep.community.sets_removed.len()), (sets_added, sets_removed), "{pick}: {:?}", rep.community);
        assert_eq!(rep.community.stubs_removed.len(), 2, "{pick}");
        assert!(community_gaps(&synth, &donor, &w).is_empty(), "{pick}: {:?}", community_gaps(&synth, &donor, &w));
    }
}

/// Letho invited to Kaer Morhen (T_letho -> session V2 donor `a4841f`): the decision is a choice line
/// inside `sq102_04_07_letho_revived.w2scene` (`q402_invited_letho_to_km` is an `AddFact_S` there), so
/// the walk takes a scene pick; the headhunters are let go (`NoFight`). The quest's end phase waits at
/// an `AndBlock` with one of two inputs, as the donor does.
#[test]
fn letho_invited_to_kaer_morhen_with_the_scene_fact() {
    let Some((lib, fx)) = setup() else { return };
    let template = load(&fx.join("session_V1").join("ManualSave_6b9be_7ea51000_9bb30d.sav"));
    let donor = load(&fx.join("session_V2").join("ManualSave_52584_7ea51000_a4841f.sav"));
    let tf = frontier::read(&template).unwrap();
    let w = walk(&lib, &template, &tf, &["2da9cd80"], &["70d40ee9=NoFight"], &["ed477b93=sq102_go _to_km_letho"], &[]);
    for f in ["q402_invited_letho_to_km", "sq102_let_headhunters_go", "sq102_helped_letho", "sq102_letho_completed", "sq102_letho_lost_medallion"] {
        assert_eq!(w.facts.get(f), Some(&1), "{f}");
    }
    assert_eq!(w.facts.get("q402_told_letho_to_go_elsewhere"), None);
    assert!(w.maybe_facts.is_empty(), "{:?}", w.maybe_facts);
    let pred = frontier::predict(&lib, &w, &tf);
    let r = frontier::compare(&lib, &pred, &template, &donor).unwrap();
    assert_eq!((r.facts.1, r.facts.2), (0, 0), "{}", r.lines.join("\n"));
    assert_eq!(r.journal.2, 0, "{}", r.lines.join("\n"));

    let (synth, rep) = synth::apply(&lib, &w, &template).unwrap();
    // Journal: same entries and statuses (the donor's root entry is read, ours unread); no counters
    // (the new entries are description entries).
    let d = journal::diff(&journal::read(&synth).unwrap(), &journal::read(&donor).unwrap());
    assert!(d.added.is_empty() && d.removed.is_empty(), "{d:?}");
    assert!(d.changed.iter().all(|c| c.before.status == c.after.status), "{d:?}");
    assert!(d.counters_added.is_empty() && d.counters_changed.is_empty(), "{d:?}");
    assert!(rep.counters_added.is_empty());
    // Spawn sets: Letho's set and the Midcots sets off, stubs and entity data gone, as in the donor.
    assert_eq!(rep.community.stubs_removed.len(), 3);
    assert_eq!(rep.community.entities_removed.len(), 3);
    assert!(community_gaps(&synth, &donor, &w).is_empty(), "{:?}", community_gaps(&synth, &donor, &w));
    // The quest scope: the End phase with its And block (`ActivatedInputs` = `In 1`) equals the donor's node.
    let and_path = "questSystem/quest[witcher3_quest.w2quest]/questThread/questThread[7b125918]/questThread/questThread[f0b52e98]/questThread/questThread[117e4006]/questThread/questThread[b8f04d50]/questThread/questThread[d8b0024d]/questThread/questBlock[e9e84a64]";
    let fp = |d: &Document| graph::locate(d, and_path).unwrap().map(|l| graph::resolved_fingerprint(d, graph::node_at(d, &l)).unwrap());
    assert!(fp(&synth).is_some(), "the And block waits in the synthesized save");
    assert_eq!(fp(&synth), fp(&donor));
    let sf = frontier::read(&synth).unwrap();
    let df = frontier::read(&donor).unwrap();
    assert_eq!(sf.find(&lib.blocks[lib.resolve("e9e84a64").unwrap()].guid).map(|b| b.inputs.clone()), Some(vec!["In 1".to_owned()]));
    // Everything the donor waits at under the quest is there, except `5f88ff5b` (a fact wait the
    // `through` walk passed; the donor's player had not) and what the donor played past (`3d5ad979`).
    for b in df.blocks.iter().filter(|b| b.path.contains("questThread[b8f04d50]") && !b.path.contains("[5f88ff5b]")) {
        assert!(sf.find(&b.guid).is_some(), "donor waits at {} which the synthesis lacks", guid_hex(&b.guid));
    }
}

/// W-3: walks whose phases end with a cut-control block or an entered sub-phase inside (D16 lodge on
/// pack 51, D11 refuse on pack 35) write; the And block of pack 51 that already held two inputs takes
/// the third and fires (`4175df2e` then waits with `In 0`).
#[test]
fn ended_phases_leave_nothing_behind_on_the_act_3_and_skellige_packs() {
    let Some((lib, fx)) = setup() else { return };
    let packs = fx.join("pack_501");
    let t51 = load(&packs.join("ManualSave_4e4ce_7ea51000_3fb3aa7.sav"));
    let tf = frontier::read(&t51).unwrap();
    let a73 = tf.find(&lib.blocks[lib.resolve("a73fe883").unwrap()].guid).expect("pack 51 waits at the q310 And block");
    assert_eq!(a73.inputs, vec!["In 2".to_owned(), "In 1".to_owned()]);
    let picks = ["25e35e14=Output", "b0cef480=Out fight 1", "91f60b5d=Out fight 1", "fc64e6ac=out convinced", "d445d2b7=out no help", "cb2fe27e=Output"];
    let w = walk(&lib, &t51, &tf, &["22f9e683"], &picks, &[], &["4175df2e"]);
    assert!(w.consumed.contains(&lib.resolve("a73fe883").unwrap()), "the And block fired with its third input");
    assert_eq!(w.facts.get("q310_completed"), Some(&1));
    let (synth, rep) = synth::apply(&lib, &w, &t51).unwrap();
    let sf = frontier::read(&synth).unwrap();
    let and = sf.find(&lib.blocks[lib.resolve("4175df2e").unwrap()].guid).expect("the part_3 And block waits");
    assert_eq!(and.inputs, vec!["In 0".to_owned()]);
    for s in ["fdfe2034", "c467122b"] {
        assert!(sf.find(&lib.blocks[lib.resolve(s).unwrap()].guid).is_some(), "{s} waits for reason_of_state");
    }
    assert!(!rep.community.phases_set.is_empty());

    let t35 = load(&packs.join("ManualSave_54acc_7ea51000_3f7d081.sav"));
    let tf = frontier::read(&t35).unwrap();
    let w = walk(&lib, &t35, &tf, &["3daf7b27"], &["31f4501a=NoThanks"], &[], &[]);
    assert!(w.exited.contains(&lib.resolve("6cc212b3").unwrap()), "q206 ends through Fail");
    assert!(w.entered.iter().all(|e| !lib.scope_chain(*e, &|_| false).contains(&lib.blocks[lib.resolve("6cc212b3").unwrap()].guid)));
    let (synth, _) = synth::apply(&lib, &w, &t35).unwrap();
    let sf = frontier::read(&synth).unwrap();
    assert!(sf.find(&lib.blocks[lib.resolve("95d1d2e0").unwrap()].guid).is_none(), "no cut-control block inside the ended q206 scope");
}
