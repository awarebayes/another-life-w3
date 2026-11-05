//! The template registry (`questbook/templates.toml`) and the automatic template choice of a run.
//!
//! A *window template* is a real 5.01 save parked before the decisions of one decision window
//! (docs/plan/decision-chaining.md). The templates sit on one line (the pack playthrough they come
//! from, ordered by `line`); a later template already holds the pack player's answers to every earlier
//! window (`encodes`, read off the save with `w3al template-scan`, never typed by hand). For a run we
//! take the latest template at or before the run's stop point whose earlier answers either match the
//! player's or can be rewritten after the fact (`fixed` lists the ones that cannot, with the reason);
//! the quests of later windows are not in that save and the app says so. Design and evidence:
//! docs/plan/story-chain.md.
//!
//! ```toml
//! [[template]]
//! id     = "w9_skellige_act3"
//! stem   = "ManualSave_4dcef_7ea51000_400b253"    # file `<stem>.sav` in the template folders
//! line   = 54                                      # position on the template line (pack save number)
//! quests = ["child_of_the_elder_blood"]            # the window: quests decided on this template
//! source = "pack 54, 5.01 re-save"
//! [template.encodes]                               # earlier quests already done in this save, with answers
//! get_junior = { junior = "kill" }
//! ugly_baby  = {}                                  # done, no choices
//! [template.fixed]                                 # earlier quests whose answer cannot be changed here
//! whispering_hillock = "the Baron finale (q107) already played the spirit's outcome"
//!
//! [[line]]                                         # quests on the line without a usable template
//! quest = "a_favor_for_a_friend"
//! line  = 12
//! note  = "pack 12 has no 5.01 re-save"
//! ```
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostic::Diagnostic;
use crate::questbook::Questbook;
use crate::run::{Run, Status};

/// File name of the registry inside the questbook folder (not loaded as a quest).
pub const REGISTRY_FILE: &str = "templates.toml";
/// Default template folder name, next to the questbook folder.
pub const DEFAULT_DIR: &str = "templates";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    #[serde(default, rename = "template")]
    pub templates: Vec<Template>,
    #[serde(default)]
    pub line: Vec<LineQuest>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub stem: String,
    pub line: u32,
    /// The window: quests whose effects apply on this template, in questbook order.
    pub quests: Vec<String>,
    #[serde(default)]
    pub source: String,
    /// quest -> (choice -> option): earlier quests this save already holds as completed, with the answers.
    #[serde(default)]
    pub encodes: BTreeMap<String, BTreeMap<String, String>>,
    /// quest -> reason: earlier quests whose answer cannot be rewritten on this template.
    #[serde(default)]
    pub fixed: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineQuest {
    pub quest: String,
    pub line: u32,
    #[serde(default)]
    pub note: String,
}

impl Registry {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// `questbook/templates.toml`, or an empty registry when the file does not exist.
    pub fn load_for_questbook(questbook_dir: &Path) -> Result<Self, String> {
        let p = questbook_dir.join(REGISTRY_FILE);
        if p.is_file() {
            Self::load(&p)
        } else {
            Ok(Self::default())
        }
    }

