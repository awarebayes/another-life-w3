//! The story chain on real saves (docs/plan/story-chain.md):
//!
//! 1. **Rewrite pairs.** For decisions where both branches exist as saves a human played from the same
//!    template (session V1/V2, `BRANCHES.txt`), take the played save of branch A, rewrite it to branch B
//!    with the delta measured between the two *synthesized* references on the window template (no donor:
//!    the references are walked from the game's quest graphs), and compare with the played save of
//!    branch B: the facts that tell the branches apart, the journal entries the branches differ by and the
//!    waits the branches differ by must all match (0 differences), in both directions.
//! 2. **Full story on pack 54.** The shipped questbook and registry, a run up to Child of the Elder Blood
//!    with several earlier answers that differ from the pack line, exported through `chain::export_story`
//!    (template chosen automatically, earlier windows rewritten, VERIFIED).
//!
//! Gated: `W3AL_FIXTURES` (holding `session_V1`, `session_V2`, `pack_501`) and the game install at
//! `W3AL_INSTALL` or the Steam default. Skips (passes) when either is missing. Outputs go to a fresh temp
//! folder (`create_new`) and are removed.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use w3al_core::chain::{self, Sources};
use w3al_core::export::{ExportOptions, DEFAULT_GAME_DIR};
use w3al_core::questbook::Quest;
use w3al_core::run::{QuestEntry, Status};
use w3al_core::templates::{self, Baked, Registry, Template};
use w3al_core::{Questbook, Run};
use w3graph::quest::Library;
use w3save::journal;
use w3save::{Container, Document};

fn fixtures() -> Option<PathBuf> {
    let f = PathBuf::from(std::env::var_os("W3AL_FIXTURES")?);
    f.join("session_V1").is_dir().then_some(f)
}

fn install() -> Option<PathBuf> {
    let p = std::env::var_os("W3AL_INSTALL").map_or_else(|| PathBuf::from(DEFAULT_GAME_DIR), PathBuf::from);
    p.join("content").join("content0").join("bundles").is_dir().then_some(p)
}

fn load(dir: &Path, stem: &str) -> Document {
    let b = std::fs::read(dir.join(format!("{stem}.sav"))).unwrap_or_else(|e| panic!("{stem}: {e}"));
    let c = Container::read(&b).unwrap();
    Document::parse(&c.payload, c.data_start).unwrap()
}

/// Test quests (not content): the three decisions with both branches played, as synth effects with the
/// walk arguments of docs/plan/no-human-pipeline.md (spirit, Keira) and docs/research.md (Letho).
const QUESTS: &[&str] = &[
    r#"
id = "spirit_t"
title = "spirit"
act = 2
order = 10
kind = "secondary"
region = "velen"
level = 1
summary = "s"
[[choices]]
id = "spirit"
prompt = "?"
default = "release"
options = [ { id = "release", text = "r", effects = "released" }, { id = "destroy", text = "d", effects = "destroyed" } ]
[effects.released]
source = "test"
[effects.released.synth]
start = ["c2549952", "c4d54c91"]
through = true
pick = ["ba20c454=q105_ritual_done_out"]
stop = ["51f0f429", "0d3a7efd"]
[effects.destroyed]
source = "test"
[effects.destroyed.synth]
start = ["c2549952", "c4d54c91"]
through = true
pick = ["ba20c454=q105_betrayed_out"]
stop = ["1f5f6045"]
"#,
    r#"
id = "keira_t"
title = "keira"
act = 2
order = 20
kind = "secondary"
region = "velen"
level = 1
summary = "s"
[[choices]]
id = "keira"
prompt = "?"
default = "km"
options = [ { id = "km", text = "k", effects = "km" }, { id = "radovid", text = "r", effects = "radovid" } ]
[effects.km]
source = "test"
[effects.km.synth]
start = ["215f2719"]
through = true
pick = ["d24f649d=Keira to KM"]
stop = ["4c2a9b2d"]
[effects.radovid]
source = "test"
[effects.radovid.synth]
start = ["215f2719"]
through = true
pick = ["d24f649d=Keira to Radovid"]
stop = ["4c2a9b2d"]
"#,
    r#"
id = "letho_t"
title = "letho"
act = 2
order = 30
kind = "secondary"
region = "velen"
level = 1
summary = "s"
[[choices]]
id = "letho"
prompt = "?"
default = "invited"
options = [ { id = "invited", text = "i", effects = "invited" }, { id = "elsewhere", text = "e", effects = "elsewhere" } ]
[effects.invited]
source = "test"
[effects.invited.synth]
start = ["2da9cd80"]
through = true
pick = ["70d40ee9=NoFight"]
scene_pick = ["ed477b93=sq102_go _to_km_letho"]
stop = ["5f88ff5b"]
[effects.elsewhere]
source = "test"
[effects.elsewhere.synth]
start = ["2da9cd80"]
through = true
pick = ["70d40ee9=NoFight"]
scene_pick = ["ed477b93=sq102_find_shelter"]
stop = ["5f88ff5b"]
"#,
];

