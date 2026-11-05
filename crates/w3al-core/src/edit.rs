//! Edit mode: open one of the player's own saves, read where it stands in the questbook and change it
//! into a NEW save (never the opened one).
//!
//! * [`Engine::plan`] reads the save: for every registered decision window the facts of every answer
//!   combination, exported on the window's own template (`chain::scan_window`, the same builds
//!   `w3al template-scan` uses to derive `encodes`), are matched against the save's facts. The window is
//!   *decided* (the best combination is clear: its answers are the save's), *not reached* (none of the facts
//!   the window writes are in the save) or *unknown* (mixed: nothing matches well, or combinations tie; a
//!   choice whose answers tie stays unknown). A save that is exactly a registered template also gets the
//!   registry's `encodes` for what the facts cannot tell. What can be changed follows `chain.rs`: a decided
//!   decision is rewritten after the fact unless the registry's `fixed` rules of the template the save sits at
//!   (the first window after the last decided one) say its consequence already played out; the first window
//!   not reached yet is offered with its own synth effects when a dry run on this save builds.
//! * [`Engine::apply`] builds the new save with `chain::build_story` (rewrite of earlier windows, then the
//!   window's effects, then the start kit), writes it with `create_new` next to the chosen folder (sidecars
//!   copied) and re-reads it. The opened file is only read; its bytes are compared once more at the end.
//!
//! Design and evidence: docs/plan/story-chain.md (sections 2 and 3), docs/findings/character-inventory.md.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use w3graph::frontier;
use w3graph::quest::Library;
use w3save::facts::FactTable;
use w3save::{character, Container, Document, Severity};

use crate::chain::{self, Sources};
use crate::export::{self, ExportError, ExportOptions};
use crate::questbook::Questbook;
use crate::run::{QuestEntry, Run, Status};
use crate::start_kit::{self, Gear, StartKit};
use crate::templates::{self, Baked, BakedQuest, Choice, Registry, Template};

type Answers = BTreeMap<String, String>;
type WindowAnswers = BTreeMap<String, Answers>;

/// Bump when the scan's shape or meaning changes (invalidates the disk cache).
const CACHE_VERSION: u32 = 1;
/// Refuse windows with more answer combinations than this (the scan builds each one).
const MAX_COMBOS: usize = 64;

// ---------------------------------------------------------------------------------------------
// The scan: facts of every answer combination of every window (cached on disk)

/// Facts one answer combination of a window writes on the window's own template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandFacts {
    pub answers: WindowAnswers,
    /// `None` = the combination does not export.
    pub facts: Option<BTreeMap<String, i64>>,
}

/// One window's scan: the window template's own facts (`base`) and every combination's.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowScan {
    pub id: String,
    pub base: Option<BTreeMap<String, i64>>,
    pub cands: Vec<CandFacts>,
    /// Why the window could not be scanned (template file missing, too many combinations).
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    fingerprint: String,
    scans: Vec<WindowScan>,
}

fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x100_0000_01b3);
    }
}

/// What the scan depends on: the questbook files, the registry and the window template files (name, size, time).
fn fingerprint(book: &Questbook, reg: &Registry, dirs: &[PathBuf], opts: &ExportOptions) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    fnv(&mut h, &CACHE_VERSION.to_le_bytes());
    // Donor folders decide which graph effects can be built (a window without its donors scans to nothing).
    for d in &opts.donor_dirs {
        fnv(&mut h, d.to_string_lossy().as_bytes());
        let n = fs::read_dir(d).map_or(0, |r| r.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "sav")).count());
        fnv(&mut h, &n.to_le_bytes());
    }
    fnv(&mut h, &[u8::from(opts.allow_name_remap)]);
    for q in &book.quests {
        fnv(&mut h, q.id.as_bytes());
        match fs::read(&q.file) {
            Ok(b) => fnv(&mut h, &b),
            // Quests built in memory (tests) carry no file: their serialization stands in.
            Err(_) => fnv(&mut h, serde_json::to_string(q).unwrap_or_default().as_bytes()),
        }
    }
    fnv(&mut h, serde_json::to_string(reg).unwrap_or_default().as_bytes());
    for t in &reg.templates {
        fnv(&mut h, t.stem.as_bytes());
        if let Some(p) = templates::find_file(dirs, &t.stem) {
            if let Ok(m) = fs::metadata(&p) {
                fnv(&mut h, &m.len().to_le_bytes());
                if let Some(d) = m.modified().ok().and_then(|x| x.duration_since(std::time::UNIX_EPOCH).ok()) {
                    fnv(&mut h, &d.as_secs().to_le_bytes());
                }
            }
        }
    }
    format!("{h:016x}")
}

fn facts_map(doc: &Document) -> Result<BTreeMap<String, i64>, String> {
    let t = doc.facts_table().map_err(|e| e.to_string())?;
    Ok(t.facts().iter().filter(|f| !f.records.is_empty() && !frontier::is_noise_fact(&f.name)).map(|f| (f.name.clone(), f.sum())).collect())
}

fn fact_value(t: &FactTable, name: &str) -> Option<i64> {
    t.get(name).filter(|f| !f.records.is_empty()).map(w3save::facts::Fact::sum)
}

