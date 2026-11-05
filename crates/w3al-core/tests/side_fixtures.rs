//! Side quests finished by the auto-complete walk (`w3graph::complete`), checked against real saves.
//!
//! 1. Validation: quests of different kinds that a player finished between two saves of the pack line
//!    (`C:\w3-fixtures\pack_501`) are auto-completed on the earlier save alone and compared with the later one
//!    (`w3graph::complete::check`): the journal root is Success in both, the quest's phase is gone in both, and
//!    the quest's journal entries and facts the player's save has are mostly there.
//! 2. Export: about thirty generated side quests (`questbook/side`) completed on the pack 43 and pack 54
//!    templates in one run export VERIFIED (export's own checks: frontier, journal, facts, untouched blocks).
//!
//! Gated: `W3AL_FIXTURES` (holding `pack_501`) and the game install at `W3AL_INSTALL` or the Steam default.
//! Skips (passes) when either is missing. Nothing is written to disk.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use w3al_core::export::{self, ExportOptions};
use w3al_core::run::Status;
use w3al_core::{paths, resolve, Questbook, Run};
use w3graph::complete;
use w3graph::quest::Library;
use w3save::{Container, Document};

const PACK: [(u32, &str); 9] = [
    (21, "ManualSave_10028e_7ea51000_3f6825b"),
    (33, "ManualSave_8c0ab_7ea51000_3f73208"),
    (35, "ManualSave_54acc_7ea51000_3f7d081"),
    (42, "ManualSave_54142_7ea51000_3f8aed5"),
    (43, "ManualSave_100840_7ea51000_3f9b30a"),
    (48, "ManualSave_86355_7ea51000_3fa5db8"),
    (51, "ManualSave_4e4ce_7ea51000_3fb3aa7"),
    (52, "ManualSave_105a8e_7ea51000_4002c60"),
    (54, "ManualSave_4dcef_7ea51000_400b253"),
];

fn setup() -> Option<(PathBuf, PathBuf)> {
    let pack = PathBuf::from(std::env::var("W3AL_FIXTURES").ok()?).join("pack_501");
    let install = std::env::var("W3AL_INSTALL").map_or_else(|_| PathBuf::from(export::DEFAULT_GAME_DIR), PathBuf::from);
    if !pack.join(format!("{}.sav", PACK[0].1)).is_file() || !install.join("content").is_dir() {
        eprintln!("skipped: needs W3AL_FIXTURES with pack_501 and the game install");
        return None;
    }
    Some((pack, install))
}

fn lib(install: &Path) -> &'static Library {
    static LIB: OnceLock<Library> = OnceLock::new();
    LIB.get_or_init(|| Library::load_install(install).expect("quest graphs"))
}

fn doc(pack: &Path, line: u32) -> Document {
    let stem = PACK.iter().find(|p| p.0 == line).expect("pack line").1;
    let b = std::fs::read(pack.join(format!("{stem}.sav"))).unwrap();
    let c = Container::read(&b).unwrap();
    Document::parse(&c.payload, c.data_start).unwrap()
}

/// (earlier pack, later pack, journal file stem, kind) — quests the later save has finished.
const SAMPLE: &[(u32, u32, &str, &str)] = &[
    (21, 33, "cgnovigrad", "gwent"),
    (21, 33, "mh303succubus", "contract"),
    (21, 33, "sq106killbill", "side"),
    (21, 33, "sq310dangerousgame", "side"),
    (35, 42, "mh206fiend", "contract"),
    (35, 42, "mh210lamia", "contract"),
    (35, 42, "sq108griffon", "contract"),
    (35, 42, "sq107pigs", "side"),
    (35, 42, "mq3041showingpresence", "side"),
    (35, 42, "q205herbarium", "main"),
    (42, 43, "cgtalar", "gwent"),
    (43, 48, "mq4004bastion", "side"),
    (43, 48, "mq4005sword", "side"),
    (43, 48, "mq4006armor", "side"),
    (43, 48, "q401_megascope", "main"),
    (43, 48, "wolfupgrades1b", "scavenger"),
];

