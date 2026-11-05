//! `[effects.<key>.synth]` through the product path (questbook -> `resolve` -> `export`): Keira to Kaer
//! Morhen and Keira to Radovid synthesized from the game's quest graphs on T_keira, with no donor, compared
//! with the saves a human really played from the same template (docs/plan/no-human-pipeline.md).
//!
//! Gated: `W3AL_FIXTURES` (e.g. `C:\w3-fixtures`, holding `session_V1`) and the game install, read-only, at
//! `W3AL_INSTALL` or the Steam default (`export::DEFAULT_GAME_DIR`). Skips (passes) when either is missing.
//! Output goes to a fresh folder under the system temp dir (written with `create_new`) and is removed.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use w3al_core::export::{self, ExportOptions};
use w3al_core::questbook::Quest;
use w3al_core::run::{QuestEntry, Status};
use w3al_core::{resolve, validate, Questbook, Run};
use w3save::journal;
use w3save::{Container, Document};

const TEMPLATE: &str = "ManualSave_568ef_7ea51000_923cb5"; // T_keira
const DONOR_KM: &str = "ManualSave_568ef_7ea51000_94abd5";
const DONOR_RADOVID: &str = "ManualSave_568ef_7ea51000_97cdc9";

/// A test questbook (not content): one quest, one choice, two synth effects with exactly the
/// `synth-apply` arguments of docs/plan/no-human-pipeline.md.
const QUEST: &str = r#"
id = "keira_synth_test"
title = "Keira (synth test)"
act = 2
order = 2300
kind = "secondary"
region = "velen"
level = 11
summary = "Test quest."

[[choices]]
id = "keira"
prompt = "Where does Keira go?"
default = "kaer_morhen"
options = [
  { id = "kaer_morhen", text = "Kaer Morhen", effects = "keira_km" },
  { id = "radovid", text = "Radovid", effects = "keira_radovid" },
  { id = "km_then_fact", text = "Kaer Morhen, then a fact edit", effects = "keira_km_then_fact" },
]

[effects.keira_km]
source = "quest graph q109 (w3al synth-walk on T_keira)"
[effects.keira_km.synth]
start = ["215f2719"]
through = true
pick = ["d24f649d=Keira to KM"]
stop = ["4c2a9b2d"]

[effects.keira_radovid]
source = "quest graph q109 (w3al synth-walk on T_keira)"
[effects.keira_radovid.synth]
start = ["215f2719"]
through = true
pick = ["d24f649d=Keira to Radovid"]
stop = ["4c2a9b2d"]

# Explicit facts apply after the walk: the walk sets q109_confronted_keira, this removes it again.
[effects.keira_km_then_fact]
source = "test: synth then facts_remove"
facts_remove = ["q109_confronted_keira"]
[effects.keira_km_then_fact.synth]
start = ["215f2719"]
through = true
pick = ["d24f649d=Keira to KM"]
stop = ["4c2a9b2d"]
"#;

fn book() -> Questbook {
    let q: Quest = toml::from_str(QUEST).unwrap();
    Questbook::from_quests(vec![q])
}

fn run(option: &str) -> Run {
    let mut run = Run::default();
    run.quests.insert(
        "keira_synth_test".into(),
        QuestEntry { status: Status::Completed, answers: [("keira".to_owned(), option.to_owned())].into() },
    );
    run
}

fn setup() -> Option<(PathBuf, PathBuf)> {
    let v1 = PathBuf::from(std::env::var("W3AL_FIXTURES").ok()?).join("session_V1");
    let install = std::env::var("W3AL_INSTALL").map_or_else(|_| PathBuf::from(export::DEFAULT_GAME_DIR), PathBuf::from);
    if !v1.join(format!("{TEMPLATE}.sav")).is_file() || !install.join("content").is_dir() {
        eprintln!("skipped: needs W3AL_FIXTURES with session_V1 and the game install");
        return None;
    }
    Some((v1, install))
}

fn load(path: &Path) -> Document {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let c = Container::read(&bytes).unwrap();
    Document::parse(&c.payload, c.data_start).unwrap()
}

