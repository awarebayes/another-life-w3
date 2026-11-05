//! Export: template save + resolved effects -> a new save set on disk.
//!
//! Safety rules from the plan live here: never overwrite an existing save (every
//! file is opened with `create_new`), and refuse to run on damaged templates.
//!
//! Effects are applied in run order; within one effect: graph (`[effects.X.graph]`, which may append
//! the donor's NM names) or synth (`[effects.X.synth]`, walked through the game's quest graphs), then
//! journal (`journal_*`), then facts. Graph ops copy subtrees from donor saves found by stem in the
//! export's donor folders; synth effects read the quest graphs of the game install (`game_dir`), loaded
//! once per export. The whole save is verified before it is written (re-parse, table, post-order,
//! `check-save`, copied subtrees equal the donor's, the synthesized frontier / journal / facts as
//! predicted, journal and facts as requested and nothing else changed, untouched blocks byte-identical)
//! and the file is re-read after writing. Design: docs/findings/export-wiring.md.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use w3save::graph::{self, GraftOp, NameMode};
use w3save::journal::{self, Guid, JournalEdit};
use w3graph::quest::Library;
use w3save::{Container, Document, Edits, Kind, Severity};

use crate::graph_effect::{self, GuardedOp};
use crate::questbook::Effect;
use crate::synth_effect::{self, SynthEffect, Synthesized};
use crate::resolve::Resolution;
use crate::run::Run;
use crate::save_name::{self, SaveTime};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("the questbook run has errors: {0}")]
    Invalid(String),
    #[error("template save is not usable: {0}")]
    Template(String),
    /// Scaffold effects (`placeholder = true` or a `PLACEHOLDER_` name) must never reach a
    /// real save: the names are not game identifiers.
    #[error("refusing to export placeholder effects (not traced to a script or save diff): {0}")]
    Placeholder(String),
    #[error("bad output name {0:?}: use letters, digits, `_`, `-` and spaces only (no dots or path separators)")]
    BadStem(String),
    /// A donor save a graph effect copies from is missing, ambiguous or unreadable.
    #[error("donor save: {0}")]
    Donor(String),
    /// The game's quest graphs a synth effect walks could not be loaded (no install at `game_dir`).
    #[error("game quest graphs: {0}")]
    Graphs(String),
    /// An effect does not fit the template it is applied to (guard hash, overlapping paths, names).
    #[error("refusing effect {0}")]
    Effect(String),
    /// The built save failed its own checks. Nothing was written (or, for a disk mismatch, the
    /// file named in the message must not be used).
    #[error("the built save did not verify: {0}")]
    Verify(String),
    #[error(transparent)]
    Save(#[from] w3save::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub sav: PathBuf,
    pub sidecars: Vec<PathBuf>,
    pub effects_applied: usize,
    /// One line per applied effect (what changed, name mode, donor), for the CLI and the app.
    pub notes: Vec<String>,
}

/// Flatten resolved quest effects into the plain edits `w3save` understands (facts and the legacy
/// id lists; journal and graph tables are applied per effect by [`build_save_with`]).
pub fn to_edits(res: &Resolution) -> Edits {
    let mut e = Edits::default();
    for a in &res.effects {
        e.facts.extend(a.effect.facts.iter().map(|(k, v)| (k.clone(), *v)));
        e.quests.extend(a.effect.quests.iter().map(|(k, p)| {
            let phase = match p {
                crate::questbook::Phase::Started => "started",
                crate::questbook::Phase::Completed => "completed",
                crate::questbook::Phase::Failed => "failed",
            };
            (k.clone(), phase.to_owned())
        }));
        e.journal.extend(a.effect.journal.iter().cloned());
    }
    e
}

/// Knobs of an export that are not part of the run.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Output file stem. `None` = a generated game-style `ManualSave_<a>_<b>_<c>` name (see
    /// [`default_stem`]); the Load list shows the quest name only for that shape. Any other
    /// stem (`AnotherLife_...`) is used as given.
    pub stem: Option<String>,
    /// Development only: let `placeholder = true` effects through (the facts then carry
    /// `PLACEHOLDER_` names that are not real game identifiers).
    pub allow_placeholders: bool,
    /// Folders searched for donor saves (`<donor stem>.sav`, not recursive). A stem found in more
    /// than one folder is refused as ambiguous.
    pub donor_dirs: Vec<PathBuf>,
    /// Use `NameMode::Remap` for a graph effect whose donor's NM names do not extend the save's
    /// (typically the second of two donors). HYPOTHESIS: proven offline only, untested in game.
    pub allow_name_remap: bool,
    /// The Witcher 3 install whose quest graphs `synth` effects walk (read-only). `None` =
    /// [`DEFAULT_GAME_DIR`]. Only read when the run has a synth effect.
    pub game_dir: Option<PathBuf>,
    /// The start kit applied after the story effects (`start_kit.rs`); `None` = none.
    pub start_kit: Option<crate::start_kit::StartKit>,
}

/// Where Steam installs the game; the default `game_dir`.
pub const DEFAULT_GAME_DIR: &str = r"C:\Program Files (x86)\Steam\steamapps\common\The Witcher 3";

impl ExportOptions {
    /// The export knobs a run file carries (`donors`, `allow_name_remap`, `game_dir`); the stem stays generated.
    pub fn for_run(run: &Run) -> Self {
        Self {
            donor_dirs: run.donors.iter().map(PathBuf::from).collect(),
            allow_name_remap: run.allow_name_remap,
            game_dir: run.game_dir.as_ref().filter(|d| !d.trim().is_empty()).map(PathBuf::from),
            start_kit: run.start_kit.clone(),
            ..Self::default()
        }
    }

    /// The game folder synth effects read: `game_dir`, else [`DEFAULT_GAME_DIR`].
    pub fn game_dir(&self) -> PathBuf {
        self.game_dir.clone().unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR))
    }
}

/// Load the game's quest graphs when some effect of the resolution is synthesized (`None` otherwise:
/// a run without synth effects never needs the install). Once per export; every synth effect shares it.
pub fn load_graphs(res: &Resolution, opts: &ExportOptions) -> Result<Option<Library>, ExportError> {
    if !res.effects.iter().any(|a| a.effect.has_synth()) {
        return Ok(None);
    }
    let dir = opts.game_dir();
    let bundles = dir.join("content").join("content0").join("bundles");
    if !bundles.is_dir() {
        return Err(ExportError::Graphs(format!(
            "synthesized effects read the quest graphs of your Witcher 3 install, but {} is not one (no content\\content0\\bundles); \
             set the game folder (`game_dir` in the run file, `w3al export --game-dir DIR`, or the app's Game folder field)",
            dir.display()
        )));
    }
    let lib = Library::load_install(&dir).map_err(|e| ExportError::Graphs(format!("{}: {e}", dir.display())))?;
    if lib.blocks.is_empty() {
        return Err(ExportError::Graphs(format!("{}: no quest graphs found in the bundles", dir.display())));
    }
    Ok(Some(lib))
}