const REGISTRY: &str = r#"
[[template]]
id = "t_spirit"
stem = "ManualSave_6255c_7ea51000_8f3325"
line = 8
quests = ["spirit_t"]
[[template]]
id = "t_keira"
stem = "ManualSave_568ef_7ea51000_923cb5"
line = 13
quests = ["keira_t"]
[[template]]
id = "t_letho"
stem = "ManualSave_6b9be_7ea51000_9bb30d"
line = 14
quests = ["letho_t"]
"#;

fn test_book() -> Questbook {
    Questbook::from_quests(QUESTS.iter().map(|q| toml::from_str::<Quest>(q).unwrap()).collect())
}

fn present(t: &w3save::facts::FactTable, n: &str) -> Option<i64> {
    t.get(n).filter(|f| !f.records.is_empty()).map(w3save::facts::Fact::sum)
}

#[derive(Debug, Default)]
struct PairStats {
    decision_facts: usize,
    facts_before: usize,
    facts_after: usize,
    journal_keys: usize,
    journal_before: usize,
    journal_after: usize,
    waits: usize,
    waits_before: usize,
    waits_after: usize,
}

/// Rewrite played save `from` (answer `a`) into answer `b` and compare with played save `to`.
#[allow(clippy::too_many_arguments)]
fn pair(lib: &Library, src: &Sources, window: &str, quest: &str, choice: &str, a: &str, b: &str, from: &Document, to: &Document) -> PairStats {
    let w = src.registry.get(window).unwrap().clone();
    let chosen = Template { encodes: [(quest.to_owned(), [(choice.to_owned(), a.to_owned())].into())].into(), ..w.clone() };
    let mut run = Run::default();
    run.quests.insert(quest.into(), QuestEntry { status: Status::Completed, answers: [(choice.to_owned(), b.to_owned())].into() });
    let (rw, ra, rb) = chain::references(src, Some(lib), &run, &chosen, &w).unwrap();
    let d = chain::window_delta(&rw, &ra, &rb).unwrap();
    let (out, rep) = chain::apply_delta(Some(lib), from, &d).unwrap();
    eprintln!("  delta {}\n  rewrite {}", d.summary(), rep.summary());
    let here = w3graph::frontier::read(from).unwrap().guids();
    let common: Vec<String> = d.common_new.iter().filter(|g| here.contains(*g)).map(|g| lib.describe(lib.get(g).unwrap())).collect();
    eprintln!("  common aftermath here: {common:?}");

    let mut s = PairStats::default();
    let (ft, fo, fd) = (from.facts_table().unwrap(), out.facts_table().unwrap(), to.facts_table().unwrap());
    s.decision_facts = d.facts.len();
    for (n, _, _) in &d.facts {
        let (x, y, z) = (present(&ft, n), present(&fo, n), present(&fd, n));
        s.facts_before += usize::from(x.is_some() != z.is_some());
        if y.is_some() != z.is_some() {
            s.facts_after += 1;
            eprintln!("    fact {n}: rewritten {y:?}, played {z:?}");
        }
    }
    let (jt, jo, jd) = (journal::read(from).unwrap(), journal::read(&out).unwrap(), journal::read(to).unwrap());
    let mut keys: Vec<Vec<journal::Guid>> = d.journal_a.iter().chain(&d.journal_b).map(journal::Entry::key).collect();
    keys.extend(d.journal_changed.iter().map(|(x, _)| x.key()));
    s.journal_keys = keys.len();
    let st = |j: &journal::Journal, k: &[journal::Guid]| j.find(k).first().map(|&i| j.entries[i].status.clone());
    for k in &keys {
        s.journal_before += usize::from(st(&jt, k) != st(&jd, k));
        if st(&jo, k) != st(&jd, k) {
            s.journal_after += 1;
            eprintln!("    journal {}: rewritten {:?}, played {:?}", journal::key_hex(k), st(&jo, k), st(&jd, k));
        }
    }
    let (gt, go, gd) = (
        w3graph::frontier::read(from).unwrap().guids(),
        w3graph::frontier::read(&out).unwrap().guids(),
        w3graph::frontier::read(to).unwrap().guids(),
    );
    let waits: Vec<_> = d.frontier_a.iter().chain(&d.frontier_b).map(|b| b.guid).collect();
    s.waits = waits.len();
    for g in &waits {
        s.waits_before += usize::from(gt.contains(g) != gd.contains(g));
        if go.contains(g) != gd.contains(g) {
            s.waits_after += 1;
            eprintln!("    wait {}: rewritten {}, played {}", w3graph::quest::short(g), go.contains(g), gd.contains(g));
        }
    }
    eprintln!("  {s:?}");
    s
}

