//! Edit mode on real saves (w3al_core::edit, docs/plan/story-chain.md):
//!
//! 1. `edit-plan` of a registered template reports, for every earlier decision, the answers the registry
//!    records for it (`encodes`), and a facts-only read (the same save under an unregistered name) agrees on
//!    every answer the scan can express.
//! 2. An edit (Yennefer love on pack 54, level 30, a kit item) writes a VERIFIED new save, leaves the input file
//!    byte-identical, refuses a decision whose consequence already played out and decides the next window.
//!
//! Gated: `W3AL_FIXTURES` (holding `pack_501` and `session_V1`) and the game install at `W3AL_INSTALL` or the
//! Steam default; skips (passes) when either is missing. Outputs go to a fresh temp folder and are removed; the
//! scan cache lives in the temp folder too. Nothing is written into the fixtures, the install or the game's saves.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use w3al_core::edit::{Changes, Engine, Plan};
use w3al_core::export::{ExportError, ExportOptions, DEFAULT_GAME_DIR};
use w3al_core::templates::{self, Registry};
use w3al_core::Questbook;
use w3save::{character, Container, Document};

fn fixtures() -> Option<PathBuf> {
    let f = PathBuf::from(std::env::var_os("W3AL_FIXTURES")?);
    (f.join("session_V1").is_dir() && f.join("pack_501").is_dir()).then_some(f)
}

fn install() -> Option<PathBuf> {
    let p = std::env::var_os("W3AL_INSTALL").map_or_else(|| PathBuf::from(DEFAULT_GAME_DIR), PathBuf::from);
    p.join("content").join("content0").join("bundles").is_dir().then_some(p)
}

/// One engine for the whole test binary (loading the graphs and scanning is the slow part).
fn engine() -> Option<(&'static Engine, PathBuf)> {
    static E: OnceLock<Option<(Engine, PathBuf)>> = OnceLock::new();
    let e = E.get_or_init(|| {
        let fx = fixtures()?;
        let game = install()?;
        let qb = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../questbook");
        let book = Questbook::load_dir(&qb).unwrap();
        let reg = Registry::load_for_questbook(&qb).unwrap();
        let dirs = vec![fx.join("pack_501"), fx.join("session_V1")];
        let donors = vec![fx.join("session_V1"), fx.join("session_V2")];
        let opts = ExportOptions { game_dir: Some(game), allow_name_remap: true, donor_dirs: donors, ..ExportOptions::default() };
        let cache = std::env::temp_dir().join("w3al-edit-test-scan-cache.json");
        let engine = Engine::load(book, reg, dirs, opts, Some(&cache), &|m| eprintln!("... {m}")).unwrap();
        Some((engine, fx))
    });
    e.as_ref().map(|(e, f)| (e, f.clone()))
}

fn template_path(fx: &Path, stem: &str) -> Option<PathBuf> {
    templates::find_file(&[fx.join("pack_501"), fx.join("session_V1")], stem)
}

fn plan_of(engine: &Engine, bytes: &[u8], stem: &str) -> Plan {
    engine.plan(bytes, stem, false).unwrap_or_else(|e| panic!("{stem}: {e}"))
}

/// Answers the scan cannot express (hand-set in the registry): not compared against a facts-only read.
const HAND_SET: &[(&str, &str)] = &[("a_favor_for_a_friend", "date"), ("kings_gambit", "side"), ("lord_of_undvik", "vigi")];

#[test]
fn plan_reports_the_registry_answers() {
    let Some((engine, fx)) = engine() else { return };
    let mut checked = 0;
    let mut facts_checked = 0;
    for t in &engine.registry.templates {
        let Some(path) = template_path(&fx, &t.stem) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        // (a) the template under its own name: the registry's answers, all of them.
        let plan = plan_of(engine, &bytes, &t.stem);
        assert_eq!(plan.is_template.as_deref(), Some(t.id.as_str()));
        for (q, answers) in &t.encodes {
            let qv = plan.quests.iter().find(|x| &x.quest == q).unwrap_or_else(|| panic!("{q} missing"));
            assert_eq!(qv.status, "done", "{}: {q}", t.id);
            for (c, o) in answers {
                let cv = qv.choices.iter().find(|x| &x.id == c).unwrap();
                assert_eq!(cv.current.as_deref(), Some(o.as_str()), "{}: {q}.{c}", t.id);
                checked += 1;
            }
        }
        // The template's own window is the one ahead.
        assert!(plan.windows.iter().any(|w| w.id == t.id && w.state == "active"), "{}: {:?}", t.id, plan.windows.iter().map(|w| (&w.id, w.state)).collect::<Vec<_>>());
        for q in &t.quests {
            let qv = plan.quests.iter().find(|x| &x.quest == q).unwrap();
            assert_eq!(qv.status, "active", "{}: {q}", t.id);
        }
        // (b) the same save under an unregistered name: facts only. Whatever the facts tell agrees with the registry.
        let anon = plan_of(engine, &bytes, "ManualSave_unregistered_0_0");
        assert!(anon.is_template.is_none());
        for (q, answers) in &t.encodes {
            for (c, o) in answers {
                if HAND_SET.contains(&(q.as_str(), c.as_str())) {
                    continue;
                }
                let qv = anon.quests.iter().find(|x| &x.quest == q).unwrap();
                let cv = qv.choices.iter().find(|x| &x.id == c).unwrap();
                match &cv.current {
                    Some(f) => {
                        assert_eq!(f, o, "{}: facts say {q}.{c} = {f}, the registry says {o}", t.id);
                        facts_checked += 1;
                    }
                    None => eprintln!("{}: {q}.{c} not read from the facts ({})", t.id, qv.note),
                }
            }
        }
    }
    eprintln!("{checked} registry answers reproduced, {facts_checked} of them from the facts alone");
    assert!(checked >= 20 && facts_checked >= 15, "{checked} / {facts_checked}");
}