    pub fn get(&self, id: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.id == id)
    }

    /// The window template a quest is decided on.
    pub fn window_of(&self, quest: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.quests.iter().any(|q| q == quest))
    }

    /// The quest's position on the template line: its window template's, else its `[[line]]` entry.
    pub fn line_of(&self, quest: &str) -> Option<u32> {
        self.window_of(quest).map(|t| t.line).or_else(|| self.line.iter().find(|l| l.quest == quest).map(|l| l.line))
    }

    /// Templates sorted by line, latest first.
    fn latest_first(&self) -> Vec<&Template> {
        let mut v: Vec<&Template> = self.templates.iter().collect();
        v.sort_by(|a, b| b.line.cmp(&a.line).then_with(|| a.id.cmp(&b.id)));
        v
    }

    /// Shape checks against the questbook (lint): unknown quests, choices and options, a quest in two
    /// windows, a quest off the line, encodes/fixed naming a quest that is not earlier on the line.
    pub fn lint(&self, book: &Questbook) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let err = |m: String| Diagnostic::error(None, format!("{REGISTRY_FILE}: {m}"));
        let mut seen_ids = Vec::new();
        for t in &self.templates {
            if seen_ids.contains(&t.id) {
                out.push(err(format!("template id `{}` is used twice", t.id)));
            }
            seen_ids.push(t.id.clone());
            if crate::export::check_stem(&t.stem).is_err() {
                out.push(err(format!("template `{}`: stem {:?} is not a file stem", t.id, t.stem)));
            }
            for q in &t.quests {
                if book.get(q).is_none() {
                    out.push(err(format!("template `{}`: unknown quest `{q}`", t.id)));
                }
                if self.templates.iter().filter(|o| o.quests.contains(q)).count() > 1 {
                    out.push(err(format!("quest `{q}` is in more than one window")));
                }
            }
            for (q, answers) in &t.encodes {
                let Some(quest) = book.get(q) else {
                    out.push(err(format!("template `{}`: encodes unknown quest `{q}`", t.id)));
                    continue;
                };
                if self.line_of(q).is_some_and(|l| l >= t.line) && !t.quests.contains(q) {
                    out.push(err(format!("template `{}`: encodes `{q}`, which is not earlier on the line", t.id)));
                }
                for (c, o) in answers {
                    match quest.choices.iter().find(|x| &x.id == c) {
                        None => out.push(err(format!("template `{}`: `{q}` has no choice `{c}`", t.id))),
                        Some(ch) if !ch.options.iter().any(|x| &x.id == o) => {
                            out.push(err(format!("template `{}`: `{q}.{c}` has no option `{o}`", t.id)))
                        }
                        _ => {}
                    }
                }
            }
            for k in t.fixed.keys() {
                let q = fixed_key_quest(k);
                let Some(quest) = book.get(q) else {
                    out.push(err(format!("template `{}`: fixed names unknown quest `{q}`", t.id)));
                    continue;
                };
                if let Some(rest) = k.strip_prefix(q).and_then(|r| r.strip_prefix('.')) {
                    let (c, o) = rest.split_once('=').map_or((rest, None), |(c, o)| (c, Some(o)));
                    match quest.choices.iter().find(|x| x.id == c) {
                        None => out.push(err(format!("template `{}`: fixed key `{k}`: no choice `{c}`", t.id))),
                        Some(ch) if o.is_some_and(|o| !ch.options.iter().any(|x| x.id == o)) => {
                            out.push(err(format!("template `{}`: fixed key `{k}`: no such option", t.id)))
                        }
                        _ => {}
                    }
                } else if k != q {
                    out.push(err(format!("template `{}`: fixed key `{k}`: want quest, quest.choice or quest.choice=option", t.id)));
                }
            }
        }
        for l in &self.line {
            if book.get(&l.quest).is_none() {
                out.push(err(format!("line: unknown quest `{}`", l.quest)));
            }
            if self.window_of(&l.quest).is_some() {
                out.push(err(format!("line: `{}` is already in a window", l.quest)));
            }
        }
        if !self.templates.is_empty() {
            for q in &book.quests {
                if self.line_of(&q.id).is_none() {
                    out.push(Diagnostic::warn(Some(&q.id), format!("{REGISTRY_FILE}: quest has no place on the template line")));
                }
            }
        }
        out
    }
}

/// Folders searched for template saves: the run's `template_dirs` (relative ones against `base`; the player's
/// own override, searched first), then the `templates` folder next to the questbook folder (shipped with the
/// app: the installer bundles `templates/` beside `questbook/`).
pub fn search_dirs(run: &Run, base: &Path, questbook_dir: &Path) -> Vec<PathBuf> {
    let parent = questbook_dir.parent().unwrap_or(questbook_dir);
    let mut dirs: Vec<PathBuf> = run.template_dirs.iter().map(|d| base.join(d)).collect();
    let shipped = parent.join(DEFAULT_DIR);
    if !dirs.contains(&shipped) {
        dirs.push(shipped);
    }
    dirs
}

/// `<stem>.sav` in the first folder that has it (not recursive).
pub fn find_file(dirs: &[PathBuf], stem: &str) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(format!("{stem}.sav"))).find(|p| p.is_file())
}

