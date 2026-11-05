//! Side quests without consequences (side quests, contracts, treasure hunts, Gwent, races, fist fights,
//! scavenger hunts, and main-story quests between decisions): one small generated TOML per quest in
//! `questbook/side/` (`w3al gen-side-quests`), each with a `done` effect in the auto-complete mode
//! (`[effects.done.synth] complete = "GROUP/QUEST"`, `w3graph::complete`). The player marks them
//! completed or skipped (`run.side_quests`); export applies the completed ones after the story's own effects.
//!
//! No game text is stored: the official title is resolved from the player's install at run time by its
//! string id (`title_id`, `w3graph::names`); `fallback_title` is the quest's internal graph path. Schema:
//! `questbook/SCHEMA.md` "Side quests".
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostic::Diagnostic;
use crate::questbook::{Effect, LoadError, Questbook};
use crate::resolve::AppliedEffect;
use crate::run::{Run, Status};

/// The folder of the side-quest files inside the questbook folder (not loaded as decision quests).
pub const SIDE_DIR: &str = "side";

/// Regions a side quest can carry (the questbook's six plus `other`).
pub const REGIONS: &[&str] = &["white_orchard", "velen", "novigrad", "skellige", "kaer_morhen", "vizima", "other"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideKind {
    /// A main-story quest between decisions (listed apart).
    Main,
    Side,
    /// A witcher contract (`QuestType_MonsterHunt`).
    Contract,
    /// A treasure hunt (`QuestType_TreasureHunt`).
    Treasure,
    /// Scavenger hunt for witcher gear diagrams.
    Scavenger,
    Gwent,
    /// Horse race.
    Race,
    /// Fist fight.
    Fistfight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SideQuest {
    /// Unique, `snake_case`: the quest's journal file stem.
    pub id: String,
    pub kind: SideKind,
    /// One of [`REGIONS`].
    pub region: String,
    /// 1..6 as the questbook's acts.
    pub act: u8,
    /// Sort key inside the side list (unique among side quests).
    pub order: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// String id of the official title (`CJournalQuest.title`), resolved from the install at run time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<u32>,
    /// Neutral title when the install's strings are not at hand: the quest's internal graph path.
    pub fallback_title: String,
    /// The journal root `GROUP/QUEST` (32 hex digits each).
    pub journal: String,
    /// The quest's graph file (its home), for reviewers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Key of the effect applied when the quest is marked completed.
    pub done_effects: String,
    /// Decisions the auto-complete walk passes whose branches differ only in facts or journal entries that nothing
    /// outside the quest reads (`w3graph::consequence`), as `block GUID=socket the walk takes`. Informational: the
    /// UI can show what the walk chose for the player. A quest with a decision that matters is not generated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_picks: Vec<String>,
    /// The generator's trial on each window template of `questbook/templates.toml`: template id -> `ok` (completes),
    /// `done` (already done there) or the refusal reason. Informational; export decides on the real save.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub templates: BTreeMap<String, String>,
    #[serde(default)]
    pub effects: BTreeMap<String, Effect>,
    #[serde(skip)]
    pub file: PathBuf,
}

impl SideQuest {
    /// The journal root as two GUIDs (`None` when malformed).
    pub fn root_key(&self) -> Option<Vec<[u8; 16]>> {
        self.journal.split('/').map(parse_guid).collect()
    }
}

fn parse_guid(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut g = [0u8; 16];
    for (i, b) in g.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(g)
}

/// The side quests of a questbook folder (`<questbook>/side/**/*.toml`), sorted by `order`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SideBook {
    pub quests: Vec<SideQuest>,
}

impl SideBook {
    /// Load `dir` (the `side` folder); a missing folder is an empty book.
    pub fn load_dir(dir: &Path) -> Result<Self, LoadError> {
        let mut files = Vec::new();
        if dir.is_dir() {
            collect(dir, &mut files)?;
        }
        files.sort();
        let mut quests = Vec::with_capacity(files.len());
        for path in files {
            let text = fs::read_to_string(&path).map_err(|source| LoadError::Io { path: path.clone(), source })?;
            let mut q: SideQuest = toml::from_str(&text).map_err(|e| LoadError::Parse { path: path.clone(), message: e.to_string() })?;
            q.file = path;
            quests.push(q);
        }
        quests.sort_by_key(|q| q.order);
        Ok(Self { quests })
    }

    pub fn get(&self, id: &str) -> Option<&SideQuest> {
        self.quests.iter().find(|q| q.id == id)
    }

    /// Shape checks (lint): ids and orders unique and not shared with the decision questbook, regions, acts,
    /// the journal root, the done effect present, its `synth.complete` naming the same root, no placeholder.
    pub fn lint(&self, book: &Questbook) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut ids: HashMap<&str, usize> = HashMap::new();
        let mut orders: HashMap<u32, &str> = HashMap::new();
        let mut roots: HashMap<&str, &str> = HashMap::new();
        for q in &self.quests {
            let id = Some(q.id.as_str());
            *ids.entry(&q.id).or_default() += 1;
            if let Some(prev) = orders.insert(q.order, &q.id) {
                out.push(Diagnostic::error(id, format!("side order {} is also used by `{prev}`", q.order)));
            }
            if let Some(prev) = roots.insert(&q.journal, &q.id) {
                out.push(Diagnostic::error(id, format!("journal root {} is also `{prev}`", q.journal)));
            }
            if book.get(&q.id).is_some() {
                out.push(Diagnostic::error(id, "side quest id is also a decision-questbook quest id"));
            }
            if !q.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') || q.id.is_empty() {
                out.push(Diagnostic::error(id, "side quest id must be snake_case"));
            }
            if !REGIONS.contains(&q.region.as_str()) {
                out.push(Diagnostic::error(id, format!("region `{}` is not one of {}", q.region, REGIONS.join(", "))));
            }
            if !(1..=6).contains(&q.act) {
                out.push(Diagnostic::error(id, format!("act {} is not 1..6", q.act)));
            }
            if q.fallback_title.trim().is_empty() {
                out.push(Diagnostic::error(id, "empty fallback_title"));
            }
            if q.root_key().is_none_or(|k| k.len() != 2) {
                out.push(Diagnostic::error(id, format!("journal {:?}: want GROUP/QUEST, 32 hex digits each", q.journal)));
            }
            match q.effects.get(&q.done_effects) {
                None => out.push(Diagnostic::error(id, format!("done_effects refers to undefined effects `{}`", q.done_effects))),
                Some(e) => {
                    match e.synth.as_ref().and_then(|s| s.complete.as_deref()) {
                        None => out.push(Diagnostic::error(id, "the done effect has no `[synth] complete` table")),
                        Some(k) if !k.eq_ignore_ascii_case(&q.journal) => {
                            out.push(Diagnostic::error(id, format!("synth complete {k} is not the quest's journal root {}", q.journal)))
                        }
                        Some(_) => {}
                    }
                    if let Some(Err(m)) = e.synth.as_ref().map(crate::synth_effect::SynthEffect::check) {
                        out.push(Diagnostic::error(id, format!("effects `{}`: {m}", q.done_effects)));
                    }
                    if e.placeholder {
                        out.push(Diagnostic::error(id, "a side-quest effect cannot be a placeholder"));
                    }
                    if e.source.trim().is_empty() {
                        out.push(Diagnostic::error(id, "the done effect has no `source`"));
                    }
                }
            }
            for k in q.effects.keys().filter(|k| **k != q.done_effects) {
                out.push(Diagnostic::warn(id, format!("effects `{k}` is never referenced")));
            }
        }
        for (id, n) in ids {
            if n > 1 {
                out.push(Diagnostic::error(Some(id), format!("side quest id used {n} times")));
            }
        }
        out
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), LoadError> {
    let io = |source| LoadError::Io { path: dir.to_owned(), source };
    for entry in fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "toml") {
            out.push(path);
        }
    }
    Ok(())
}