/// Scaffold effects in a resolution, as readable reasons. Empty = nothing to refuse. Both
/// the `placeholder` flag and a `PLACEHOLDER_` prefix on any fact, quest or journal id or
/// graph donor count, so a content author who forgets the flag is still caught.
pub fn placeholder_effects(res: &Resolution) -> Vec<String> {
    let mut out = Vec::new();
    for a in &res.effects {
        let e = &a.effect;
        let named = e
            .facts
            .keys()
            .chain(e.quests.keys())
            .chain(e.journal.iter())
            .chain(e.graph.iter().filter_map(|g| g.donor.as_ref()))
            .find(|n| n.starts_with("PLACEHOLDER_"));
        if e.placeholder || named.is_some() {
            out.push(format!("{}/{}", a.quest, a.origin));
        }
    }
    out
}

/// A file stem we are willing to create: non-empty, at most 100 chars, letters, digits, `_`,
/// `-` and spaces. No dots (the extension is ours) and no separators.
pub fn check_stem(stem: &str) -> Result<(), ExportError> {
    let ok = !stem.is_empty()
        && stem.len() <= 100
        && !stem.starts_with(' ')
        && !stem.ends_with(' ')
        && stem.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ' '));
    if ok {
        Ok(())
    } else {
        Err(ExportError::BadStem(stem.to_owned()))
    }
}

// ---------------------------------------------------------------------------------------------
// Donors

/// Donor documents by stem.
pub type Donors = BTreeMap<String, Document>;

/// The donor save `<stem>.sav` in exactly one of `dirs`.
pub fn find_donor(dirs: &[PathBuf], stem: &str) -> Result<PathBuf, ExportError> {
    graph_effect::check_donor_stem(stem).map_err(ExportError::Donor)?;
    if dirs.is_empty() {
        return Err(ExportError::Donor(format!(
            "a graph effect copies from {stem}.sav, but no donor folder was given (`donors` in the run file, or `w3al export --donors DIR`)"
        )));
    }
    let hits: Vec<PathBuf> = dirs.iter().map(|d| d.join(format!("{stem}.sav"))).filter(|p| p.is_file()).collect();
    match &hits[..] {
        [one] => Ok(one.clone()),
        [] => Err(ExportError::Donor(format!(
            "{stem}.sav not found in {}",
            dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
        ))),
        many => Err(ExportError::Donor(format!(
            "{stem}.sav is ambiguous: {}",
            many.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

fn read_doc(bytes: &[u8]) -> Result<Document, w3save::Error> {
    let c = Container::read(bytes)?;
    Document::parse(&c.payload, c.data_start)
}

/// Load every donor the resolution's graph effects copy from (effects whose ops only remove and
/// recount need none).
pub fn load_donors(res: &Resolution, dirs: &[PathBuf]) -> Result<Donors, ExportError> {
    let mut out = Donors::new();
    for a in &res.effects {
        let Some(g) = a.effect.graph.as_ref().filter(|g| g.needs_donor()) else { continue };
        let Some(stem) = &g.donor else { continue }; // `plan` refuses this case with a lint message
        if out.contains_key(stem) {
            continue;
        }
        let path = find_donor(dirs, stem)?;
        let bytes = fs::read(&path).map_err(|e| ExportError::Donor(format!("{}: {e}", path.display())))?;
        let doc = read_doc(&bytes).map_err(|e| ExportError::Donor(format!("{}: {e}", path.display())))?;
        out.insert(stem.clone(), doc);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Plan: everything that can be refused before the template is read

/// One resolved effect, validated and ready to apply.
struct Step<'a> {
    label: String,
    effect: &'a Effect,
    graph: Option<Vec<GuardedOp>>,
    synth: Option<&'a SynthEffect>,
    journal: Option<JournalEdit>,
}

impl Step<'_> {
    fn donor(&self) -> Option<&str> {
        self.effect.graph.as_ref().and_then(|g| g.donor.as_deref())
    }

    /// The donor document, when this step's ops copy from it.
    fn donor_doc<'d>(&self, donors: &'d Donors) -> Result<Option<&'d Document>, ExportError> {
        match self.effect.graph.as_ref().filter(|g| g.needs_donor()).and_then(|g| g.donor.as_deref()) {
            None => Ok(None),
            Some(stem) => donors
                .get(stem)
                .map(Some)
                .ok_or_else(|| ExportError::Donor(format!("{}: donor {stem}.sav was not loaded", self.label))),
        }
    }
}

fn plan<'a>(res: &'a Resolution, opts: &ExportOptions) -> Result<Vec<Step<'a>>, ExportError> {
    if res.has_errors() {
        let msgs: Vec<_> = res
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message.as_str())
            .collect();
        return Err(ExportError::Invalid(msgs.join("; ")));
    }
    let placeholders = placeholder_effects(res);
    if !placeholders.is_empty() && !opts.allow_placeholders {
        return Err(ExportError::Placeholder(placeholders.join(", ")));
    }
    let mut steps = Vec::with_capacity(res.effects.len());
    for a in &res.effects {
        let label = format!("{}/{}", a.quest, a.origin);
        let e = &a.effect;
        // The scaffold's id lists never had a format behind them.
        if !e.quests.is_empty() || !e.journal.is_empty() {
            return Err(ExportError::Save(w3save::Error::Unverified(format!(
                "{label}: the legacy `quests` / `journal` id lists are not save edits; use captured `journal_*` and `graph` tables"
            ))));
        }
        let journal = if e.has_journal_edit() {
            Some(e.journal_edit().map_err(|m| ExportError::Invalid(format!("{label}: {m}")))?)
        } else {
            None
        };
        let graph = e.graph_ops().map_err(|m| ExportError::Invalid(format!("{label}: {m}")))?;
        if let Some(s) = &e.synth {
            s.check().map_err(|m| ExportError::Invalid(format!("{label}: {m}")))?;
            if graph.is_some() {
                return Err(ExportError::Invalid(format!("{label}: `synth` and `graph` on the same effect")));
            }
        }
        steps.push(Step { label, effect: e, graph, synth: e.synth.as_ref(), journal });
    }
    check_overlaps(&steps)?;
    Ok(steps)
}