fn doc_of(bytes: &[u8]) -> Result<Document, String> {
    let c = Container::read(bytes).map_err(|e| e.to_string())?;
    Document::parse(&c.payload, c.data_start).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// The engine

/// Everything edit mode needs, loaded once (the quest graphs of the install and the scans are the slow parts).
pub struct Engine {
    pub book: Questbook,
    pub registry: Registry,
    pub template_dirs: Vec<PathBuf>,
    pub opts: ExportOptions,
    lib: Library,
    /// Registry templates in line order with their scans.
    scans: Vec<(Template, WindowScan)>,
}

impl Engine {
    /// Load the quest graphs from `opts.game_dir` and scan every window (or read the scan from `cache`, a
    /// JSON file that is rewritten when the questbook, registry or template files changed). `progress` is
    /// told what is being built.
    pub fn load(book: Questbook, registry: Registry, template_dirs: Vec<PathBuf>, opts: ExportOptions, cache: Option<&Path>, progress: &dyn Fn(&str)) -> Result<Self, String> {
        progress("loading the game's quest graphs");
        let lib = chain::load_library(&opts).map_err(|e| e.to_string())?;
        let fp = fingerprint(&book, &registry, &template_dirs, &opts);
        let mut order: Vec<Template> = registry.templates.clone();
        order.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.id.cmp(&b.id)));
        let cached: Option<Vec<WindowScan>> = cache
            .and_then(|p| fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok())
            .filter(|c| c.fingerprint == fp && c.scans.len() == order.len() && c.scans.iter().zip(&order).all(|(s, t)| s.id == t.id))
            .map(|c| c.scans);
        let scans = match cached {
            Some(s) => s,
            None => {
                let src = Sources { book: &book, registry: &registry, template_dirs: template_dirs.clone(), opts: opts.clone() };
                let mut out = Vec::new();
                for t in &order {
                    progress(&format!("reading the answers of {} (first time only, then cached)", t.id));
                    out.push(scan_one(&src, &lib, t));
                }
                if let Some(p) = cache {
                    if let Ok(text) = serde_json::to_vec(&CacheFile { fingerprint: fp, scans: out.clone() }) {
                        if let Some(d) = p.parent() {
                            let _ = fs::create_dir_all(d);
                        }
                        let _ = fs::write(p, text); // a cache that cannot be written only costs time next run
                    }
                }
                out
            }
        };
        Ok(Self { book, registry, template_dirs, opts, lib, scans: order.into_iter().zip(scans).collect() })
    }

    fn src(&self) -> Sources<'_> {
        Sources { book: &self.book, registry: &self.registry, template_dirs: self.template_dirs.clone(), opts: self.opts.clone() }
    }
}

fn scan_one(src: &Sources, lib: &Library, t: &Template) -> WindowScan {
    let fail = |e: String| WindowScan { id: t.id.clone(), base: None, cands: Vec::new(), error: Some(e) };
    let cands = match chain::scan_window(src, Some(lib), t, MAX_COMBOS) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let base = match templates::find_file(&src.template_dirs, &t.stem).ok_or_else(|| format!("{}.sav not found", t.stem)).and_then(|p| fs::read(p).map_err(|e| e.to_string())).and_then(|b| doc_of(&b)).and_then(|d| facts_map(&d)) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    WindowScan { id: t.id.clone(), base: Some(base), cands: cands.into_iter().map(|c| CandFacts { answers: c.answers, facts: c.facts }).collect(), error: None }
}

// ---------------------------------------------------------------------------------------------
// Classifying a window against a save

#[derive(Debug, Clone, PartialEq, Eq)]
enum WinState {
    Decided,
    NotReached,
    Unknown,
}

#[derive(Debug, Clone)]
struct WinClass {
    state: WinState,
    why: String,
    /// Deciding facts that differ from the best combination (presence), and the runner-up's count.
    mismatches: usize,
    next_best: Option<usize>,
    /// quest -> choice -> option, only the choices the facts settle.
    known: WindowAnswers,
}

fn unknown(why: impl Into<String>) -> WinClass {
    WinClass { state: WinState::Unknown, why: why.into(), mismatches: 0, next_best: None, known: WindowAnswers::new() }
}

#[cfg(test)]
fn classify(scan: &WindowScan, tf: &FactTable) -> WinClass {
    classify_after(scan, tf, &BTreeSet::new())
}