/// What the generator derives from a journal quest: kind, region, act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub kind: SideKind,
    pub region: &'static str,
    pub act: u8,
}

/// Kind, region and act of a quest from its journal type (`QuestType_*`, as the file's names spell it, with or
/// without the prefix), its group's internal name, its journal file, its graph file and its `world` id.
/// Region: the graph path's area folder (`no_mans_land` = Velen, ...), else the world id (1 Novigrad, 2 Skellige,
/// 3 Kaer Morhen, 4 and 8 White Orchard, 5 Vizima, 9 Velen; read off the install's journals by area). Act: the
/// questbook's (1 White Orchard / Vizima, 2 Velen, 3 Novigrad, 4 Skellige, 5 Kaer Morhen); main quests of the
/// `Chapter 2` group are act 5, of `Chapter 3` act 6.
pub fn classify(qtype: &str, group: &str, journal_file: &str, home: &str, world: u32) -> Classified {
    let t = qtype.trim_start_matches("QuestType_").to_ascii_lowercase();
    let stem = journal_file.rsplit('/').next().unwrap_or(journal_file).trim_end_matches(".journal").to_ascii_lowercase();
    let g = group.to_ascii_lowercase();
    let starts_num = |p: &str| stem.strip_prefix(p).is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
    let kind = if t == "story" || t == "chapter" {
        SideKind::Main
    } else if t == "monsterhunt" {
        SideKind::Contract
    } else if t == "treasurehunt" {
        if stem.contains("upgrades") || starts_num("th") || stem.starts_with("th") && stem.ends_with("mq") || stem.contains("treasurehuntset") {
            SideKind::Scavenger
        } else {
            SideKind::Treasure
        }
    } else if g == "gwint" || stem.starts_with("cg") {
        SideKind::Gwent
    } else if starts_num("hr") || starts_num("br") || stem.contains("horserac") {
        SideKind::Race
    } else if stem.contains("fistfight") || starts_num("ff") {
        SideKind::Fistfight
    } else {
        SideKind::Side
    };
    let h = home.to_ascii_lowercase();
    let region = if h.contains("no_mans_land") || h.contains("/velen") {
        "velen"
    } else if h.contains("/novigrad") {
        "novigrad"
    } else if h.contains("/skellige") {
        "skellige"
    } else if h.contains("kaer_morhen") {
        "kaer_morhen"
    } else if h.contains("prologue") || h.contains("white_orchard") {
        "white_orchard"
    } else if h.contains("vizima") || h.contains("wyzima") {
        "vizima"
    } else {
        match world {
            9 => "velen",
            1 => "novigrad",
            2 => "skellige",
            3 => "kaer_morhen",
            4 | 8 => "white_orchard",
            5 => "vizima",
            _ => "other",
        }
    };
    let act = match (kind, g.as_str()) {
        (SideKind::Main, "chapter 2") => 5,
        (SideKind::Main, "chapter 3") => 6,
        _ => match region {
            "white_orchard" | "vizima" => 1,
            "velen" => 2,
            "novigrad" => 3,
            "skellige" => 4,
            "kaer_morhen" => 5,
            _ => 6,
        },
    };
    Classified { kind, region, act }
}