/// What happens to an earlier quest (already in the chosen template) for this run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Baked {
    /// The template holds the player's answers.
    Same,
    /// The answers differ: the decision is rewritten after the fact (facts, journal, live quest-graph state).
    Rewrite { from: BTreeMap<String, String>, to: BTreeMap<String, String> },
    /// The template holds other answers and they cannot be changed here.
    Fixed { from: BTreeMap<String, String>, to: BTreeMap<String, String>, why: String },
    /// The registry does not say what the template holds for this quest; kept as it is.
    Unchecked,
}

#[derive(Debug, Clone, Serialize)]
pub struct BakedQuest {
    pub quest: String,
    #[serde(flatten)]
    pub action: Baked,
}

/// The template choice for a run.
#[derive(Debug, Clone, Serialize)]
pub struct Choice {
    pub template: Template,
    /// The file, when found in the search folders.
    pub path: Option<PathBuf>,
    /// Quests whose effects export applies on the template (the window's, in questbook order).
    pub applied: Vec<String>,
    /// Earlier quests, already in the template.
    pub baked: Vec<BakedQuest>,
    /// Quests the run completes or starts up to its stop point that come after the template on the
    /// line: they are not in the save (decide them in game).
    pub beyond: Vec<String>,
    /// Later templates passed over, with the reason (an earlier answer that cannot be rewritten there,
    /// or the file is missing).
    pub passed_over: Vec<String>,
    pub notes: Vec<String>,
}

impl Choice {
    pub fn rewrites(&self) -> impl Iterator<Item = &BakedQuest> {
        self.baked.iter().filter(|b| matches!(b.action, Baked::Rewrite { .. }))
    }
}

/// The player's answers to a quest's choices (defaults filled in), or `None` when the run does not
/// complete it.
pub fn answers(book: &Questbook, run: &Run, quest: &str) -> Option<BTreeMap<String, String>> {
    let q = book.get(quest)?;
    let e = run.quests.get(quest).filter(|e| e.status == Status::Completed)?;
    Some(q.choices.iter().map(|c| (c.id.clone(), e.answers.get(&c.id).cloned().unwrap_or_else(|| c.default.clone()))).collect())
}

/// The run's stop quest: `stop_after`, else the last quest (book order) the run completes or starts.
pub fn stop_quest(book: &Questbook, run: &Run) -> Option<String> {
    run.stop_after.clone().or_else(|| {
        book.quests
            .iter()
            .rev()
            .find(|q| run.quests.get(&q.id).is_some_and(|e| e.status != Status::Skipped))
            .map(|q| q.id.clone())
    })
}

/// What happens to `quest` (already done in template `t`) for `run`: same, rewrite, fixed or unchecked.
/// Edit mode (`edit.rs`) calls it with a pseudo template whose `encodes` are read off the opened save.
pub fn baked_action(book: &Questbook, run: &Run, t: &Template, quest: &str) -> Baked {
    let Some(enc) = t.encodes.get(quest) else { return Baked::Unchecked };
    let want = answers(book, run, quest);
    let to = want.clone().unwrap_or_else(|| {
        let st = run.quests.get(quest).map_or(Status::Skipped, |e| e.status);
        [("status".to_owned(), format!("{st:?}").to_lowercase())].into()
    });
    // Compare only the choices the registry records (a choice it does not know is not checked).
    let Some(want) = want else {
        return Baked::Fixed { from: enc.clone(), to, why: "the template already has this quest done; a quest cannot be un-done after the fact".into() };
    };
    let differing: Vec<(&String, &String)> = enc.iter().filter(|(c, o)| want.get(*c) != Some(*o)).filter_map(|(c, _)| want.get(c).map(|w| (c, w))).collect();
    if differing.is_empty() {
        return Baked::Same;
    }
    // `fixed` keys: `quest` (any change), `quest.choice` (that choice), `quest.choice=option` (changing to that option).
    let why: Vec<String> = differing
        .iter()
        .filter_map(|(c, o)| {
            t.fixed.get(quest).or_else(|| t.fixed.get(&format!("{quest}.{c}"))).or_else(|| t.fixed.get(&format!("{quest}.{c}={o}"))).map(|w| format!("{c}={o}: {w}"))
        })
        .collect();
    if why.is_empty() {
        Baked::Rewrite { from: enc.clone(), to }
    } else {
        Baked::Fixed { from: enc.clone(), to, why: why.join("; ") }
    }
}