/// [`classify`], where `earlier` = the facts an earlier window's template already holds: they are no evidence
/// that this window ran (a save at Skellige holds `triss_dd_start`, which the romance window's template lacks
/// and its walks set again; 2026-11-06, a w4_undvik save read as past W5-W7).
fn classify_after(scan: &WindowScan, tf: &FactTable, earlier: &BTreeSet<String>) -> WinClass {
    if let Some(e) = &scan.error {
        return unknown(format!("the window's template could not be read ({e})"));
    }
    let Some(base) = &scan.base else { return unknown("no baseline") };
    let cands: Vec<(&WindowAnswers, &BTreeMap<String, i64>)> = scan.cands.iter().filter_map(|c| c.facts.as_ref().map(|f| (&c.answers, f))).collect();
    if cands.is_empty() {
        return unknown("no answer combination of the window exports");
    }
    let mut names: BTreeSet<&str> = base.keys().map(String::as_str).collect();
    for (_, f) in &cands {
        names.extend(f.keys().map(String::as_str));
    }
    // Positive evidence that the window ran: the save holds a fact some combination writes that the window's
    // template lacks (later play may have removed others, so one is enough).
    let evidence = names
        .iter()
        .filter(|n| base.get(**n).is_none() && !earlier.contains(**n) && cands.iter().any(|(_, f)| f.contains_key(**n)) && fact_value(tf, n).is_some())
        .count();
    if evidence == 0 {
        return WinClass { state: WinState::NotReached, why: "none of the facts the window writes is in the save".into(), mismatches: 0, next_best: None, known: WindowAnswers::new() };
    }
    // The facts that tell the combinations apart (as `w3al template-scan`: the facts that vary between them).
    // Facts every combination writes are the window's aftermath and later play consumes many of them: no evidence
    // for the answers.
    let varying: Vec<&str> = names.into_iter().filter(|n| cands.iter().any(|(_, f)| f.get(*n) != cands[0].1.get(*n))).collect();
    let score = |f: &BTreeMap<String, i64>| -> (usize, usize) {
        let (mut pm, mut vm) = (0, 0);
        for n in &varying {
            let (want, here) = (f.get(*n).copied(), fact_value(tf, n));
            if want.is_some() != here.is_some() {
                pm += 1;
            } else if want != here {
                vm += 1;
            }
        }
        (pm, vm)
    };
    let mut scored: Vec<((usize, usize), &WindowAnswers)> = cands.iter().map(|(a, f)| (score(f), *a)).collect();
    scored.sort_by_key(|x| x.0);
    let best = scored[0].0;
    let limit = varying.len().div_ceil(3).max(3);
    if best.0 > limit {
        return WinClass { state: WinState::Unknown, why: format!("mixed: the best answer still differs in {} of {} deciding facts", best.0, varying.len()), mismatches: best.0, next_best: scored.get(1).map(|x| x.0 .0), known: WindowAnswers::new() };
    }
    let tied: Vec<&WindowAnswers> = scored.iter().take_while(|x| x.0 == best).map(|x| x.1).collect();
    let next_best = scored.iter().find(|x| x.0 != best).map(|x| x.0 .0);
    // A choice is known when every tied combination agrees on it.
    let mut known = WindowAnswers::new();
    for (q, a) in tied[0] {
        let entry = known.entry(q.clone()).or_default();
        for (c, o) in a {
            if tied.iter().all(|t| t.get(q).and_then(|m| m.get(c)) == Some(o)) {
                entry.insert(c.clone(), o.clone());
            }
        }
    }
    let why = if tied.len() > 1 { format!("{} answer combinations tie; only the choices they agree on are known", tied.len()) } else { String::new() };
    WinClass { state: WinState::Decided, why, mismatches: best.0, next_best, known }
}

// ---------------------------------------------------------------------------------------------
// The plan