#[test]
fn rewriting_a_played_branch_gives_the_other_played_branch() {
    let (Some(fx), Some(inst)) = (fixtures(), install()) else {
        eprintln!("skipped: W3AL_FIXTURES with session_V1 and the game install are needed");
        return;
    };
    let (v1, v2) = (fx.join("session_V1"), fx.join("session_V2"));
    let lib = Library::load_install(&inst).unwrap();
    let book = test_book();
    let reg: Registry = toml::from_str(REGISTRY).unwrap();
    let opts = ExportOptions { game_dir: Some(inst), ..Default::default() };
    let src = Sources { book: &book, registry: &reg, template_dirs: vec![v1.clone()], opts };

    let cases = [
        ("t_spirit", "spirit_t", "spirit", ("destroy", &v1, "ManualSave_52587_7ea51000_91a5f1"), ("release", &v1, "ManualSave_52587_7ea51000_900fd9")),
        ("t_keira", "keira_t", "keira", ("km", &v1, "ManualSave_568ef_7ea51000_94abd5"), ("radovid", &v1, "ManualSave_568ef_7ea51000_97cdc9")),
        ("t_letho", "letho_t", "letho", ("invited", &v2, "ManualSave_52584_7ea51000_a4841f"), ("elsewhere", &v1, "ManualSave_52584_7ea51000_9d341b")),
    ];
    for (window, quest, choice, (oa, da, sa), (ob, db, sb)) in cases {
        let (x, y) = (load(da, sa), load(db, sb));
        for (a, b, from, to) in [(oa, ob, &x, &y), (ob, oa, &y, &x)] {
            eprintln!("{quest}: {a} -> {b}");
            let s = pair(&lib, &src, window, quest, choice, a, b, from, to);
            assert!(s.decision_facts > 0 && s.journal_keys > 0, "{quest}: the branches differ by nothing? {s:?}");
            assert_eq!(s.facts_after, 0, "{quest} {a} -> {b}: decision facts differ from the played branch");
            assert_eq!(s.journal_after, 0, "{quest} {a} -> {b}: journal entries differ from the played branch");
        }
    }
}

fn entry(answers: &[(&str, &str)]) -> QuestEntry {
    QuestEntry { status: Status::Completed, answers: answers.iter().map(|(c, o)| ((*c).to_owned(), (*o).to_owned())).collect::<BTreeMap<_, _>>() }
}