/// The quest a `fixed` key names (`quest`, `quest.choice` or `quest.choice=option`).
fn fixed_key_quest(key: &str) -> &str {
    key.split(['.', '=']).next().unwrap_or(key)
}

/// Pick the template for a run: the latest template on the line at or before the stop quest whose
/// earlier answers are the player's or can be rewritten, and whose file is found (`find`). `Err` = no
/// template fits (the app then asks for a file).
pub fn choose(book: &Questbook, reg: &Registry, run: &Run, find: &dyn Fn(&Template) -> Option<PathBuf>) -> Result<Choice, String> {
    let stop = stop_quest(book, run).ok_or("the run completes no quest, so there is no stop point")?;
    let stop_line = reg
        .line_of(&stop)
        .ok_or_else(|| format!("the stop quest `{stop}` has no place on the template line ({REGISTRY_FILE})"))?;
    let stop_order = book.get(&stop).map_or(u32::MAX, |q| q.order);
    let mut passed_over = Vec::new();
    for t in reg.latest_first().into_iter().filter(|t| t.line <= stop_line) {
        let path = find(t);
        let baked: Vec<BakedQuest> = book
            .quests
            .iter()
            .filter(|q| !t.quests.contains(&q.id) && reg.line_of(&q.id).is_some_and(|l| l <= t.line))
            .map(|q| BakedQuest { quest: q.id.clone(), action: baked_action(book, run, t, &q.id) })
            .collect();
        let fixed: Vec<String> = baked
            .iter()
            .filter_map(|b| match &b.action {
                Baked::Fixed { why, .. } => Some(format!("{}: {why}", b.quest)),
                _ => None,
            })
            .collect();
        if !fixed.is_empty() {
            passed_over.push(format!("{} (line {}): {}", t.id, t.line, fixed.join("; ")));
            continue;
        }
        if path.is_none() {
            passed_over.push(format!("{} (line {}): {}.sav not found in the template folders", t.id, t.line, t.stem));
            continue;
        }
        let applied: Vec<String> = book.quests.iter().filter(|q| t.quests.contains(&q.id) && q.order <= stop_order).map(|q| q.id.clone()).collect();
        let beyond: Vec<String> = book
            .quests
            .iter()
            .filter(|q| q.order <= stop_order && run.quests.get(&q.id).is_some_and(|e| e.status != Status::Skipped))
            .filter(|q| reg.line_of(&q.id).is_some_and(|l| l > t.line))
            .map(|q| q.id.clone())
            .collect();
        let mut notes = Vec::new();
        for b in &baked {
            if b.action == Baked::Unchecked && run.quests.contains_key(&b.quest) {
                notes.push(format!("{}: the template's answer is not recorded; the save keeps whatever the template holds", b.quest));
            }
        }
        return Ok(Choice { template: t.clone(), path, applied, baked, beyond, passed_over, notes });
    }
    Err(format!(
        "no registered template fits this run (stop at `{stop}`, line {stop_line}){}",
        if passed_over.is_empty() { String::new() } else { format!(": {}", passed_over.join(" | ")) }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::questbook::{Choice as QChoice, ChoiceOption, Kind, Quest, Region};
    use crate::run::QuestEntry;

    fn quest(id: &str, order: u32, choices: &[(&str, &[&str])]) -> Quest {
        Quest {
            id: id.into(),
            title: id.into(),
            act: 2,
            order,
            kind: Kind::Secondary,
            region: Region::Velen,
            level: 1,
            requires: vec![],
            excludes: vec![],
            image: None,
            summary: "s".into(),
            started_effects: None,
            done_effects: None,
            choices: choices
                .iter()
                .map(|(c, opts)| QChoice {
                    id: (*c).into(),
                    prompt: "?".into(),
                    default: opts[0].into(),
                    options: opts.iter().map(|o| ChoiceOption { id: (*o).into(), text: "t".into(), effects: None }).collect(),
                })
                .collect(),
            effects: BTreeMap::new(),
            file: PathBuf::new(),
        }
    }

    fn book() -> Questbook {
        Questbook::from_quests(vec![quest("a", 10, &[("x", &["a1", "a2"])]), quest("b", 20, &[("y", &["b1", "b2"])]), quest("c", 30, &[]), quest("d", 40, &[])])
    }

    fn reg() -> Registry {
        toml::from_str(
            r#"
[[template]]
id = "ta"
stem = "TA"
line = 1
quests = ["a"]
[[template]]
id = "tb"
stem = "TB"
line = 2
quests = ["b"]
encodes = { a = { x = "a1" } }
[[template]]
id = "tc"
stem = "TC"
line = 3
quests = ["c"]
encodes = { a = { x = "a1" }, b = { y = "b1" } }
fixed = { b = "played out" }
[[line]]
quest = "d"
line = 4
"#,
        )
        .unwrap()
    }

    fn run(stop: &str, a: &str, b: &str) -> Run {
        let mut r = Run { stop_after: Some(stop.into()), ..Run::default() };
        r.quests.insert("a".into(), QuestEntry { status: Status::Completed, answers: [("x".to_string(), a.to_string())].into() });
        r.quests.insert("b".into(), QuestEntry { status: Status::Completed, answers: [("y".to_string(), b.to_string())].into() });
        r.quests.insert("c".into(), QuestEntry { status: Status::Completed, answers: BTreeMap::new() });
        r
    }

    #[test]
    fn picks_the_latest_template_at_or_before_the_stop() {
        let all = |t: &Template| Some(PathBuf::from(format!("{}.sav", t.stem)));
        let c = choose(&book(), &reg(), &run("c", "a1", "b1"), &all).unwrap();
        assert_eq!((c.template.id.as_str(), c.applied.clone()), ("tc", vec!["c".to_string()]));
        assert!(c.baked.iter().all(|b| b.action == Baked::Same), "{:?}", c.baked);
        // Stop earlier: the earlier window; c is beyond the stop and not reported.
        let c = choose(&book(), &reg(), &run("b", "a1", "b1"), &all).unwrap();
        assert_eq!(c.template.id, "tb");
        assert!(c.beyond.is_empty());
        // An earlier answer that differs and can be rewritten: same template, a rewrite.
        let c = choose(&book(), &reg(), &run("c", "a2", "b1"), &all).unwrap();
        assert_eq!(c.template.id, "tc");
        assert_eq!(c.rewrites().count(), 1);
        // A fixed one: fall back to the latest template before it; the later quest is beyond.
        let c = choose(&book(), &reg(), &run("c", "a1", "b2"), &all).unwrap();
        assert_eq!(c.template.id, "tb");
        assert_eq!(c.beyond, ["c"]);
        assert_eq!(c.passed_over.len(), 1);
        // A missing file: passed over too.
        let only_a = |t: &Template| (t.id == "ta").then(|| PathBuf::from("TA.sav"));
        let c = choose(&book(), &reg(), &run("c", "a1", "b1"), &only_a).unwrap();
        assert_eq!((c.template.id.as_str(), c.passed_over.len()), ("ta", 2));
        let none = |_: &Template| None;
        assert!(choose(&book(), &reg(), &run("c", "a1", "b1"), &none).is_err());
    }

    #[test]
    fn lint_catches_bad_names() {
        let b = book();
        assert!(reg().lint(&b).iter().all(|d| !d.is_error()), "{:?}", reg().lint(&b));
        let mut r = reg();
        r.templates[1].encodes.insert("a".into(), [("x".to_string(), "nope".to_string())].into());
        r.templates[2].quests.push("a".into());
        r.line.push(LineQuest { quest: "zz".into(), line: 9, note: String::new() });
        let errs: Vec<String> = r.lint(&b).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
        assert!(errs.iter().any(|m| m.contains("no option `nope`")), "{errs:?}");
        assert!(errs.iter().any(|m| m.contains("more than one window")), "{errs:?}");
        assert!(errs.iter().any(|m| m.contains("unknown quest `zz`")), "{errs:?}");
    }
}