#[derive(Debug, Clone, Serialize)]
pub struct OptionView {
    pub id: String,
    /// The option can be chosen on this save (a rewrite is possible, or the window is still ahead).
    pub allowed: bool,
    pub why: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChoiceView {
    pub id: String,
    /// The save's answer; `None` = not read from the save (undecided, or the facts do not tell).
    pub current: Option<String>,
    /// `facts` (matched against the save), `registry` (the save is a registered template and the registry
    /// records it), `none`.
    pub source: &'static str,
    /// `rewrite` (a decided answer that can be rewritten), `synth` (not decided yet: the window's own effects
    /// apply on this save), `readonly`.
    pub mode: &'static str,
    pub options: Vec<OptionView>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuestView {
    pub quest: String,
    pub window: Option<String>,
    /// `done`, `active` (the next window: the save heads for it), `not_reached`, `unknown`, `untracked`.
    pub status: &'static str,
    pub note: String,
    pub choices: Vec<ChoiceView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowView {
    pub id: String,
    pub line: u32,
    pub quests: Vec<String>,
    /// `decided`, `active`, `not_reached`, `unknown`.
    pub state: &'static str,
    pub why: String,
    pub mismatches: usize,
    pub next_best: Option<usize>,
    /// For the active window: does a dry run of its default answers build on this save?
    pub buildable: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KitItemView {
    pub name: String,
    pub quantity: u16,
    /// `gear`, `charged` or `stack`.
    pub form: &'static str,
    pub carried: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KitTable {
    pub from_level: u32,
    pub items: Vec<KitItemView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CharacterView {
    pub level: i32,
    pub max_level: i32,
    pub crowns: Option<u32>,
    pub items: usize,
    /// The save is a New Game+ run (`w3save::newgameplus`); `None` = the state could not be read.
    pub new_game_plus: Option<bool>,
    /// Ability points: free, spent on learned skills.
    pub skill_free: i32,
    pub skill_used: i32,
    /// The recommended kit per level band (the band is the highest `from_level` at or below the target level).
    pub kit: Vec<KitTable>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub stem: String,
    /// The registered template the save sits at (its `fixed` rules apply), when any.
    pub position: Option<String>,
    /// The save IS this registered template (matched by file name).
    pub is_template: Option<String>,
    pub windows: Vec<WindowView>,
    pub quests: Vec<QuestView>,
    pub character: Option<CharacterView>,
    pub character_error: Option<String>,
    pub notes: Vec<String>,
}

struct Analysis {
    classes: Vec<WinClass>,
    /// Index (in `Engine::scans`) of the active window.
    pending: Option<usize>,
    /// Index of the template whose `fixed` rules govern (the save's position).
    position: usize,
    exact: Option<usize>,
    /// quest -> choice -> option currently held (facts first, registry for an exact template).
    current: WindowAnswers,
    /// quest -> choice -> source tag
    source: BTreeMap<(String, String), &'static str>,
    notes: Vec<String>,
}

fn kit_item_view(k: &start_kit::KitItem, have: &character::Character) -> KitItemView {
    use w3save::character::ItemForm;
    KitItemView {
        name: k.name.to_owned(),
        quantity: k.quantity,
        form: match k.form {
            ItemForm::Gear { .. } => "gear",
            ItemForm::Charged { .. } => "charged",
            ItemForm::Stack => "stack",
        },
        carried: have.items.iter().find(|i| i.name == k.name).map(|i| i.quantity),
    }
}

impl Engine {
    fn analyse(&self, doc: &Document, stem: &str) -> Result<Analysis, String> {
        let tf = doc.facts_table().map_err(|e| e.to_string())?;
        let mut earlier: BTreeSet<String> = BTreeSet::new();
        let mut classes: Vec<WinClass> = Vec::with_capacity(self.scans.len());
        for (_, s) in &self.scans {
            classes.push(classify_after(s, &tf, &earlier));
            earlier.extend(s.base.iter().flat_map(|b| b.keys().cloned()));
        }
        let exact = self.scans.iter().position(|(t, _)| t.stem == stem);
        let mut notes = Vec::new();
        if let Some(i) = exact {
            notes.push(format!("this save is the registered template {} (line {}): its recorded answers fill in what the facts cannot tell", self.scans[i].0.id, self.scans[i].0.line));
        }
        if let Some(e) = exact {
            // The save IS this template: its own window and everything later are ahead; what lies before it is
            // what the facts say, and where they do not settle a window the registry's `encodes` do.
            let enc = &self.scans[e].0.encodes;
            for (i, ((t, _), c)) in self.scans.iter().zip(classes.iter_mut()).enumerate() {
                if i >= e {
                    *c = WinClass { state: WinState::NotReached, why: if i == e { "the save is parked right before this window".into() } else { String::new() }, mismatches: 0, next_best: None, known: WindowAnswers::new() };
                } else if c.state != WinState::Decided && t.quests.iter().all(|q| enc.contains_key(q)) {
                    let was = format!("{:?}{}", c.state, if c.why.is_empty() { String::new() } else { format!(": {}", c.why) });
                    *c = WinClass { state: WinState::Decided, why: format!("from the registry (the facts gave: {was})"), mismatches: c.mismatches, next_best: c.next_best, known: WindowAnswers::new() };
                }
            }
        }
        // A save past a window cannot be before an earlier one: those are facts later play removed.
        let last_decided = classes.iter().rposition(|c| c.state == WinState::Decided);
        if let Some(ld) = last_decided {
            for c in classes.iter_mut().take(ld).filter(|c| c.state == WinState::NotReached) {
                *c = unknown("the save is past this window, but the facts it wrote are gone");
            }
        }
        let after = last_decided.map_or(0, |i| i + 1);
        let position = exact.unwrap_or_else(|| if after < self.scans.len() { after } else { self.scans.len().saturating_sub(1) });
        // The active window: the first one not reached after the last decided one (the exact template's own window).
        let pending = exact.or_else(|| (after..self.scans.len()).find(|i| classes[*i].state == WinState::NotReached));
        let mut current = WindowAnswers::new();
        let mut source = BTreeMap::new();
        for ((t, _), c) in self.scans.iter().zip(&classes) {
            if c.state != WinState::Decided {
                continue;
            }
            for q in &t.quests {
                let e = current.entry(q.clone()).or_default();
                if let Some(k) = c.known.get(q) {
                    for (cid, o) in k {
                        e.insert(cid.clone(), o.clone());
                        source.insert((q.clone(), cid.clone()), "facts");
                    }
                }
            }
        }
        if let Some(i) = exact {
            for (q, enc) in &self.scans[i].0.encodes {
                let e = current.entry(q.clone()).or_default();
                for (cid, o) in enc {
                    match e.get(cid) {
                        Some(f) if f == o => continue,
                        // `encodes` is derived with the scan and corrected by hand where the scan cannot express the
                        // answer (vigi, the Cerys trial): for the template itself the registry wins.
                        Some(f) => notes.push(format!("{q}.{cid}: the registry records `{o}` for this template; the facts alone would say `{f}` (a hand-set value the scan cannot express)")),
                        None => {}
                    }
                    e.insert(cid.clone(), o.clone());
                    source.insert((q.clone(), cid.clone()), "registry");
                }
            }
        }
        Ok(Analysis { classes, pending, position, exact, current, source, notes })
    }

    /// The registered template whose `fixed` rules govern, with `encodes` replaced by what the save holds.
    fn pseudo_template(&self, an: &Analysis) -> Template {
        let mut t = self.scans[an.position].0.clone();
        t.encodes = an.current.clone();
        t
    }

    fn window_index(&self, quest: &str) -> Option<usize> {
        self.scans.iter().position(|(t, _)| t.quests.iter().any(|q| q == quest))
    }

    /// Can `quest.choice` of a decided window be changed to `option`? (`Err` = why not.)
    fn rewrite_check(&self, an: &Analysis, pseudo: &Template, quest: &str, choice: &str, option: &str) -> Result<(), String> {
        let wi = self.window_index(quest).ok_or("the quest is not in a decision window")?;
        let (w, scan) = &self.scans[wi];
        if scan.error.is_some() {
            return Err(format!("the window template of {} is needed to rebuild the decision but is not available", w.id));
        }
        if w.line >= pseudo.line {
            return Err("no later registered template records what the save played after this decision, so it cannot be rewritten safely".into());
        }
        let cur = an.current.get(quest).cloned().unwrap_or_default();
        if !cur.contains_key(choice) {
            return Err("the save's answer to this decision is not known".into());
        }
        let mut want = cur;
        want.insert(choice.to_owned(), option.to_owned());
        let mut run = Run::default();
        run.quests.insert(quest.to_owned(), QuestEntry { status: Status::Completed, answers: want });
        match templates::baked_action(&self.book, &run, pseudo, quest) {
            Baked::Rewrite { .. } | Baked::Same => Ok(()),
            Baked::Fixed { why, .. } => Err(why),
            Baked::Unchecked => Err("not recorded for this save's position".into()),
        }
    }

    /// Read a save and say where it stands and what can change. `check_pending` runs a dry build of the
    /// active window's default answers (a second or two).
    pub fn plan(&self, bytes: &[u8], stem: &str, check_pending: bool) -> Result<Plan, String> {
        let container = Container::read(bytes).map_err(|e| e.to_string())?;
        let damage: Vec<String> = container.diagnostics().into_iter().filter(|i| i.severity == Severity::Error).map(|i| i.message).collect();
        if !damage.is_empty() {
            return Err(format!("this save is damaged: {}", damage.join("; ")));
        }
        let doc = Document::parse(&container.payload, container.data_start).map_err(|e| e.to_string())?;
        let an = self.analyse(&doc, stem)?;
        let pseudo = self.pseudo_template(&an);
        let buildable = match (an.pending, check_pending) {
            (Some(p), true) => Some(self.dry_build(bytes, &self.scans[p].0).map(|_| ())),
            _ => None,
        };

        let mut windows = Vec::new();
        for (i, ((t, _), c)) in self.scans.iter().zip(&an.classes).enumerate() {
            let is_pending = an.pending == Some(i);
            windows.push(WindowView {
                id: t.id.clone(),
                line: t.line,
                quests: t.quests.clone(),
                state: match (&c.state, is_pending) {
                    (WinState::NotReached, true) => "active",
                    (WinState::NotReached, false) => "not_reached",
                    (WinState::Decided, _) => "decided",
                    (WinState::Unknown, _) => "unknown",
                },
                why: c.why.clone(),
                mismatches: c.mismatches,
                next_best: c.next_best,
                buildable: if is_pending { buildable.as_ref().map(Result::is_ok) } else { None },
            });
        }

        let mut quests = Vec::new();
        for q in &self.book.quests {
            let wi = self.window_index(&q.id);
            let cur = an.current.get(&q.id);
            let status: &'static str;
            let note: String;
            match wi {
                None => {
                    // A quest on the line without a window template: only a registered template records it.
                    if cur.is_some() {
                        status = "done";
                        note = "recorded by the registry for this template; there is no window template to rewrite it from".into();
                    } else {
                        status = "untracked";
                        note = self
                            .registry
                            .line
                            .iter()
                            .find(|l| l.quest == q.id && !l.note.is_empty())
                            .map_or_else(|| "no window template: this version cannot tell whether the save is past it".to_owned(), |l| l.note.clone());
                    }
                }
                Some(i) => {
                    let c = &an.classes[i];
                    status = match (&c.state, an.pending == Some(i)) {
                        (WinState::Decided, _) => "done",
                        (WinState::NotReached, true) => "active",
                        (WinState::NotReached, false) => "not_reached",
                        (WinState::Unknown, _) => "unknown",
                    };
                    note = c.why.clone();
                }
            }
            let mut choices = Vec::new();
            for ch in &q.choices {
                let current = cur.and_then(|m| m.get(&ch.id)).cloned();
                let source = an.source.get(&(q.id.clone(), ch.id.clone())).copied().unwrap_or("none");
                let (mode, options, reason) = match (wi, status) {
                    (Some(_), "done") | (None, "done") => {
                        if wi.is_none() {
                            ("readonly", self.all_options(ch, false, None), Some("there is no window template to rewrite this decision from".to_owned()))
                        } else if current.is_none() {
                            ("readonly", self.all_options(ch, false, None), Some("the save's facts do not tell this answer".to_owned()))
                        } else {
                            let mut opts = Vec::new();
                            let mut firsterr = None;
                            for o in &ch.options {
                                if Some(&o.id) == current.as_ref() {
                                    opts.push(OptionView { id: o.id.clone(), allowed: true, why: None });
                                    continue;
                                }
                                match self.rewrite_check(&an, &pseudo, &q.id, &ch.id, &o.id) {
                                    Ok(()) => opts.push(OptionView { id: o.id.clone(), allowed: true, why: None }),
                                    Err(w) => {
                                        firsterr.get_or_insert(w.clone());
                                        opts.push(OptionView { id: o.id.clone(), allowed: false, why: Some(w) });
                                    }
                                }
                            }
                            let any = opts.iter().any(|o| o.allowed && Some(&o.id) != current.as_ref());
                            (if any { "rewrite" } else { "readonly" }, opts, if any { None } else { firsterr })
                        }
                    }
                    (Some(_), "active") => match &buildable {
                        Some(Err(e)) => ("readonly", self.all_options(ch, false, None), Some(format!("this save does not sit at the start of the window, or its effects are not finished: {e}"))),
                        // Built fine, or not checked (the caller asked for no dry run).
                        _ => ("synth", self.all_options(ch, true, None), None),
                    },
                    (_, "unknown") => ("readonly", self.all_options(ch, false, None), Some(note.clone())),
                    // Not reached, but the save's template completes it late (`Template::late`, `[effects.*.late]`).
                    (Some(i), "not_reached") if pseudo.late.contains(&q.id) && self.scans[i].0.line > pseudo.line => {
                        ("late", self.all_options(ch, true, None), Some("not reached in this save; it can be completed late with your answer (the quest and its prerequisites are auto-completed)".to_owned()))
                    }
                    _ => ("readonly", self.all_options(ch, false, None), Some("this part of the story is not reached in the save".to_owned())),
                };
                choices.push(ChoiceView { id: ch.id.clone(), current, source, mode, options, reason });
            }
            quests.push(QuestView { quest: q.id.clone(), window: wi.map(|i| self.scans[i].0.id.clone()), status, note, choices });
        }

        let (character, character_error) = match character::read(&doc) {
            Ok(have) => {
                let mut kit = Vec::new();
                for from in [1u32, 11, 15, 18, 26, 30, 34] {
                    kit.push(KitTable { from_level: from, items: start_kit::kit_for(from).iter().map(|k| kit_item_view(k, &have)).collect() });
                }
                (Some(CharacterView { level: have.level.level, max_level: have.level.last_custom_level, crowns: have.crowns(), items: have.items.len(), new_game_plus: w3save::newgameplus::read(&doc).ok().map(|s| s.is_new_game_plus()), skill_free: have.level.skill.free, skill_used: have.level.skill.used, kit }), None)
            }
            Err(e) => (None, Some(e.to_string())),
        };
        let mut notes = an.notes.clone();
        if let Some(Err(e)) = &buildable {
            notes.push(format!("the next window ({}) cannot be offered on this save: {e}", self.scans[an.pending.unwrap_or(0)].0.id));
        }
        Ok(Plan {
            stem: stem.to_owned(),
            position: Some(self.scans[an.position].0.id.clone()),
            is_template: an.exact.map(|i| self.scans[i].0.id.clone()),
            windows,
            quests,
            character,
            character_error,
            notes,
        })
    }

    fn all_options(&self, ch: &crate::questbook::Choice, allowed: bool, why: Option<String>) -> Vec<OptionView> {
        ch.options.iter().map(|o| OptionView { id: o.id.clone(), allowed, why: why.clone() }).collect()
    }

    /// Build the window's quests with their default answers on this save, in memory: does the window fit?
    fn dry_build(&self, bytes: &[u8], w: &Template) -> Result<(), String> {
        let mut run = Run::default();
        for q in &w.quests {
            run.quests.insert(q.clone(), QuestEntry { status: Status::Completed, answers: Answers::new() });
        }
        let res = crate::resolve::resolve_only(&self.book, &run, |q| w.quests.contains(&q.id));
        if res.has_errors() {
            return Err(res.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect::<Vec<_>>().join("; "));
        }
        let mut opts = self.opts.clone();
        opts.start_kit = None;
        let donors = export::load_donors(&res, &opts.donor_dirs).map_err(|e| e.to_string())?;
        export::build_with_sources(bytes, &res, &opts, &donors, Some(&self.lib)).map(|_| ()).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------------------------
// Apply

/// What the player asked to change.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Changes {
    /// quest -> choice -> new option.
    #[serde(default)]
    pub answers: WindowAnswers,
    #[serde(default)]
    pub level: Option<u32>,
    #[serde(default)]
    pub crowns: Option<u32>,
    /// Add the recommended kit.
    #[serde(default)]
    pub gear: bool,
    /// Single items of the recommended table, by name.
    #[serde(default)]
    pub items: Vec<String>,
    /// Turn a New Game+ save into a normal game (`start_kit::StartKit::new_game_plus = Some(false)`).
    #[serde(default)]
    pub no_new_game_plus: bool,
    /// Ability points in total, learned + free (`start_kit::StartKit::skill_points`).
    #[serde(default)]
    pub skill_points: Option<u32>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.answers.values().all(Answers::is_empty) && self.level.is_none() && self.crowns.is_none() && !self.gear && self.items.is_empty() && !self.no_new_game_plus && self.skill_points.is_none()
    }
}

#[derive(Debug, Serialize)]
pub struct EditOutcome {
    pub sav: PathBuf,
    pub sidecars: Vec<PathBuf>,
    /// One line per rewritten window, applied effect and kit change.
    pub notes: Vec<String>,
    /// Quests whose decision was rewritten.
    pub rewritten: Vec<String>,
    /// Quests of the active window that were applied.
    pub applied: Vec<String>,
    pub verified: String,
}

fn refuse(m: impl Into<String>) -> ExportError {
    ExportError::Effect(m.into())
}

impl Engine {
    /// Build the changed save from `sav` and write it as a NEW file in `out_dir` (`create_new`, sidecars
    /// copied). The opened file is only read; its bytes are compared again at the end.
    pub fn apply(&self, sav: &Path, out_dir: &Path, ch: &Changes, stem: Option<&str>) -> Result<EditOutcome, ExportError> {
        if let Some(s) = stem {
            export::check_stem(s)?;
        }
        if ch.is_empty() {
            return Err(ExportError::Invalid("nothing to change".into()));
        }
        let bytes = fs::read(sav)?;
        let file_stem = sav.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
        let container = Container::read(&bytes)?;
        let damage: Vec<String> = container.diagnostics().into_iter().filter(|i| i.severity == Severity::Error).map(|i| i.message).collect();
        if !damage.is_empty() {
            return Err(ExportError::Template(damage.join("; ")));
        }
        let doc = Document::parse(&container.payload, container.data_start)?;
        let an = self.analyse(&doc, &file_stem).map_err(ExportError::Template)?;
        let pseudo = self.pseudo_template(&an);

        // Sort the asked answers into rewrites of decided windows and answers of the active window.
        let mut run = Run::default();
        let mut rewrite_quests: Vec<String> = Vec::new();
        let mut baked: Vec<BakedQuest> = Vec::new();
        let mut applied: Vec<String> = Vec::new();
        let mut late_quests: Vec<String> = Vec::new();
        let mut pending_answers: WindowAnswers = WindowAnswers::new();
        let mut buildable: Option<Result<(), String>> = None;
        for (qid, cm) in &ch.answers {
            let quest = self.book.get(qid).ok_or_else(|| refuse(format!("unknown quest `{qid}`")))?;
            let wi = self.window_index(qid).ok_or_else(|| refuse(format!("`{qid}` is not in a decision window, so it cannot be changed")))?;
            let mut changed = Answers::new();
            for (cid, oid) in cm {
                let c = quest.choices.iter().find(|c| &c.id == cid).ok_or_else(|| refuse(format!("`{qid}` has no decision `{cid}`")))?;
                if !c.options.iter().any(|o| &o.id == oid) {
                    return Err(refuse(format!("`{qid}.{cid}` has no option `{oid}`")));
                }
                changed.insert(cid.clone(), oid.clone());
            }
            match an.classes[wi].state {
                WinState::Decided => {
                    let cur = an.current.get(qid).cloned().unwrap_or_default();
                    let mut want = cur.clone();
                    let mut any = false;
                    for (cid, oid) in &changed {
                        if cur.get(cid) == Some(oid) {
                            continue;
                        }
                        self.rewrite_check(&an, &pseudo, qid, cid, oid).map_err(|why| refuse(format!("{qid}.{cid} -> {oid}: {why}")))?;
                        want.insert(cid.clone(), oid.clone());
                        any = true;
                    }
                    if any {
                        let w = &self.scans[wi].0;
                        for wq in &w.quests {
                            if !rewrite_quests.contains(wq) {
                                rewrite_quests.push(wq.clone());
                            }
                        }
                        run.quests.insert(qid.clone(), QuestEntry { status: Status::Completed, answers: want.clone() });
                        baked.push(BakedQuest { quest: qid.clone(), action: Baked::Rewrite { from: cur, to: want } });
                    }
                }
                WinState::NotReached if an.pending == Some(wi) => {
                    let b = buildable.get_or_insert_with(|| self.dry_build(&bytes, &self.scans[wi].0));
                    if let Err(e) = b {
                        return Err(refuse(format!("{qid}: the window is ahead of this save but does not build on it: {e}")));
                    }
                    pending_answers.entry(qid.clone()).or_default().extend(changed);
                    for wq in &self.scans[wi].0.quests {
                        if !applied.contains(wq) {
                            applied.push(wq.clone());
                        }
                    }
                }
                // A later window's quest the save's template completes late (`Template::late`: Now or Never on the
                // Skellige template), with the asked answers (defaults for the rest).
                WinState::NotReached if pseudo.late.contains(qid) && self.scans[wi].0.line > pseudo.line => {
                    run.quests.entry(qid.clone()).or_insert(QuestEntry { status: Status::Completed, answers: Answers::new() }).answers.extend(changed);
                    if !late_quests.contains(qid) {
                        late_quests.push(qid.clone());
                    }
                }
                WinState::NotReached => return Err(refuse(format!("`{qid}` is not reached in this save, and only the next window can be decided here"))),
                WinState::Unknown => return Err(refuse(format!("`{qid}`: {}", an.classes[wi].why))),
            }
        }
        // The window quests as the run sees them: rewrites need every quest of their windows (the references
        // are built from them); the active window's quests are all completed with the chosen or default answers.
        for wq in &rewrite_quests {
            let cur = an.current.get(wq).cloned().unwrap_or_default();
            run.quests.entry(wq.clone()).or_insert(QuestEntry { status: Status::Completed, answers: cur });
        }
        for wq in &applied {
            run.quests.insert(wq.clone(), QuestEntry { status: Status::Completed, answers: pending_answers.get(wq).cloned().unwrap_or_default() });
        }
        // Order the applied quests like the questbook.
        applied.sort_by_key(|q| self.book.get(q).map_or(0, |x| x.order));

        let kit = StartKit {
            level: ch.level.filter(|l| character::read(&doc).map(|c| c.level.level as u32 != *l).unwrap_or(true)),
            crowns: ch.crowns,
            gear: if ch.gear { Gear::Recommended } else { Gear::None },
            items: ch.items.clone(),
            new_game_plus: ch.no_new_game_plus.then_some(false),
            skill_points: ch.skill_points,
        };
        let mut src = self.src();
        src.opts.start_kit = (!kit.is_empty()).then_some(kit);
        src.opts.stem = None;
        let choice = Choice {
            template: pseudo,
            path: Some(sav.to_path_buf()),
            applied: applied.clone(),
            baked,
            beyond: Vec::new(),
            late: late_quests.clone(),
            passed_over: Vec::new(),
            notes: Vec::new(),
        };
        let (out_bytes, mut notes, rewrites) = chain::build_story(&src, &run, &choice, &bytes, Some(&self.lib))?;

        // Final checks on the finished bytes, then write the new file.
        let info = w3save::inspect(&out_bytes)?;
        let bad: Vec<String> = info.issues.iter().filter(|i| i.severity == Severity::Error).map(|i| i.message.clone()).collect();
        if !bad.is_empty() {
            return Err(ExportError::Verify(format!("check-save: {}", bad.join("; "))));
        }
        let again = doc_of(&out_bytes).map_err(ExportError::Verify)?;
        if again.table() != again.map.variables || again.to_bytes() != Container::read(&out_bytes)?.payload {
            return Err(ExportError::Verify("the written save does not re-serialize to itself".into()));
        }
        let new_stem = stem.map_or_else(|| export::default_stem(sav, out_dir), str::to_owned);
        let mut result = export::write_save_set_as(sav, out_dir, &new_stem, &out_bytes)?;
        if fs::read(&result.sav)? != out_bytes {
            return Err(ExportError::Verify(format!("{} on disk differs from the verified bytes; do not use it", result.sav.display())));
        }
        if fs::read(sav)? != bytes {
            return Err(ExportError::Verify(format!("{} changed while editing; this must never happen", sav.display())));
        }
        if notes.is_empty() {
            notes.push("no story change; character edits only".into());
        }
        result.notes = notes.clone();
        Ok(EditOutcome {
            sav: result.sav,
            sidecars: result.sidecars,
            notes,
            rewritten: rewrites.iter().flat_map(|w| w.quests.clone()).collect(),
            applied: late_quests.into_iter().chain(applied).collect(),
            verified: "VERIFIED: the earlier decisions rewritten and checked (facts, quest log, untouched blocks), the window's effects and the kit applied and verified as export does, check-save clean, the file re-read; the opened save was not touched".into(),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Saves in a folder (for the picker)

#[derive(Debug, Clone, Serialize)]
pub struct SaveEntry {
    pub stem: String,
    pub path: String,
    /// `manual`, `quick`, `auto`, `checkpoint` or `other`, from the file name.
    pub kind: &'static str,
    /// Seconds since the Unix epoch (file modification time).
    pub modified: u64,
    pub size: u64,
    /// The `.png` thumbnail beside it, when there is one.
    pub thumbnail: Option<String>,
}

/// The `.sav` files in `dir` (not recursive), newest first. Read-only.
pub fn list_saves(dir: &Path) -> Result<Vec<SaveEntry>, String> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
        let p = e.path();
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sav")) || !p.is_file() {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else { continue };
        let Ok(meta) = e.metadata() else { continue };
        let png = p.with_extension("png");
        out.push(SaveEntry {
            kind: match stem.split('_').next().unwrap_or("") {
                "ManualSave" => "manual",
                "QuickSave" => "quick",
                "AutoSave" => "auto",
                "CheckPoint" => "checkpoint",
                _ => "other",
            },
            modified: meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs()),
            size: meta.len(),
            thumbnail: png.is_file().then(|| png.display().to_string()),
            path: p.display().to_string(),
            stem,
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.stem.cmp(&b.stem)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(base: &[(&str, i64)], cands: &[(&str, &[(&str, i64)])]) -> WindowScan {
        let m = |f: &[(&str, i64)]| f.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect::<BTreeMap<_, _>>();
        WindowScan {
            id: "w".into(),
            base: Some(m(base)),
            cands: cands
                .iter()
                .map(|(opt, f)| CandFacts { answers: [("q".to_owned(), [("c".to_owned(), (*opt).to_owned())].into())].into(), facts: Some(m(f)) })
                .collect(),
            error: None,
        }
    }

    fn table(facts: &[&str]) -> FactTable {
        let p = w3save::payload::testutil::synthetic_with_facts(3084, &facts.iter().map(|n| (*n, 0u16, &[(1u16, 1.0f32, -1.0f32)][..])).collect::<Vec<_>>());
        let c = Container::from_payload(p).write().unwrap();
        let c = Container::read(&c).unwrap();
        Document::parse(&c.payload, c.data_start).unwrap().facts_table().unwrap()
    }

    #[test]
    fn a_clear_match_decides_and_the_baseline_means_not_reached() {
        let s = scan(&[("old", 1)], &[("a", &[("old", 1), ("fa", 1), ("done", 1)]), ("b", &[("old", 1), ("fb", 1), ("done", 1)])]);
        let c = classify(&s, &table(&["old", "fb", "done"]));
        assert_eq!((c.state.clone(), c.known["q"]["c"].as_str()), (WinState::Decided, "b"), "{c:?}");
        // Nothing the window writes is there: not reached, whatever the other facts say.
        let c = classify(&s, &table(&["old"]));
        assert_eq!(c.state, WinState::NotReached);
        // Only the common aftermath is there: decided, but the choice stays unknown (a tie).
        let c = classify(&s, &table(&["old", "done"]));
        assert_eq!(c.state, WinState::Decided);
        assert!(c.known["q"].is_empty() && !c.why.is_empty(), "{c:?}");
    }

    #[test]
    fn a_window_that_matches_nothing_is_unknown() {
        let facts: Vec<String> = (0..9).map(|i| format!("f{i}")).collect();
        let a: Vec<(&str, i64)> = facts[..9].iter().map(|n| (n.as_str(), 1)).collect();
        let s = scan(&[], &[("a", &a), ("b", &[("x", 1)])]);
        // The save holds the first four of a's facts only, none of b's: 5 of 10 deciding facts differ.
        let have: Vec<&str> = facts[..4].iter().map(String::as_str).collect();
        let c = classify(&s, &table(&have));
        assert_eq!(c.state, WinState::Unknown, "{c:?}");
    }

    #[test]
    fn changes_know_when_they_are_empty() {
        assert!(Changes::default().is_empty());
        let mut c = Changes::default();
        c.answers.insert("q".into(), Answers::new());
        assert!(c.is_empty());
        c.level = Some(30);
        assert!(!c.is_empty());
    }
}