/// A side quest's id from its journal file (`gameplay/journal/quests/lynx upgrades 1b.journal` -> `lynx_upgrades_1b`).
pub fn side_id(journal_file: &str) -> String {
    let stem = journal_file.rsplit('/').next().unwrap_or(journal_file).trim_end_matches(".journal").to_ascii_lowercase();
    let mut id: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    while id.contains("__") {
        id = id.replace("__", "_");
    }
    id.trim_matches('_').to_owned()
}

/// The side quests a save already has finished (journal root Success or Failed): the Side quests page shows them
/// checked and disabled, and export would refuse them (`w3graph::complete::Refusal::AlreadyDone`).
pub fn done_in_save(side: &SideBook, save: &[u8]) -> Result<Vec<String>, String> {
    let c = w3save::Container::read(save).map_err(|e| e.to_string())?;
    let doc = w3save::Document::parse(&c.payload, c.data_start).map_err(|e| e.to_string())?;
    let j = w3save::journal::read(&doc).map_err(|e| e.to_string())?;
    Ok(side
        .quests
        .iter()
        .filter(|q| q.root_key().is_some_and(|k| j.find(&k).first().is_some_and(|i| matches!(j.entries[*i].status.as_str(), "JS_Success" | "JS_Failed"))))
        .map(|q| q.id.clone())
        .collect())
}

/// The done effects of the side quests the run marks completed, in side order (applied after the story's
/// own effects). Unknown ids in `run.side_quests` are ignored (the UI only offers known ones).
pub fn resolve(side: &SideBook, run: &Run) -> Vec<AppliedEffect> {
    side.quests
        .iter()
        .filter(|q| run.side_quests.get(&q.id) == Some(&Status::Completed))
        .filter_map(|q| q.effects.get(&q.done_effects).map(|e| AppliedEffect { quest: q.id.clone(), origin: "done".into(), effect: e.clone() }))
        .collect()
}