#[test]
fn auto_completed_quests_match_the_saves_where_a_player_finished_them() {
    let Some((pack, install)) = setup() else { return };
    let lib = lib(&install);
    let idx = complete::index(lib).unwrap();
    // The walk is validated against the saves where a player finished the quests, decisions included (the sample holds
    // mh303succubus and q205herbarium, which the consequence check refuses for the generator).
    let opts = complete::Options { check_consequences: false, ..Default::default() };
    let mut lines = Vec::new();
    let (mut j_hit, mut j_miss, mut f_hit, mut f_miss) = (0, 0, 0, 0);
    let mut last: Option<(u32, Document, Document)> = None;
    for &(a, b, stem, kind) in SAMPLE {
        if last.as_ref().is_none_or(|l| l.0 != a) {
            last = Some((a, doc(&pack, a), doc(&pack, b)));
        }
        let (_, da, db) = last.as_ref().unwrap();
        let q = idx.quests.iter().find(|q| q.file.ends_with(&format!("/{stem}.journal"))).unwrap_or_else(|| panic!("{stem} not in the install"));
        let tf = w3graph::frontier::read(da).unwrap();
        let c = complete::complete(lib, &idx, da, &tf, q, &opts).unwrap_or_else(|r| panic!("{stem} (pack {a}): {r}"));
        let (sd, _) = w3graph::synth::apply(lib, &c.walk, da).unwrap_or_else(|e| panic!("{stem}: {e}"));
        let k = complete::check(lib, q, &c, da, &sd, db).unwrap();
        lines.push(format!(
            "{stem:<24} {kind:<9} pack {a}->{b}  picks {:>2}  journal {}/{}/{}  facts {}/{}/{}  phase {}/{}  sets {}/{}",
            c.picks.len(), k.journal.0, k.journal.1, k.journal.2, k.facts.0, k.facts.1, k.facts.2, k.scope_ended.0, k.scope_ended.1, k.community.0, k.community.1
        ));
        assert_eq!(k.root.0.as_deref(), Some("JS_Success"), "{stem}: synthesized root");
        assert_eq!(k.root.1.as_deref(), Some("JS_Success"), "{stem}: played root");
        assert!(k.scope_ended.0 || !k.scope_ended.1, "{stem}: the played save ended the quest's phase, the synthesized one did not");
        assert!(k.journal.0 > 0, "{stem}: no journal entry in common");
        j_hit += k.journal.0;
        j_miss += k.journal.1;
        f_hit += k.facts.0;
        f_miss += k.facts.1;
    }
    for l in &lines {
        eprintln!("{l}");
    }
    eprintln!("journal entries {j_hit} hit / {j_miss} miss, facts {f_hit} hit / {f_miss} miss");
    // Most of what the player's save holds for these quests is reproduced (measured 2026-11-05: see docs/research.md).
    assert!(j_hit * 10 >= (j_hit + j_miss) * 8, "journal {j_hit}/{j_miss}");
    assert!(f_hit * 10 >= (f_hit + f_miss) * 6, "facts {f_hit}/{f_miss}");
}

fn side_book() -> Questbook {
    let dir = paths::locate_questbook([PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../questbook")]).expect("questbook");
    Questbook::load_dir(&dir).expect("questbook loads")
}

/// Up to `n` generated side quests whose trial on `template` was `ok`.
fn run_with_side(book: &Questbook, template: &str, n: usize) -> Run {
    let mut run = Run::default();
    for q in book.side.quests.iter().filter(|q| q.templates.get(template).is_some_and(|o| o == "ok")).take(n) {
        run.side_quests.insert(q.id.clone(), Status::Completed);
    }
    run
}

#[test]
fn thirty_side_quests_export_verified_on_pack_43_and_pack_54() {
    let Some((pack, install)) = setup() else { return };
    let book = side_book();
    if book.side.quests.is_empty() {
        eprintln!("skipped: questbook/side is empty");
        return;
    }
    let lib = lib(&install);
    let opts = ExportOptions { game_dir: Some(install.clone()), ..Default::default() };
    for (line, template) in [(43, "w6_allies"), (54, "w9_skellige_act3")] {
        let run = run_with_side(&book, template, 30);
        assert!(run.side_quests.len() >= 25, "{template}: only {} side quests complete there", run.side_quests.len());
        let res = resolve(&book, &run);
        assert_eq!(res.effects.len(), run.side_quests.len());
        let stem = PACK.iter().find(|p| p.0 == line).unwrap().1;
        let bytes = std::fs::read(pack.join(format!("{stem}.sav"))).unwrap();
        let built = export::build_with_sources(&bytes, &res, &opts, &Default::default(), Some(lib)).unwrap_or_else(|e| panic!("pack {line}: {e}"));
        // Every side quest's journal root is Success in the result.
        let c = Container::read(&built.bytes).unwrap();
        let d = Document::parse(&c.payload, c.data_start).unwrap();
        let j = w3save::journal::read(&d).unwrap();
        for id in run.side_quests.keys() {
            let q = book.side.get(id).unwrap();
            let k = q.root_key().unwrap();
            let st = j.find(&k).first().map(|i| j.entries[*i].status.clone());
            assert_eq!(st.as_deref(), Some("JS_Success"), "pack {line}: {id}");
        }
        eprintln!("pack {line}: {} side quests, VERIFIED, {} bytes", run.side_quests.len(), built.bytes.len());
    }
}