#[test]
fn full_story_on_the_latest_template() {
    let (Some(fx), Some(inst)) = (fixtures(), install()) else {
        eprintln!("skipped: W3AL_FIXTURES and the game install are needed");
        return;
    };
    if !fx.join("pack_501").is_dir() {
        eprintln!("skipped: no pack_501");
        return;
    }
    let qb = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../questbook");
    let book = Questbook::load_dir(&qb).unwrap();
    let reg = Registry::load_for_questbook(&qb).unwrap();
    let run = story_run();
    let dirs = vec![fx.join("pack_501"), fx.join("session_V1")];
    let opts = ExportOptions {
        game_dir: Some(inst),
        donor_dirs: vec![fx.join("session_V1"), fx.join("session_V2")],
        allow_name_remap: true,
        ..Default::default()
    };
    let src = Sources { book: &book, registry: &reg, template_dirs: dirs.clone(), opts };
    let choice = templates::choose(&book, &reg, &run, &|t| templates::find_file(&dirs, &t.stem)).unwrap();
    assert_eq!(choice.template.id, "w9_skellige_act3", "{:?}", choice.passed_over);
    let rewrites: Vec<&str> = choice.rewrites().map(|b| b.quest.as_str()).collect();
    eprintln!("rewrites: {rewrites:?}; fixed: {:?}", choice.baked.iter().filter(|b| matches!(b.action, Baked::Fixed { .. })).collect::<Vec<_>>());
    assert!(rewrites.len() >= 3, "{rewrites:?}");
    let out = std::env::temp_dir().join(format!("w3al-story-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let r = chain::export_story(&src, &run, &out).unwrap();
    for n in &r.result.notes {
        eprintln!("  {n}");
    }
    // The decisions the player changed hold the player's facts in the written save.
    let doc = load(&out, r.result.sav.file_stem().unwrap().to_str().unwrap());
    let f = doc.facts_table().unwrap();
    for (fact, want) in expected_facts() {
        assert_eq!(present(&f, fact).is_some(), want, "{fact}");
    }
    std::fs::remove_dir_all(&out).unwrap();
}

/// A whole story up to Child of the Elder Blood: the pack line except where marked.
fn story_run() -> Run {
    let mut run = Run { stop_after: Some("child_of_the_elder_blood".into()), ..Run::default() };
    let mut q = |id: &str, a: &[(&str, &str)]| {
        run.quests.insert(id.into(), entry(a));
    };
    q("lilac_and_gooseberries", &[]);
    q("beast_of_white_orchard", &[]);
    q("family_matters", &[]);
    q("whispering_hillock", &[("spirit", "destroy")]);
    q("a_favor_for_a_friend", &[("date", "night")]);
    q("for_the_advancement_of_learning", &[("keira", "kaer_morhen")]);
    q("ghosts_of_the_past", &[("letho", "kaer_morhen")]);
    q("get_junior", &[("junior", "kill")]);
    q("lord_of_undvik", &[("folan", "folan_lives")]);
    q("kings_gambit", &[("side", "in_game")]);
    q("now_or_never", &[("triss", "go")]); // differs (pack: stay)
    q("the_last_wish", &[("yennefer", "love")]); // differs (pack: friends)
    q("ugly_baby", &[]);
    q("brothers_in_arms", &[]);
    // differs in all three (pack: snowball, imlerith, none)
    q("blood_on_the_battlefield", &[("snowball", "talk"), ("ride", "emhyr"), ("reward", "accept")]);
    q("final_preparations", &[("dijkstra", "spared"), ("lodge", "help")]); // lodge differs (pack: alone)
    q("reason_of_state", &[("plot", "agree"), ("side", "dijkstra")]);
    q("child_of_the_elder_blood", &[("necklace", "necklace"), ("grave", "skip")]);
    run
}

fn expected_facts() -> Vec<(&'static str, bool)> {
    vec![
        ("q309_triss_lover", false),
        ("sq202_yen_girlfriend", true),
        ("sq202_yen_just_friends", false),
        ("q110_calmed_down_ciri", false),
        ("q110_visited_emhyr", true),
        ("q110_accepted_reward", true),
        ("q310_ciri_faced_lodge", false),
        ("q210_gave_lara_necklace", true),
        // kept from the pack line
        ("q302_whoreson_dead", true),
        ("q109_keira_to_km", true),
    ]
}

/// The template choice on the shipped registry (no files needed): a changed answer that cannot be
/// rewritten on the latest template moves the save to an earlier one and says what is lost.
#[test]
fn shipped_registry_choices() {
    let qb = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../questbook");
    let book = Questbook::load_dir(&qb).unwrap();
    let reg = Registry::load_for_questbook(&qb).unwrap();
    assert!(reg.lint(&book).iter().all(|d| !d.is_error()), "{:?}", reg.lint(&book));
    let all = |t: &Template| Some(PathBuf::from(format!("{}.sav", t.stem)));
    let run = story_run();
    let c = templates::choose(&book, &reg, &run, &all).unwrap();
    assert_eq!(c.template.id, "w9_skellige_act3");
    let rw: Vec<&str> = c.rewrites().map(|b| b.quest.as_str()).collect();
    assert_eq!(rw, ["now_or_never", "the_last_wish", "blood_on_the_battlefield", "final_preparations"]);
    // Junior spared: Final Preparations (pack 51 -> 52) already read it, so the save is pack 51's.
    let mut spared = story_run();
    spared.quests.insert("get_junior".into(), entry(&[("junior", "spare")]));
    let c = templates::choose(&book, &reg, &spared, &all).unwrap();
    assert_eq!(c.template.id, "w8_novigrad_act3");
    assert_eq!(c.beyond, ["child_of_the_elder_blood"]);
    assert!(c.passed_over[0].contains("get_junior"), "{:?}", c.passed_over);
    // The spirit released: only T_spirit holds it; everything after is beyond.
    let mut released = story_run();
    released.quests.insert("whispering_hillock".into(), entry(&[("spirit", "release")]));
    let c = templates::choose(&book, &reg, &released, &all).unwrap();
    assert_eq!(c.template.id, "w1_spirit");
    assert!(c.beyond.contains(&"child_of_the_elder_blood".to_string()));
    // Stopping at Brothers in Arms with Keira sent to Radovid: W6, Keira rewritten before the battle.
    let mut keira = story_run();
    keira.stop_after = Some("brothers_in_arms".into());
    keira.quests.insert("for_the_advancement_of_learning".into(), entry(&[("keira", "radovid")]));
    let c = templates::choose(&book, &reg, &keira, &all).unwrap();
    assert_eq!(c.template.id, "w6_allies");
    assert_eq!(c.rewrites().map(|b| b.quest.as_str()).collect::<Vec<_>>(), ["for_the_advancement_of_learning", "now_or_never", "the_last_wish"]);
}

/// Now or Never before Skellige: a run that stops at Lord of Undvik with Now or Never done gets the Skellige
/// template (pack 33), which has not reached it; the registry's `late` list completes it there with the run's
/// answer (`[effects.*.late]`: sq301 then q309, auto-completed with the player's picks).
fn undvik_run(triss: &str) -> Run {
    let mut run = story_run();
    run.stop_after = Some("lord_of_undvik".into());
    run.quests.retain(|k, _| !matches!(k.as_str(), "the_last_wish" | "kings_gambit" | "ugly_baby" | "brothers_in_arms" | "blood_on_the_battlefield" | "final_preparations" | "reason_of_state" | "child_of_the_elder_blood"));
    run.quests.insert("now_or_never".into(), entry(&[("triss", triss)]));
    run
}

#[test]
fn now_or_never_completed_late_on_the_skellige_template() {
    let qb = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../questbook");
    let book = Questbook::load_dir(&qb).unwrap();
    let reg = Registry::load_for_questbook(&qb).unwrap();
    let all = |t: &Template| Some(PathBuf::from(format!("{}.sav", t.stem)));
    let c = templates::choose(&book, &reg, &undvik_run("go"), &all).unwrap();
    assert_eq!(c.template.id, "w4_undvik");
    assert_eq!(c.late, ["now_or_never"]);
    assert!(c.beyond.is_empty(), "{:?}", c.beyond);

    let (Some(fx), Some(inst)) = (fixtures(), install()) else {
        eprintln!("skipped: W3AL_FIXTURES and the game install are needed");
        return;
    };
    if !fx.join("pack_501").is_dir() {
        eprintln!("skipped: no pack_501");
        return;
    }
    let dirs = vec![fx.join("pack_501")];
    let opts = ExportOptions { game_dir: Some(inst), donor_dirs: vec![fx.join("session_V1"), fx.join("session_V2")], allow_name_remap: true, ..Default::default() };
    let src = Sources { book: &book, registry: &reg, template_dirs: dirs, opts };
    for (triss, lover) in [("go", false), ("stay", true)] {
        let out = std::env::temp_dir().join(format!("w3al-late-{triss}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let r = chain::export_story(&src, &undvik_run(triss), &out).unwrap();
        for n in &r.result.notes {
            eprintln!("  {n}");
        }
        let doc = load(&out, r.result.sav.file_stem().unwrap().to_str().unwrap());
        let f = doc.facts_table().unwrap();
        for (fact, want) in [("sq301_completed", true), ("q309_completed", true), ("q309_triss_lover", lover), ("q309_triss_stayed", lover), ("q309_asked_triss_to_stay", lover), ("sq301_kissed_triss", lover)] {
            assert_eq!(present(&f, fact).is_some(), want, "{triss}: {fact}");
        }
        // The ways the player did not take stay out (stops): no refusing or abandoning Triss.
        for fact in ["q309_triss_refused", "q309_abandon_triss_rescue"] {
            assert!(present(&f, fact).is_none(), "{triss}: {fact}");
        }
        let j = journal::read(&doc).unwrap();
        for root in ["a93885ce3a22194c955e802d55358928/32105cdf52d0cf43b1426705b0578c5e", "419f978ff624ed49a444903714632529/400c9506df5222448169c603f3b4d23f"] {
            let key: Vec<[u8; 16]> = root.split('/').map(|g| journal::parse_guid(g).unwrap()).collect();
            let st = j.find(&key).first().map(|i| j.entries[*i].status.clone());
            assert_eq!(st.as_deref(), Some("JS_Success"), "{triss}: {root}");
        }
        std::fs::remove_dir_all(&out).unwrap();
    }
}