fn answer(plan: &Plan, q: &str, c: &str) -> Option<String> {
    plan.quests.iter().find(|x| x.quest == q)?.choices.iter().find(|x| x.id == c)?.current.clone()
}

fn load(path: &Path) -> Document {
    let c = Container::read(&std::fs::read(path).unwrap()).unwrap();
    Document::parse(&c.payload, c.data_start).unwrap()
}

fn fact(doc: &Document, name: &str) -> bool {
    doc.facts_table().unwrap().get(name).is_some_and(|f| !f.records.is_empty())
}

#[test]
fn an_edit_writes_a_verified_new_save_and_leaves_the_input_alone() {
    let Some((engine, fx)) = engine() else { return };
    let t = engine.registry.get("w9_skellige_act3").unwrap();
    let Some(src) = template_path(&fx, &t.stem) else { return };
    let before = std::fs::read(&src).unwrap();
    let out = std::env::temp_dir().join(format!("w3al-edit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);

    // What the plan offers on pack 54: the romance and the lodge can be rewritten, Junior and the allies cannot.
    let plan = plan_of(engine, &before, &t.stem);
    let mode = |q: &str, c: &str| plan.quests.iter().find(|x| x.quest == q).unwrap().choices.iter().find(|x| x.id == c).unwrap().mode;
    assert_eq!(mode("the_last_wish", "yennefer"), "rewrite");
    assert_eq!(mode("final_preparations", "lodge"), "rewrite");
    assert_eq!(mode("get_junior", "junior"), "readonly");
    assert_eq!(mode("brothers_in_arms", "zoltan"), "readonly");
    assert_eq!(mode("child_of_the_elder_blood", "grave"), "synth", "the window ahead builds on its own template");
    assert_eq!(answer(&plan, "the_last_wish", "yennefer").as_deref(), Some("friends"));

    // Yennefer love, level 30, a single kit item.
    let mut ch = Changes { level: Some(30), items: vec!["Swallow 1".into()], ..Changes::default() };
    ch.answers.insert("the_last_wish".into(), [("yennefer".to_owned(), "love".to_owned())].into());
    let r = engine.apply(&src, &out, &ch, None).unwrap();
    println!("{:#?}", r.notes);
    assert!(r.sav.is_file() && r.sav != src);
    assert!(r.rewritten.contains(&"the_last_wish".to_string()), "{:?}", r.rewritten);
    assert!(r.verified.starts_with("VERIFIED"));
    assert!(!r.sidecars.is_empty(), "thumbnail and json are copied");
    // The input is byte-identical.
    assert_eq!(std::fs::read(&src).unwrap(), before, "the opened save must never change");
    // The new save: the decision rewritten, the level set, a clean save.
    let doc = load(&r.sav);
    assert!(fact(&doc, "sq202_yen_girlfriend") && !fact(&doc, "sq202_yen_just_friends"), "Yennefer love in the facts");
    assert_eq!(character::read(&doc).unwrap().level.level, 30);
    assert!(w3save::inspect(&std::fs::read(&r.sav).unwrap()).unwrap().issues.iter().all(|i| i.severity != w3save::Severity::Error));
    // Planning the written save reads the new answer back (facts only: it is no registered template).
    let again = plan_of(engine, &std::fs::read(&r.sav).unwrap(), r.sav.file_stem().unwrap().to_str().unwrap());
    assert_eq!(answer(&again, "the_last_wish", "yennefer").as_deref(), Some("love"), "{:?}", again.quests.iter().find(|q| q.quest == "the_last_wish"));
    // A second write never overwrites the first.
    let r2 = engine.apply(&src, &out, &ch, None).unwrap();
    assert_ne!(r.sav, r2.sav);

    // Refused: a consequence that already played out, an unknown quest, nothing at all.
    let mut bad = Changes::default();
    bad.answers.insert("get_junior".into(), [("junior".to_owned(), "spare".to_owned())].into());
    match engine.apply(&src, &out, &bad, None) {
        Err(ExportError::Effect(m)) => assert!(m.contains("Final Preparations"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(engine.apply(&src, &out, &Changes::default(), None), Err(ExportError::Invalid(_))));

    // The window ahead is decided with its own effects.
    let mut next = Changes::default();
    next.answers.insert("child_of_the_elder_blood".into(), [("necklace".to_owned(), "necklace".to_owned()), ("grave".to_owned(), "skip".to_owned())].into());
    let r3 = engine.apply(&src, &out, &next, None).unwrap();
    assert_eq!(r3.applied, ["child_of_the_elder_blood"]);
    assert!(fact(&load(&r3.sav), "q210_gave_lara_necklace"), "the necklace decision reaches the facts");
    assert!(!r3.notes.is_empty());
    assert_eq!(std::fs::read(&src).unwrap(), before);
    std::fs::remove_dir_all(&out).unwrap();
}