fn fact_sums(doc: &Document) -> BTreeMap<String, i64> {
    doc.facts_table().unwrap().facts().iter().map(|f| (f.name.clone(), f.sum())).collect()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("w3al-synth-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_test_questbook_lints_clean() {
    let errors: Vec<_> = validate::lint(&book()).into_iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn keira_km_and_radovid_synthesized_match_the_played_donors() {
    let Some((v1, install)) = setup() else { return };
    let template_path = v1.join(format!("{TEMPLATE}.sav"));
    let template = load(&template_path);
    let tfacts = fact_sums(&template);
    let out = scratch("keira");
    let book = book();
    for (option, donor, decision) in [("kaer_morhen", DONOR_KM, "q109_keira_to_km"), ("radovid", DONOR_RADOVID, "q109_keira_to_radovid")] {
        let res = resolve(&book, &run(option));
        assert!(!res.has_errors(), "{:?}", res.diagnostics);
        // No donor folder at all: the effect needs none.
        let opts = ExportOptions { stem: Some(format!("synth_{option}")), game_dir: Some(install.clone()), ..Default::default() };
        let r = export::export_with(&template_path, &out, &res, &opts).unwrap_or_else(|e| panic!("{option}: {e}"));
        assert!(r.notes[0].contains("synth"), "{:?}", r.notes);
        let got = load(&r.sav);
        let donor = load(&v1.join(format!("{donor}.sav")));

        // Journal: 0 differences to the played save.
        let d = journal::diff(&journal::read(&got).unwrap(), &journal::read(&donor).unwrap());
        assert!(d.changed.is_empty() && d.added.is_empty() && d.removed.is_empty(), "{option}: {d:?}");

        // Decision facts: every fact the export changed holds the donor's value, and the decision is there.
        let (gfacts, dfacts) = (fact_sums(&got), fact_sums(&donor));
        let changed: Vec<&String> =
            gfacts.keys().chain(tfacts.keys()).filter(|k| gfacts.get(*k) != tfacts.get(*k)).collect();
        assert!(changed.len() >= 4, "{option}: {changed:?}");
        for f in changed {
            assert_eq!(gfacts.get(f), dfacts.get(f), "{option}: fact {f}");
        }
        assert_eq!(gfacts.get(decision), Some(&1), "{option}");
        assert_eq!(gfacts.get("keira_part1_dd_start"), None, "{option}: removed by the walk");
    }
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn explicit_facts_apply_after_the_walk_with_graphs_loaded_once() {
    let Some((v1, install)) = setup() else { return };
    let template_bytes = std::fs::read(v1.join(format!("{TEMPLATE}.sav"))).unwrap();
    let opts = ExportOptions { game_dir: Some(install), ..Default::default() };
    let book = book();
    // Graphs loaded once, shared by two builds.
    let res = resolve(&book, &run("km_then_fact"));
    let graphs = export::load_graphs(&res, &opts).unwrap().expect("a synth run loads the graphs");
    let built = export::build_with_sources(&template_bytes, &res, &opts, &Default::default(), Some(&graphs)).unwrap();
    let c = Container::read(&built.bytes).unwrap();
    let f = fact_sums(&Document::parse(&c.payload, c.data_start).unwrap());
    assert_eq!(f.get("q109_keira_to_km"), Some(&1));
    assert_eq!(f.get("q109_confronted_keira"), None, "facts_remove runs after the walk");

    let res = resolve(&book, &run("radovid"));
    assert!(export::build_with_sources(&template_bytes, &res, &opts, &Default::default(), Some(&graphs)).is_ok());

    // A start the save is not waiting at is refused (the KM save is past the decision).
    let km = export::build_with_sources(&template_bytes, &resolve(&book, &run("kaer_morhen")), &opts, &Default::default(), Some(&graphs))
        .unwrap()
        .bytes;
    let err = export::build_with_sources(&km, &resolve(&book, &run("radovid")), &opts, &Default::default(), Some(&graphs)).unwrap_err();
    assert!(err.to_string().contains("not waiting in the save"), "{err}");
}