/// Composition rule (DECISIONS.md, "Quest graph transplant"): two effects must not touch the same
/// node, or one a node under the other's, unless both come from the same donor; a later replace would
/// silently undo an earlier delta. Recounts are exempt: they count the final children, which is how
/// deltas share a list (scene list, spawn sets, stubs).
fn check_overlaps(steps: &[Step]) -> Result<(), ExportError> {
    let paths = |s: &Step| -> Vec<String> {
        s.graph
            .iter()
            .flatten()
            .filter(|o| !matches!(o.op, GraftOp::Recount { .. }))
            .map(|o| o.op.path().to_owned())
            .collect()
    };
    for (i, a) in steps.iter().enumerate() {
        let pa = paths(a);
        for b in &steps[i + 1..] {
            if a.donor().is_some() && a.donor() == b.donor() {
                continue;
            }
            for q in paths(b) {
                if let Some(p) = pa.iter().find(|p| graph_effect::paths_overlap(p, &q)) {
                    return Err(ExportError::Effect(format!(
                        "{}: its graph op on {q} overlaps {p} of {} (different donors); one delta would undo the other",
                        b.label, a.label
                    )));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Build

/// Build the new save's bytes from a template. Empty edits yield the template's
/// payload re-compressed (the Phase 2 "no effects" check). Placeholder effects are refused.
pub fn build_save(template: &[u8], res: &Resolution) -> Result<Vec<u8>, ExportError> {
    build_save_with(template, res, &ExportOptions::default())
}

pub fn build_save_with(template: &[u8], res: &Resolution, opts: &ExportOptions) -> Result<Vec<u8>, ExportError> {
    Ok(build_verified(template, res, opts)?.bytes)
}

/// A built and verified save.
#[derive(Debug)]
pub struct Built {
    pub bytes: Vec<u8>,
    pub notes: Vec<String>,
}

/// Plan (refusals that need no file), load donors from `opts.donor_dirs` and, when a synth effect needs
/// them, the game's quest graphs from `opts.game_dir` (once), apply, verify.
pub fn build_verified(template: &[u8], res: &Resolution, opts: &ExportOptions) -> Result<Built, ExportError> {
    plan(res, opts)?; // refuse before touching any file
    let donors = load_donors(res, &opts.donor_dirs)?;
    let graphs = load_graphs(res, opts)?;
    build_with_sources(template, res, opts, &donors, graphs.as_ref())
}

/// [`build_verified`] with the donors already loaded (by stem) and no quest graphs (a synth effect is refused).
pub fn build_with_donors(template: &[u8], res: &Resolution, opts: &ExportOptions, donors: &Donors) -> Result<Built, ExportError> {
    build_with_sources(template, res, opts, donors, None)
}

/// [`build_verified`] with donors and quest graphs already loaded (callers that export several runs
/// load the graphs once with [`load_graphs`]).
pub fn build_with_sources(
    template: &[u8],
    res: &Resolution,
    opts: &ExportOptions,
    donors: &Donors,
    graphs: Option<&Library>,
) -> Result<Built, ExportError> {
    let steps = plan(res, opts)?;
    let (mut container, mut doc) = read_template(template)?;
    let original = doc.clone();
    let mut notes = Vec::new();
    let mut modes = Vec::with_capacity(steps.len());
    let mut synths = Vec::with_capacity(steps.len());
    for s in &steps {
        let (note, mode, synth) = apply_step(&mut doc, s, donors, graphs, opts)?;
        notes.push(note);
        modes.push(mode);
        synths.push(synth);
    }
    container.payload = doc.to_bytes();
    let bytes = container.write()?;
    let problems = verify(&original, &bytes, &steps, &modes, &synths, donors);
    if !problems.is_empty() {
        return Err(ExportError::Verify(problems.join("; ")));
    }
    // The start kit goes on after the story effects and is verified on its own (docs/findings/character-inventory.md).
    let bytes = match opts.start_kit.as_ref().filter(|k| !k.is_empty()) {
        Some(kit) => {
            let (b, kit_notes) = crate::start_kit::apply_to_save(&bytes, kit).map_err(ExportError::Verify)?;
            notes.extend(kit_notes);
            b
        }
        None => bytes,
    };
    Ok(Built { bytes, notes })
}

/// What a synth step did, kept for [`verify`] (the document itself is not kept).
struct SynthOutcome {
    journal: JournalEdit,
    facts_set: Vec<(String, i64)>,
    facts_removed: Vec<String>,
    prediction: w3graph::frontier::Prediction,
}

/// Apply one effect: graph or synth, then journal, then facts. Returns a summary line, the name mode
/// used (graph) and the synth outcome (synth).
fn apply_step(
    doc: &mut Document,
    s: &Step,
    donors: &Donors,
    graphs: Option<&Library>,
    opts: &ExportOptions,
) -> Result<(String, Option<NameMode>, Option<SynthOutcome>), ExportError> {
    let refuse = |why: String| ExportError::Effect(format!("{}: {why}", s.label));
    let mut parts = Vec::new();
    let mut mode = None;
    let mut outcome = None;
    if let Some(spec) = s.synth {
        let lib = graphs.ok_or_else(|| ExportError::Graphs(format!("{}: the quest graphs were not loaded", s.label)))?;
        let Synthesized { doc: new_doc, report: r, journal, facts_removed, prediction } =
            synth_effect::apply(lib, doc, spec).map_err(|e| refuse(format!("synth: {e}")))?;
        *doc = new_doc;
        parts.push(format!(
            "synth {} block(s) removed, {} added, {} scope(s) removed, {} added, {} input(s) changed, scenes -{} +{}; journal {} status set, \
             {} appended; facts set [{}], removed [{}]; {} NM names appended{}",
            r.removed_blocks.len(),
            r.added_blocks.len(),
            r.removed_scopes.len(),
            r.added_scopes.len(),
            r.input_changed.len(),
            r.scenes_removed.len(),
            r.scenes_added.len(),
            r.journal.status_set,
            r.journal.appended,
            r.facts_set.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "),
            r.facts_removed.join(" "),
            r.names_added,
            r.notes.iter().map(|n| format!("; note: {n}")).collect::<String>(),
        ));
        outcome = Some(SynthOutcome { journal, facts_set: r.facts_set, facts_removed, prediction });
    }
    if let Some(ops) = &s.graph {
        let donor = s.donor_doc(donors)?;
        // Guards are checked against the save as it is now (earlier effects applied), so a positional
        // path (`a#8`) that an earlier swap-remove moved is caught here, not mis-applied.
        graph_effect::check_guards(ops, doc, donor).map_err(refuse)?;
        let m = match donor {
            None => NameMode::Prefix,
            Some(d) if graph::names_prefix(doc, d) => NameMode::Prefix,
            Some(_) if opts.allow_name_remap => NameMode::Remap,
            Some(_) => {
                return Err(refuse(format!(
                    "the save's NM names (after earlier effects) are not a prefix of donor {}'s, so copied name indices would change \
                     meaning; this needs name remapping (allow_name_remap; HYPOTHESIS, untested in game)",
                    s.donor().unwrap_or_default()
                )))
            }
        };
        let plain: Vec<GraftOp> = ops.iter().map(|g| g.op.clone()).collect();
        // Removals and recounts copy nothing: the save is its own "donor" (its names are trivially a prefix).
        let self_donor;
        let from = match donor {
            Some(d) => d,
            None => {
                self_donor = doc.clone();
                &self_donor
            }
        };
        let r = graph::transplant_with(doc, from, &plain, m).map_err(|e| refuse(format!("graph: {e}")))?;
        parts.push(format!(
            "graph {} replaced, {} removed, {} inserted, {} recounted, {} NM names appended{}{}",
            r.replaced,
            r.removed,
            r.inserted,
            r.recounted.len(),
            r.names_added,
            s.donor().map(|d| format!(", donor {d}")).unwrap_or_default(),
            if m == NameMode::Remap { format!(", names REMAPPED ({} indices; HYPOTHESIS)", r.names_remapped) } else { String::new() }
        ));
        mode = Some(m);
    }
    if let Some(edit) = &s.journal {
        let r = journal::apply(doc, edit).map_err(|e| refuse(format!("journal: {e}")))?;
        parts.push(format!(
            "journal {} status set ({} already), {} appended ({} present), counters {} set / {} added",
            r.status_set, r.status_unchanged, r.appended, r.already_present, r.counters_set, r.counters_appended
        ));
    }
    let facts = &s.effect.facts;
    if !facts.is_empty() {
        let edits = Edits { facts: facts.iter().map(|(k, v)| (k.clone(), *v)).collect(), ..Default::default() };
        doc.apply(&edits).map_err(|e| refuse(format!("facts: {e}")))?;
        parts.push(format!("{} fact(s)", facts.len()));
    }
    let gone = &s.effect.facts_remove;
    if !gone.is_empty() {
        // `FactsRemove` semantics: drop the fact and all its records; a name the save lacks is already gone.
        let removed = doc
            .edit_facts(|t| Ok(gone.iter().filter(|n| t.remove(n).is_some()).count()))
            .map_err(|e| refuse(format!("facts_remove: {e}")))?;
        parts.push(format!("{removed} fact(s) removed"));
    }
    if parts.is_empty() {
        parts.push("nothing to change".into());
    }
    Ok((format!("{}: {}", s.label, parts.join("; ")), mode, outcome))
}

/// Read the template (refusing 5.0.0c damage) and parse it.
fn read_template(template: &[u8]) -> Result<(Container, Document), ExportError> {
    let container = Container::read(template)?;
    // Never build on top of a template with the 5.0.0c damage.
    let damage: Vec<_> = container
        .diagnostics()
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.message)
        .collect();
    if !damage.is_empty() {
        return Err(ExportError::Template(damage.join("; ")));
    }
    let doc = Document::parse(&container.payload, container.data_start)?;
    Ok((container, doc))
}

/// The shared pipeline of the research tools: read the template (refusing 5.0.0c damage), parse it,
/// let `edit` change the document, serialize and re-compress. `edit` must leave the document rebased
/// (`Document::apply` and `Document::edit_facts` do). Used by `w3al edit-fact` and `w3al journal-set`.
pub fn build_save_from_document(
    template: &[u8],
    edit: impl FnOnce(&mut Document) -> Result<(), w3save::Error>,
) -> Result<Vec<u8>, ExportError> {
    let (mut container, mut doc) = read_template(template)?;
    edit(&mut doc)?;
    container.payload = doc.to_bytes();
    Ok(container.write()?)
}

// ---------------------------------------------------------------------------------------------
// Verify

/// Top-level blocks the steps may change; every other top-level node must stay byte-identical.
fn touched_blocks(steps: &[Step]) -> BTreeSet<String> {
    let mut t = BTreeSet::new();
    for s in steps {
        for o in s.graph.iter().flatten() {
            t.insert(o.op.path().split('/').next().unwrap_or_default().to_owned());
        }
        if s.journal.is_some() || s.synth.is_some() {
            t.extend(["CJournalManager".to_owned(), "CWitcherJournalManager".to_owned()]);
        }
        if s.synth.is_some() {
            // `w3graph::synth::apply`: quest frontier and scene registry, journal (with counters), facts,
            // spawn sets (`community`: phases, new / removed sets, removed stubs) and the entity data of
            // removed stubs (`universe > world > layerStorage`).
            t.extend(["questSystem".to_owned(), "facts".to_owned(), "community".to_owned(), "universe".to_owned()]);
        }
        if !s.effect.facts.is_empty() || !s.effect.facts_remove.is_empty() {
            t.insert("facts".to_owned());
        }
    }
    t
}

/// Every check of the built bytes, independent of the in-memory result: what `journal-set` checks,
/// for a whole run. Returns readable problems (empty = verified).
fn verify(
    original: &Document,
    bytes: &[u8],
    steps: &[Step],
    modes: &[Option<NameMode>],
    synths: &[Option<SynthOutcome>],
    donors: &Donors,
) -> Vec<String> {
    let mut problems = Vec::new();
    let doc = match Container::read(bytes).and_then(|c| {
        let d = Document::parse(&c.payload, c.data_start)?;
        Ok((d.to_bytes() == c.payload, d))
    }) {
        Ok((true, d)) => d,
        Ok((false, _)) => return vec!["typed re-parse does not serialize back to the payload".into()],
        Err(e) => return vec![format!("the written bytes do not parse: {e}")],
    };
    if doc.table() != doc.map.variables {
        problems.push("variable table recomputed from the tree differs from the one in the file".into());
    }
    // Real saves are numbered in post-order (122 of 122 files) and graph/journal edits renumber that
    // way; a synthetic template that is not keeps its numbering through a fact edit.
    let renumbered = steps.iter().any(|s| s.graph.is_some() || s.journal.is_some() || s.synth.is_some());
    if (renumbered || original.indices_are_post_order()) && !doc.indices_are_post_order() {
        problems.push("table indices are not in post-order".into());
    }
    match w3save::inspect(bytes) {
        Ok(info) => problems.extend(info.issues.iter().filter(|i| i.severity == Severity::Error).map(|i| format!("check-save: {}", i.message))),
        Err(e) => problems.push(format!("check-save: {e}")),
    }
    if !graph::names_prefix(original, &doc) {
        problems.push("the template's NM names are not a prefix of the result's (a name was changed, not appended)".into());
    }

    // Untouched top-level nodes are byte-identical (names only ever get appended, so indices hold).
    let touched = touched_blocks(steps);
    let (a, b): (Vec<_>, Vec<_>) = (original.top_nodes().collect(), doc.top_nodes().collect());
    if a.len() != b.len() {
        problems.push(format!("top-level node count {} -> {}", a.len(), b.len()));
    } else {
        for (x, y) in a.iter().zip(&b) {
            let name = graph::node_name(original, x);
            if matches!(x.kind, Kind::Manu(_)) || touched.contains(&name) {
                continue;
            }
            if graph::node_name(&doc, y) != name || x.to_bytes() != y.to_bytes() {
                problems.push(format!("top-level {name} changed although no effect edits it"));
            }
        }
    }

    // Graph: copied subtrees equal the donor's; removed keyed nodes are gone (unless re-inserted later).
    for (i, (s, mode)) in steps.iter().zip(modes).enumerate() {
        let Some(ops) = &s.graph else { continue };
        let donor = s.donor_doc(donors).ok().flatten();
        for g in ops {
            let p = g.op.path();
            if g.copies() {
                let Some(d) = donor else { continue };
                let fp = |x: &Document| {
                    graph::locate(x, p).ok().flatten().map(|l| {
                        let n = graph::node_at(x, &l);
                        if *mode == Some(NameMode::Remap) {
                            graph::resolved_fingerprint(x, n).unwrap_or_default()
                        } else {
                            graph::fingerprint(x, n)
                        }
                    })
                };
                if fp(&doc).is_none() || fp(&doc) != fp(d) {
                    problems.push(format!("{}: graph node {p} differs from the donor's", s.label));
                }
            } else if matches!(g.op, GraftOp::Remove { .. } | GraftOp::SwapRemove { .. }) && !p.contains('#') {
                let reinserted = steps[i + 1..].iter().flat_map(|t| t.graph.iter().flatten()).any(|o| o.copies() && o.op.path() == p);
                if !reinserted && graph::locate(&doc, p).ok().flatten().is_some() {
                    problems.push(format!("{}: removed graph node {p} is still there", s.label));
                }
            }
        }
    }
    // Count rules hold wherever a recount ran.
    for s in steps {
        for g in s.graph.iter().flatten() {
            if let GraftOp::Recount { path, of } = &g.op {
                if let Some(problem) = count_problem(&doc, path, of) {
                    problems.push(format!("{}: {problem}", s.label));
                }
            }
        }
    }

    if synths.iter().any(Option::is_some) {
        problems.extend(verify_synth_frontier(&doc, steps, synths));
    }
    // Journal edits in apply order: per step the synthesized edit, then the effect's own tables.
    let edits: Vec<&JournalEdit> = steps
        .iter()
        .zip(synths)
        .flat_map(|(s, o)| o.as_ref().map(|o| &o.journal).into_iter().chain(s.journal.as_ref()))
        .collect();
    if !edits.is_empty() {
        problems.extend(verify_journal(original, &doc, &edits));
    }
    if synths.iter().any(Option::is_some) || steps.iter().any(|s| !s.effect.facts.is_empty() || !s.effect.facts_remove.is_empty()) {
        problems.extend(verify_facts(original, &doc, steps, synths));
    }
    problems
}

/// The quest frontier after the run holds what each synth walk predicted: its new waits are there and the
/// waits it consumed or ended are gone, unless a later synth step changed that block again.
fn verify_synth_frontier(doc: &Document, steps: &[Step], synths: &[Option<SynthOutcome>]) -> Vec<String> {
    let f = match w3graph::frontier::read(doc) {
        Ok(f) => f.guids(),
        Err(e) => return vec![format!("quest frontier does not read: {e}")],
    };
    let mut problems = Vec::new();
    for (i, (s, o)) in steps.iter().zip(synths).enumerate() {
        let Some(o) = o else { continue };
        let later = || synths[i + 1..].iter().flatten();
        let p = &o.prediction;
        for g in &p.added {
            if !f.contains(g) && !later().any(|l| l.prediction.removed.contains(g)) {
                problems.push(format!("{}: synthesized wait {} is not in the quest frontier", s.label, w3graph::quest::guid_hex(g)));
            }
        }
        for g in p.removed.difference(&p.added) {
            if f.contains(g) && !later().any(|l| l.prediction.added.contains(g)) {
                problems.push(format!("{}: block {} should have left the quest frontier", s.label, w3graph::quest::guid_hex(g)));
            }
        }
    }
    problems
}

fn count_problem(doc: &Document, path: &str, of: &str) -> Option<String> {
    let parent_path = path.rsplit_once('/')?.0;
    let parent = graph::locate(doc, parent_path).ok().flatten().map(|l| graph::node_at(doc, &l))?;
    let n = parent.children().iter().filter(|c| matches!(c.kind, Kind::Block(_)) && graph::node_name(doc, c) == of).count();
    let var = graph::locate(doc, path).ok().flatten().map(|l| graph::node_at(doc, &l))?;
    let Kind::Var(v) = &var.kind else { return Some(format!("{path} is not a value")) };
    let have = match v.value {
        w3save::Value::U32(x) => i64::from(x),
        w3save::Value::I32(x) => i64::from(x),
        _ => return Some(format!("{path} is not a count")),
    };
    (have != n as i64).then(|| format!("{path} = {have}, but there are {n} `{of}` children"))
}

/// The quest log after the run: every requested entry/counter/tracking value is there (the last
/// request for a key wins), and nothing else changed (no removals, no unrequested change or append).
fn verify_journal(original: &Document, doc: &Document, edits: &[&JournalEdit]) -> Vec<String> {
    let (a, b) = match (journal::read(original), journal::read(doc)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return vec![format!("journal does not read: {e}")],
    };
    let mut want: HashMap<Vec<Guid>, (String, Option<bool>)> = HashMap::new();
    let mut counters: HashMap<Guid, i32> = HashMap::new();
    let (mut tracked, mut highlighted) = (None, None);
    for e in edits {
        for c in &e.status {
            want.insert(c.key.clone(), (c.to.clone(), c.unread));
        }
        for x in &e.append {
            want.insert(x.key(), (x.status.clone(), Some(x.unread)));
        }
        for c in &e.counters {
            counters.insert(c.guid, c.count);
        }
        tracked = e.tracked_quest.or(tracked);
        highlighted = e.highlighted_objective.or(highlighted);
    }
    let mut problems = Vec::new();
    let d = journal::diff(&a, &b);
    if !d.removed.is_empty() || !d.counters_removed.is_empty() {
        problems.push(format!("journal lost {} entries and {} counters", d.removed.len(), d.counters_removed.len()));
    }
    for e in d.changed.iter().map(|c| &c.after).chain(d.added.iter().map(|x| &x.1)) {
        if !want.contains_key(&e.key()) {
            problems.push(format!("journal entry {} changed without being asked", e.label()));
        }
    }
    for (key, (status, unread)) in &want {
        match &b.find(key)[..] {
            [i] => {
                let e = &b.entries[*i];
                if &e.status != status || unread.is_some_and(|u| u != e.unread) {
                    problems.push(format!("journal entry {} is {} (unread {}), not {status}", e.label(), e.status, e.unread));
                }
            }
            hits => problems.push(format!("journal entry {} found {} times", journal::key_hex(key), hits.len())),
        }
    }
    for g in d.counters_changed.iter().map(|c| c.0).chain(d.counters_added.iter().map(|c| c.1.guid)) {
        if !counters.contains_key(&g) {
            problems.push(format!("objective counter {} changed without being asked", journal::guid_hex(&g)));
        }
    }
    for (g, n) in &counters {
        if b.counters.iter().find(|c| &c.guid == g).map(|c| c.count) != Some(*n) {
            problems.push(format!("objective counter {} is not {n}", journal::guid_hex(g)));
        }
    }
    if b.tracked_quest != tracked.or(a.tracked_quest) || b.highlighted_objective != highlighted.or(a.highlighted_objective) {
        problems.push("tracked quest / highlighted objective are not what the effects asked for".into());
    }
    problems
}

/// Facts after the run: each requested fact holds its value (the last request wins), no other fact changed.
fn verify_facts(original: &Document, doc: &Document, steps: &[Step], synths: &[Option<SynthOutcome>]) -> Vec<String> {
    let (a, b) = match (original.facts_table(), doc.facts_table()) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return vec![format!("facts do not read: {e}")],
    };
    // Final wish per fact in run order: `Some(v)` = set to v, `None` = removed (within a step: the synthesized
    // facts and removals, then `facts`, then `facts_remove`).
    let mut want: HashMap<&str, Option<i64>> = HashMap::new();
    for (s, o) in steps.iter().zip(synths) {
        if let Some(o) = o {
            want.extend(o.facts_set.iter().map(|(k, v)| (k.as_str(), Some(*v))));
            want.extend(o.facts_removed.iter().map(|k| (k.as_str(), None)));
        }
        want.extend(s.effect.facts.iter().map(|(k, v)| (k.as_str(), Some(*v))));
        want.extend(s.effect.facts_remove.iter().map(|k| (k.as_str(), None)));
    }
    let mut problems = Vec::new();
    for n in a.differing(&b) {
        if !want.contains_key(n.as_str()) {
            problems.push(format!("fact {n} changed without being asked"));
        }
    }
    for (n, v) in &want {
        match v {
            Some(v) if b.get(n).map(|f| f.sum()) != Some(*v) => problems.push(format!("fact {n} is not {v}")),
            None if b.get(n).is_some() => problems.push(format!("fact {n} was not removed")),
            _ => {}
        }
    }
    problems
}

/// The default output stem: `ManualSave_<a>_<b>_<c>` like the game's own saves, because the
/// Load list shows the quest name only for that shape (user-reported 2026-11-04).
///
/// `a` is the template's (it keeps the template's tracked quest), `b`/`c` are the current
/// date and time packed the way the game does, later than the template's, and never the stem
/// of a file (`.sav` or sidecar) already in `out_dir`. See [`crate::save_name`] for what the
/// ids mean and what is still a hypothesis.
pub fn default_stem(template_path: &Path, out_dir: &Path) -> String {
    let template_stem = template_path.file_stem().and_then(|s| s.to_str());
    save_name::next_stem(template_stem, SaveTime::now_utc(), |stem| stem_in_use(out_dir, stem))
}

/// True when `out_dir` already holds a `.sav` or a sidecar with this stem.
fn stem_in_use(out_dir: &Path, stem: &str) -> bool {
    // Same list as `w3save::sidecar`, plus the save itself.
    ["sav", "png", "json", "req"]
        .iter()
        .any(|ext| out_dir.join(format!("{stem}.{ext}")).exists())
}

/// `AnotherLife_YYYYMMDD_HHMMSS` (UTC), without an extension. No longer the default (it loads
/// but gets no quest name in the Load list); kept for callers that want such a name explicitly.
pub fn timestamped_stem() -> String {
    let t = SaveTime::now_utc();
    format!(
        "AnotherLife_{:04}{:02}{:02}_{:02}{:02}{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// Write the new `.sav` plus the template's sidecars beside it, under the [`default_stem`]
/// name. Existing files are never touched: `create_new` fails instead.
pub fn write_save_set(template_path: &Path, out_dir: &Path, sav_bytes: &[u8]) -> Result<ExportResult, ExportError> {
    write_save_set_as(template_path, out_dir, &default_stem(template_path, out_dir), sav_bytes)
}

/// Like [`write_save_set`] with an explicit file stem (see [`check_stem`]).
pub fn write_save_set_as(
    template_path: &Path,
    out_dir: &Path,
    stem: &str,
    sav_bytes: &[u8],
) -> Result<ExportResult, ExportError> {
    check_stem(stem)?;
    fs::create_dir_all(out_dir)?;
    let sav = out_dir.join(format!("{stem}.sav"));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&sav)?
        .write_all(sav_bytes)?;
    let sidecars = w3save::sidecar::copy_for(template_path, &sav)?;
    Ok(ExportResult {
        sav,
        sidecars,
        effects_applied: 0,
        notes: Vec::new(),
    })
}

/// The whole pipeline for a run, used by the CLI and the app.
pub fn export(template_path: &Path, out_dir: &Path, res: &Resolution) -> Result<ExportResult, ExportError> {
    export_with(template_path, out_dir, res, &ExportOptions::default())
}

pub fn export_with(
    template_path: &Path,
    out_dir: &Path,
    res: &Resolution,
    opts: &ExportOptions,
) -> Result<ExportResult, ExportError> {
    if let Some(stem) = &opts.stem {
        check_stem(stem)?; // fail before reading anything
    }
    let template = fs::read(template_path)?;
    let built = build_verified(&template, res, opts)?;
    let stem = opts.stem.clone().unwrap_or_else(|| default_stem(template_path, out_dir));
    let mut result = write_save_set_as(template_path, out_dir, &stem, &built.bytes)?;
    // Re-read the file: what is on disk must be exactly the bytes that passed `verify`.
    if fs::read(&result.sav)? != built.bytes {
        return Err(ExportError::Verify(format!("{} on disk differs from the verified bytes; do not use it", result.sav.display())));
    }
    result.effects_applied = res.effects.len();
    result.notes = built.notes;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolution(placeholder: bool, fact: &str, value: i64) -> Resolution {
        let effect = crate::Effect {
            source: "test".into(),
            placeholder,
            facts: [(fact.to_owned(), value)].into(),
            ..Default::default()
        };
        Resolution {
            effects: vec![crate::resolve::AppliedEffect { quest: "q".into(), origin: "done".into(), effect }],
            ..Default::default()
        }
    }

    fn facts_template() -> Vec<u8> {
        let p = w3save::payload::testutil::synthetic_with_facts(3084, &[("existing_fact", 0, &[(1, 100.0, -1.0)])]);
        Container::from_payload(p).write().unwrap()
    }

    fn facts_of(save: &[u8]) -> w3save::facts::FactTable {
        let c = Container::read(save).unwrap();
        Document::parse(&c.payload, c.data_start).unwrap().facts_table().unwrap()
    }

    #[test]
    fn real_fact_effects_reach_the_save() {
        let out = build_save(&facts_template(), &resolution(false, "q001_real_fact", 2)).unwrap();
        let t = facts_of(&out);
        assert_eq!(t.get("q001_real_fact").unwrap().sum(), 2);
        assert_eq!(t.get("existing_fact").unwrap().sum(), 1, "other facts stay");
        // and the file is a consistent save: re-parses with a matching variable table
        let c = Container::read(&out).unwrap();
        let doc = Document::parse(&c.payload, c.data_start).unwrap();
        assert_eq!(doc.table(), doc.map.variables);
    }

    #[test]
    fn placeholder_effects_are_refused_unless_allowed() {
        let template = facts_template();
        // flagged effect
        let err = build_save(&template, &resolution(true, "q001_whatever", 1)).unwrap_err();
        assert!(matches!(err, ExportError::Placeholder(ref s) if s == "q/done"), "{err}");
        // unflagged effect that still carries a PLACEHOLDER_ name
        assert!(matches!(
            build_save(&template, &resolution(false, "PLACEHOLDER_x", 1)),
            Err(ExportError::Placeholder(_))
        ));
        // explicit development switch lets it through (and the name lands in the save)
        let opts = ExportOptions { allow_placeholders: true, ..Default::default() };
        let out = build_save_with(&template, &resolution(true, "PLACEHOLDER_x", 1), &opts).unwrap();
        assert!(facts_of(&out).get("PLACEHOLDER_x").is_some());
    }

    #[test]
    fn quest_effects_are_still_refused() {
        let mut res = resolution(false, "q001_real_fact", 1);
        res.effects[0].effect.quests.insert("q_real".into(), crate::questbook::Phase::Completed);
        assert!(matches!(build_save(&facts_template(), &res), Err(ExportError::Save(w3save::Error::Unverified(_)))));
    }

    #[test]
    fn journal_effects_reach_the_journal_editor() {
        // No longer refused (gates G, P, S, F, R): the edit goes to `journal::apply`, which refuses a
        // synthetic template that has no quest log, naming the effect.
        let mut res = resolution(false, "q001_real_fact", 1);
        res.effects[0].effect.journal_track =
            Some(crate::journal_effect::JournalTrackEffect { quest: Some("0f3305c02a250d4aa75c6fba97048763".into()), objective: None });
        let err = build_save(&facts_template(), &res).unwrap_err();
        assert!(matches!(err, ExportError::Effect(ref m) if m.starts_with("q/done: journal:")), "{err}");
    }

    #[test]
    fn legacy_journal_ids_are_still_refused() {
        let mut res = resolution(false, "q001_real_fact", 1);
        res.effects[0].effect.journal.push("real_journal_entry".into());
        assert!(matches!(build_save(&facts_template(), &res), Err(ExportError::Save(w3save::Error::Unverified(_)))));
    }

    fn graph(donor: Option<&str>, ops: &[(&str, &str)]) -> crate::graph_effect::GraphEffect {
        crate::graph_effect::GraphEffect {
            source: None,
            donor: donor.map(str::to_owned),
            ops: ops
                .iter()
                .map(|(op, path)| {
                    // `recount` takes `PATH=of`, like `--graph-op`
                    let (path, of) = path.split_once('=').unwrap_or((path, ""));
                    crate::graph_effect::GraphOpEffect { op: (*op).into(), path: path.into(), of: of.into(), ..Default::default() }
                })
                .collect(),
        }
    }

    fn graph_resolution(effects: Vec<crate::graph_effect::GraphEffect>) -> Resolution {
        let effects = effects
            .into_iter()
            .enumerate()
            .map(|(i, g)| crate::resolve::AppliedEffect {
                quest: format!("q{i}"),
                origin: "done".into(),
                effect: crate::Effect { source: "test".into(), graph: Some(g), ..Default::default() },
            })
            .collect();
        Resolution { effects, ..Default::default() }
    }

    #[test]
    fn graph_effects_need_their_donor_save() {
        let res = graph_resolution(vec![graph(Some("ManualSave_1_2_3"), &[("insert", "questSystem/x")])]);
        // no donor folder at all
        let err = build_save(&facts_template(), &res).unwrap_err();
        assert!(matches!(err, ExportError::Donor(ref m) if m.contains("no donor folder")), "{err}");
        // a folder without it
        let dir = scratch("donor-missing");
        let opts = ExportOptions { donor_dirs: vec![dir.clone()], ..Default::default() };
        let err = build_save_with(&facts_template(), &res, &opts).unwrap_err();
        assert!(matches!(err, ExportError::Donor(ref m) if m.contains("ManualSave_1_2_3.sav not found")), "{err}");
        // found once: used; found twice: ambiguous
        fs::write(dir.join("ManualSave_1_2_3.sav"), b"x").unwrap();
        assert_eq!(find_donor(std::slice::from_ref(&dir), "ManualSave_1_2_3").unwrap(), dir.join("ManualSave_1_2_3.sav"));
        let other = dir.join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("ManualSave_1_2_3.sav"), b"x").unwrap();
        assert!(matches!(find_donor(&[dir.clone(), other], "ManualSave_1_2_3"), Err(ExportError::Donor(ref m)) if m.contains("ambiguous")));
        assert!(find_donor(std::slice::from_ref(&dir), "../ManualSave_1_2_3").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overlapping_graph_paths_from_different_donors_are_refused() {
        let scope = "questSystem/quest[q]/questThread/questThread[38424c39]";
        let a = graph(Some("DonorA"), &[("replace", scope), ("recount", "community/stubs/numStubs=a")]);
        let b = graph(Some("DonorB"), &[("remove", &format!("{scope}/questThread/questBlock[62a65bce]"))]);
        let res = graph_resolution(vec![a.clone(), b.clone()]);
        let err = plan(&res, &ExportOptions::default()).err().unwrap();
        assert!(matches!(err, ExportError::Effect(ref m) if m.contains("overlaps") && m.contains("q0/done")), "{err}");
        // the same donor may touch the same scope twice
        let same = graph(Some("DonorA"), &b.ops.iter().map(|o| (o.op.as_str(), o.path.as_str())).collect::<Vec<_>>());
        assert!(plan(&graph_resolution(vec![a.clone(), same]), &ExportOptions::default()).is_ok());
        // shared recounts and sibling paths are how deltas compose
        let c = graph(None, &[("swap-remove", "community/stubs/a#3"), ("recount", "community/stubs/numStubs=a")]);
        let d = graph(Some("DonorB"), &[("remove", "questSystem/quest[q]/questThread/questThread[38424c39b]")]);
        assert!(plan(&graph_resolution(vec![a, c, d]), &ExportOptions::default()).is_ok());
    }

    #[test]
    fn graph_lint_errors_stop_the_export() {
        let res = graph_resolution(vec![graph(None, &[("insert", "questSystem/x")])]);
        assert!(matches!(build_save(&facts_template(), &res), Err(ExportError::Invalid(ref m)) if m.contains("donor` is missing")));
        let res = graph_resolution(vec![graph(None, &[("remove", "universe/x")])]);
        assert!(matches!(build_save(&facts_template(), &res), Err(ExportError::Invalid(_))));
        let mut res = graph_resolution(vec![graph(Some("PLACEHOLDER_donor"), &[("remove", "questSystem/x")])]);
        assert!(matches!(build_save(&facts_template(), &res), Err(ExportError::Placeholder(_))));
        res.effects[0].effect.graph = None;
        res.effects[0].effect.placeholder = true;
        assert!(matches!(build_save(&facts_template(), &res), Err(ExportError::Placeholder(_))));
    }

    fn synth_resolution() -> Resolution {
        let synth = crate::synth_effect::SynthEffect {
            start: vec!["215f2719".into()],
            through: true,
            pick: vec!["d24f649d=Keira to KM".into()],
            stop: vec!["4c2a9b2d".into()],
            scene_pick: Vec::new(),
            complete: None,
        };
        let effect = crate::Effect { source: "test".into(), synth: Some(synth), ..Default::default() };
        Resolution {
            effects: vec![crate::resolve::AppliedEffect { quest: "q".into(), origin: "done".into(), effect }],
            ..Default::default()
        }
    }

    #[test]
    fn synth_effects_need_the_game_install() {
        // No install at the given folder: a clear error before the template is read.
        let opts = ExportOptions { game_dir: Some(scratch("no-install")), ..Default::default() };
        let err = build_save_with(b"not a save", &synth_resolution(), &opts).unwrap_err();
        assert!(matches!(err, ExportError::Graphs(ref m) if m.contains("game_dir") && m.contains("content0")), "{err}");
        fs::remove_dir_all(opts.game_dir.unwrap()).unwrap();
        // Graphs not handed over: refused, naming the effect.
        let err = build_with_donors(&facts_template(), &synth_resolution(), &ExportOptions::default(), &Donors::new()).unwrap_err();
        assert!(matches!(err, ExportError::Graphs(ref m) if m.starts_with("q/done")), "{err}");
        // A run without synth effects never touches the install.
        let none = ExportOptions { game_dir: Some(PathBuf::from("Z:/nowhere")), ..Default::default() };
        assert!(load_graphs(&resolution(false, "q001_real_fact", 1), &none).unwrap().is_none());
        assert_eq!(ExportOptions::default().game_dir(), PathBuf::from(DEFAULT_GAME_DIR));
        let run = Run { game_dir: Some("D:/Games/W3".into()), ..Run::default() };
        assert_eq!(ExportOptions::for_run(&run).game_dir(), PathBuf::from("D:/Games/W3"));
    }

    #[test]
    fn synth_with_graph_or_bad_shape_is_refused() {
        let mut res = synth_resolution();
        res.effects[0].effect.graph = Some(graph(None, &[("remove", "questSystem/x")]));
        assert!(matches!(plan(&res, &ExportOptions::default()), Err(ExportError::Invalid(ref m)) if m.contains("`synth` and `graph`")));
        let mut res = synth_resolution();
        res.effects[0].effect.synth.as_mut().unwrap().pick.clear();
        res.effects[0].effect.synth.as_mut().unwrap().stop.clear();
        assert!(matches!(plan(&res, &ExportOptions::default()), Err(ExportError::Invalid(_))));
        let mut res = synth_resolution();
        res.effects[0].effect.placeholder = true;
        assert!(matches!(plan(&res, &ExportOptions::default()), Err(ExportError::Placeholder(_))));
    }

    #[test]
    fn synth_lint_rules() {
        let quest = |effect: crate::Effect| crate::Quest {
            id: "q".into(),
            title: "Q".into(),
            act: 2,
            order: 10,
            kind: crate::questbook::Kind::Secondary,
            region: crate::questbook::Region::Velen,
            level: 1,
            requires: vec![],
            excludes: vec![],
            image: None,
            summary: "s".into(),
            started_effects: None,
            done_effects: Some("done".into()),
            choices: vec![],
            effects: [("done".to_owned(), effect)].into(),
            file: PathBuf::new(),
        };
        let lint = |e: crate::Effect| {
            crate::validate::lint(&crate::Questbook::from_quests(vec![quest(e)]))
                .into_iter()
                .map(|d| (d.is_error(), d.message))
                .collect::<Vec<_>>()
        };
        let good = synth_resolution().effects.remove(0).effect;
        assert!(lint(good.clone()).is_empty(), "{:?}", lint(good.clone()));
        let mut e = good.clone();
        e.graph = Some(graph(None, &[("remove", "questSystem/x")]));
        assert!(lint(e).iter().any(|(err, m)| *err && m.contains("`synth` and `graph`")));
        let mut e = good.clone();
        e.placeholder = true;
        assert!(lint(e).iter().any(|(err, m)| *err && m.contains("cannot be a placeholder")));
        let mut e = good.clone();
        e.synth.as_mut().unwrap().start.clear();
        assert!(lint(e).iter().any(|(err, m)| *err && m.contains("`start`")));
        let mut e = good;
        e.synth.as_mut().unwrap().pick = vec!["d24f649d".into()];
        assert!(lint(e).iter().any(|(err, m)| *err && m.contains("GUID=socket")));
    }

    #[test]
    fn stems_are_validated_and_used() {
        for good in ["ManualSave_52586_7ea4c800_1030a5e", "AnotherLife_20261104_172834", "a b-c"] {
            assert!(check_stem(good).is_ok(), "{good}");
        }
        for bad in ["", "a.sav", "../x", "a/b", "a\\b", " lead", "trail ", "caf\u{e9}", &"x".repeat(101)] {
            assert!(matches!(check_stem(bad), Err(ExportError::BadStem(_))), "{bad:?}");
        }
        let dir = std::env::temp_dir().join(format!("w3al-stem-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let template = dir.join("ManualSave_1_2_3.sav");
        std::fs::write(&template, facts_template()).unwrap();
        std::fs::write(template.with_extension("png"), b"thumb").unwrap();
        let out = dir.join("out");
        let opts = ExportOptions { stem: Some("ManualSave_9_9_9".into()), ..Default::default() };
        let r = export_with(&template, &out, &Resolution::default(), &opts).unwrap();
        assert_eq!(r.sav, out.join("ManualSave_9_9_9.sav"));
        assert_eq!(r.sidecars, [out.join("ManualSave_9_9_9.png")]);
        // a second export under the same name fails instead of overwriting
        assert!(export_with(&template, &out, &Resolution::default(), &opts).is_err());
        assert!(export_with(&template, &out, &Resolution::default(), &ExportOptions { stem: Some("a.b".into()), ..Default::default() }).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn legacy_stem_shape() {
        let s = timestamped_stem();
        assert!(s.starts_with("AnotherLife_") && s.len() == "AnotherLife_20261104_153012".len());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("w3al-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stamp(stem: &str) -> (u32, u32) {
        let g = save_name::parse_stem(stem).unwrap_or_else(|| panic!("{stem} is not a game-style name"));
        (g.b, g.c)
    }

    #[test]
    fn default_stem_is_game_style_keeps_a_and_is_later_than_the_template() {
        let dir = scratch("stem-default");
        // A template from the user's own game, two weeks old.
        let template = dir.join("ManualSave_52586_7ea4c400_1030937.sav");
        let s = default_stem(&template, &dir);
        let g = save_name::parse_stem(&s).unwrap();
        assert_eq!((g.prefix.as_str(), g.a.as_str()), ("ManualSave", "52586"));
        assert!(stamp(&s) > (0x7ea4c400, 0x1030937), "{s}");
        assert!(check_stem(&s).is_ok(), "{s}");
        // The output is a plain name: no AnotherLife_ unless asked for with --stem.
        assert!(!s.starts_with("AnotherLife_"));
        // A template whose own name has no ids (a renamed copy) gets the documented default `a`.
        let s = default_stem(&dir.join("AnotherLife_20261104_172834.sav"), &dir);
        assert!(s.starts_with(&format!("ManualSave_{}_", save_name::DEFAULT_QUEST_KEY)), "{s}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn default_stem_skips_names_already_on_disk() {
        let dir = scratch("stem-clash");
        let template = dir.join("ManualSave_52586_7ea4c400_1030937.sav");
        // Block the next 30 milliseconds' worth of names, each by a different file kind.
        let mut blocked = Vec::new();
        let mut when = SaveTime::now_utc();
        for i in 0..30 {
            let ext = ["sav", "png", "json", "req"][i % 4];
            let stem = format!("ManualSave_52586_{:x}_{:x}", when.b(), when.c());
            fs::write(dir.join(format!("{stem}.{ext}")), b"x").unwrap();
            blocked.push(stem);
            when = when.plus_millis(1);
        }
        let s = default_stem(&template, &dir);
        assert!(!blocked.contains(&s), "{s} is already on disk");
        assert!(!stem_in_use(&dir, &s));
        assert!(stem_in_use(&dir, &blocked[0]) && stem_in_use(&dir, &blocked[1]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn repeated_default_exports_make_distinct_game_style_saves() {
        let dir = scratch("stem-repeat");
        let template = dir.join("ManualSave_52586_7ea4c400_1030937.sav");
        fs::write(&template, facts_template()).unwrap();
        fs::write(template.with_extension("png"), b"thumb").unwrap();
        let out = dir.join("out");
        let mut stems = Vec::new();
        for _ in 0..4 {
            let r = export(&template, &out, &Resolution::default()).unwrap();
            assert!(r.sav.is_file());
            assert_eq!(r.sidecars.len(), 1);
            let stem = r.sav.file_stem().unwrap().to_str().unwrap().to_owned();
            assert!(stamp(&stem) > (0x7ea4c400, 0x1030937));
            stems.push(stem);
        }
        let mut unique = stems.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), stems.len(), "{stems:?}");
        // exporting beside the template (same folder) works too and never touches it
        let r = export(&template, &dir, &Resolution::default()).unwrap();
        assert_ne!(r.sav, template);
        assert_eq!(fs::read(&template).unwrap(), facts_template());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_explicit_stem_still_wins() {
        let dir = scratch("stem-explicit");
        let template = dir.join("ManualSave_52586_7ea4c400_1030937.sav");
        fs::write(&template, facts_template()).unwrap();
        let opts = ExportOptions { stem: Some("AnotherLife_20261104_172834".into()), ..Default::default() };
        let r = export_with(&template, &dir.join("out"), &Resolution::default(), &opts).unwrap();
        assert_eq!(r.sav.file_name().unwrap(), "AnotherLife_20261104_172834.sav");
        fs::remove_dir_all(&dir).unwrap();
    }
}