/// Graph files (lower-case file names) where the decision questbook's synth effects decide: the files of
/// every block a `pick` or `scene_pick` names (world hubs and act hubs aside: `.w2quest` files and the
/// `*sidequests*` phases that only start quests). The side-quest generator refuses a quest whose graph
/// holds one, besides `w3graph::complete::DECISION_FILES`.
pub fn decision_files(book: &Questbook, lib: &w3graph::quest::Library) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for q in book.quests.iter().filter(|q| !q.choices.is_empty()) {
        for e in q.effects.values() {
            let Some(s) = &e.synth else { continue };
            let keys = s.pick.iter().chain(&s.scene_pick).map(|k| k.split('=').next().unwrap_or(k));
            for k in keys {
                let hits: Vec<usize> = if k.len() == 32 { w3graph::quest::parse_guid(k).ok().and_then(|g| lib.get(&g)).into_iter().collect() } else { lib.by_short(k).to_vec() };
                // A short key the graphs hold several times names no file for sure.
                if hits.len() != 1 {
                    continue;
                }
                for b in hits {
                    let f = &lib.blocks[b].file;
                    if !w3graph::complete::is_shared_file(f) && !f.ends_with(".w2quest") && !f.contains("sidequests") {
                        out.insert(w3graph::complete::file_name(f));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quest(id: &str, order: u32) -> SideQuest {
        let journal = format!("67e5afe3ceb5c148a9cde5c4a99c7a84/{order:032x}");
        let effect = Effect {
            source: "quest graph (test)".into(),
            synth: Some(crate::synth_effect::SynthEffect { complete: Some(journal.clone()), ..Default::default() }),
            ..Default::default()
        };
        SideQuest {
            id: id.into(),
            kind: SideKind::Contract,
            region: "velen".into(),
            act: 2,
            order,
            level: None,
            title_id: Some(1),
            fallback_title: "quests/generic_quests/x".into(),
            journal,
            phase: None,
            done_effects: "done".into(),
            auto_picks: Vec::new(),
            templates: BTreeMap::new(),
            effects: [("done".to_owned(), effect)].into(),
            file: PathBuf::new(),
        }
    }

    #[test]
    fn classification() {
        let c = |t: &str, g: &str, j: &str, h: &str, w: u32| classify(t, g, j, h, w);
        assert_eq!(c("MonsterHunt", "Generic", "gameplay/journal/quests/mh101cockatrice.journal", "quests/generic_quests/no_mans_land/mh101_cockatrice.w2phase", 9), Classified { kind: SideKind::Contract, region: "velen", act: 2 });
        assert_eq!(c("Side", "Gwint", "gameplay/journal/quests/cgnovigrad.journal", "quests/generic_quests/novigrad/cg100.w2phase", 1).kind, SideKind::Gwent);
        assert_eq!(c("Side", "Generic", "gameplay/journal/quests/hr101baron.journal", "quests/generic_quests/no_mans_land/hr101_barons.w2phase", 9).kind, SideKind::Race);
        assert_eq!(c("Side", "Generic", "gameplay/journal/quests/fistfightnomansland.journal", "x", 9).kind, SideKind::Fistfight);
        assert_eq!(c("TreasureHunt", "dlc10", "dlc/dlc10/journal/quests/wolfupgrades1a.journal", "dlc/dlc10/data/quests/th1009_wolf/th1009_wolf.w2phase", 9).kind, SideKind::Scavenger);
        assert_eq!(c("TreasureHunt", "Treasure Hunt", "gameplay/journal/quests/sunkentreasure.journal", "x", 9).kind, SideKind::Treasure);
        let m = c("Chapter", "Chapter 3", "gameplay/journal/quests/q111imlerith.journal", "quests/part_3/q111_imlerith.w2phase", 1);
        assert_eq!((m.kind, m.act), (SideKind::Main, 6));
        assert_eq!(c("Side", "Minorquests", "gameplay/journal/quests/mq4003lake.journal", "quests/minor_quests/kaer_morhen/mq4003_lake.w2phase", 3).act, 5);
        assert_eq!(c("Side", "Minorquests", "gameplay/journal/quests/mq9.journal", "quests/x.w2phase", 0).region, "other");
        assert_eq!(side_id("gameplay/journal/quests/lynx upgrades 1b.journal"), "lynx_upgrades_1b");
        assert_eq!(side_id("gameplay/journal/quests/mq2011_liar_and_theif.journal"), "mq2011_liar_and_theif");
    }

    #[test]
    fn lint_and_resolve() {
        let side = SideBook { quests: vec![quest("mh101", 21001), quest("mh102", 21002)] };
        let book = Questbook::default();
        assert!(side.lint(&book).iter().all(|d| !d.is_error()), "{:?}", side.lint(&book));
        let mut bad = side.clone();
        bad.quests[1].order = 21001;
        bad.quests[1].region = "toussaint".into();
        bad.quests[1].effects.get_mut("done").unwrap().synth.as_mut().unwrap().complete = Some(bad.quests[0].journal.clone());
        let errs: Vec<String> = bad.lint(&book).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
        assert!(errs.iter().any(|m| m.contains("also used")), "{errs:?}");
        assert!(errs.iter().any(|m| m.contains("region")), "{errs:?}");
        assert!(errs.iter().any(|m| m.contains("is not the quest's journal root")), "{errs:?}");
        let mut run = Run::default();
        run.side_quests.insert("mh102".into(), Status::Completed);
        run.side_quests.insert("mh101".into(), Status::Skipped);
        let r = resolve(&side, &run);
        assert_eq!(r.iter().map(|a| a.quest.as_str()).collect::<Vec<_>>(), ["mh102"]);
        // TOML round trip of the generated shape.
        let text = toml::to_string_pretty(&side.quests[0]).unwrap();
        let back: SideQuest = toml::from_str(&text).unwrap();
        assert_eq!(back, side.quests[0]);
    }
}
