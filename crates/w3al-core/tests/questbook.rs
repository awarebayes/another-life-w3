use std::path::PathBuf;

use w3al_core::run::{QuestEntry, Status};
use w3al_core::{resolve, validate, Questbook, Run};

fn book() -> Questbook {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../questbook");
    Questbook::load_dir(&dir).expect("questbook loads")
}

/// A three-quest fixture with a demo choice and placeholder effects, for the resolve/export rules
/// (the shipped questbook no longer has placeholders to exercise them).
fn demo_book() -> Questbook {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/demo_questbook");
    Questbook::load_dir(&dir).expect("demo questbook loads")
}

fn entry(status: Status, answers: &[(&str, &str)]) -> QuestEntry {
    QuestEntry {
        status,
        answers: answers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    }
}

#[test]
fn shipped_questbook_lints_clean_of_errors() {
    let book = book();
    assert!(book.quests.len() >= 5);
    let errors: Vec<_> = validate::lint(&book).into_iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn quests_are_chronological() {
    let book = book();
    let keys: Vec<_> = book.quests.iter().map(|q| (q.act, q.order)).collect();
    assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
    // The hand-checked quests keep their relative order as content grows around them.
    let ids: Vec<_> = book.quests.iter().map(|q| q.id.as_str()).collect();
    let core = ["lilac_and_gooseberries", "beast_of_white_orchard", "family_matters", "for_the_advancement_of_learning", "ghosts_of_the_past"];
    let pos: Vec<_> = core.iter().map(|c| ids.iter().position(|i| i == c).unwrap_or_else(|| panic!("{c} missing"))).collect();
    assert!(pos.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
}

#[test]
fn completed_applies_chosen_option_then_done_phase() {
    let book = demo_book();
    let mut run = Run::default();
    run.quests.insert("lilac_and_gooseberries".into(), entry(Status::Completed, &[]));
    run.quests.insert("beast_of_white_orchard".into(), entry(Status::Completed, &[("demo_reward", "waive_fee")]));
    run.stop_after = Some("beast_of_white_orchard".into());

    let res = resolve(&book, &run);
    assert!(!res.has_errors(), "{:?}", res.diagnostics);
    let origins: Vec<_> = res.effects.iter().map(|e| format!("{}/{}", e.quest, e.origin)).collect();
    assert_eq!(
        origins,
        [
            "lilac_and_gooseberries/done",
            "beast_of_white_orchard/demo_reward:waive_fee",
            "beast_of_white_orchard/done"
        ]
    );
}

#[test]
fn unanswered_choice_uses_default() {
    let book = demo_book();
    let mut run = Run::default();
    for id in ["lilac_and_gooseberries", "beast_of_white_orchard"] {
        run.quests.insert(id.into(), entry(Status::Completed, &[]));
    }
    let res = resolve(&book, &run);
    let beast = res.effects.iter().find(|e| e.origin.starts_with("demo_reward")).unwrap();
    assert_eq!(beast.origin, "demo_reward:take_coin");
}

#[test]
fn skipping_a_main_quest_locks_later_main_quests() {
    let book = demo_book();
    let mut run = Run::default();
    // lilac skipped (absent); the rest claim to be completed.
    run.quests.insert("beast_of_white_orchard".into(), entry(Status::Completed, &[]));
    run.quests.insert("family_matters".into(), entry(Status::Completed, &[]));
    let res = resolve(&book, &run);
    assert!(res.effects.is_empty());
    assert_eq!(res.blocked, ["beast_of_white_orchard", "family_matters"]);
}

#[test]
fn stop_after_truncates_the_walk() {
    let book = demo_book();
    let mut run = Run::default();
    for id in ["lilac_and_gooseberries", "beast_of_white_orchard", "family_matters"] {
        run.quests.insert(id.into(), entry(Status::Completed, &[]));
    }
    run.stop_after = Some("lilac_and_gooseberries".into());
    let res = resolve(&book, &run);
    assert_eq!(res.completed, ["lilac_and_gooseberries"]);
}

#[test]
fn bad_answer_is_an_error() {
    let book = demo_book();
    let mut run = Run::default();
    for id in ["lilac_and_gooseberries", "beast_of_white_orchard"] {
        run.quests.insert(id.into(), entry(Status::Completed, &[("demo_reward", "nope")]));
    }
    assert!(resolve(&book, &run).has_errors());
}

#[test]
fn export_refuses_the_sample_questbook_placeholders() {
    // The sample questbook is all `PLACEHOLDER_` scaffolding: exporting it must fail loudly,
    // never write a save with invented fact names (or one that ignores the player's choices).
    use w3al_core::export::{build_save, ExportError};
    use w3save::Container;

    let book = demo_book();
    let mut run = Run::default();
    run.quests.insert("lilac_and_gooseberries".into(), entry(Status::Completed, &[]));
    let res = resolve(&book, &run);

    // The refusal comes before the template is even read, so a junk template is fine here.
    let template = Container::from_payload(vec![0u8; 64]).write().unwrap();
    assert!(matches!(build_save(&template, &res), Err(ExportError::Placeholder(_))));
}

#[test]
fn export_without_effects_writes_a_new_save_set_beside_sidecars() {
    use w3al_core::export;
    use w3save::payload::testutil::synthetic;
    use w3save::Container;

    let dir = std::env::temp_dir().join(format!("w3al-export-test-{}", std::process::id()));
    let out = dir.join("out");
    std::fs::create_dir_all(&dir).unwrap();

    let payload = synthetic(3084);
    let template = dir.join("ManualSave_1_2_3.sav");
    std::fs::write(&template, Container::from_payload(payload.clone()).write().unwrap()).unwrap();
    std::fs::write(template.with_extension("png"), b"thumb").unwrap();

    // A run with nothing completed resolves to no effects: the exported payload must
    // equal the template's (the Phase 2 exit check, on a synthetic template).
    let res = resolve(&book(), &Run::default());
    assert!(res.effects.is_empty());
    let r = export::export(&template, &out, &res).unwrap();

    assert!(r.sav.is_file());
    assert_ne!(r.sav, template, "must be a new file");
    assert_eq!(Container::read(&std::fs::read(&r.sav).unwrap()).unwrap().payload, payload);
    assert_eq!(r.sidecars, [r.sav.with_extension("png")]);
    assert_eq!(std::fs::read(&r.sidecars[0]).unwrap(), b"thumb");
    // The template itself is untouched.
    assert!(template.is_file());

    std::fs::remove_dir_all(&dir).unwrap();
}
