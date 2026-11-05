//! `w3al`: the scripting and research interface. Thin over `w3save` and `w3al-core`.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use w3al_core::diagnostic::Severity as CSev;
use w3al_core::{export, paths, resolve, validate, Questbook, QuestbookView, Run};
use w3save::{Container, Document, Region, Severity};

mod chain_cmd;
mod character_cmd;
mod edit_cmd;
mod edit_save_cmd;
mod facts_cmd;
mod gametext_cmd;
mod graph_cmd;
mod journal_cmd;
mod side_cmd;
mod synth_cmd;

#[derive(Parser)]
#[command(name = "w3al", version, about = "Another Life for The Witcher 3 - save tooling")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Describe a save: container, payload map, variable tags, name table.
    Dump {
        sav: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Phase 0 check: decompress -> parse -> serialize -> compare payloads, then
    /// re-compress and decode again.
    Roundtrip { sav: PathBuf },
    /// Compare two saves region by region (research tool).
    Diff { a: PathBuf, b: PathBuf },
    /// List the facts of a save (name, c1, records), or diff them against another save.
    Facts {
        sav: PathBuf,
        /// Print added / removed / changed facts against this save.
        #[arg(long)]
        diff: Option<PathBuf>,
        /// Only facts whose name contains this text (case-insensitive).
        #[arg(long)]
        grep: Option<String>,
    },
    /// Validate every questbook TOML file.
    Lint {
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Flag damage signatures (5.0.0c table overflow, oversized fact ids).
    CheckSave { sav: PathBuf },
    /// Build a save from a run profile.
    Export {
        #[arg(long)]
        run: PathBuf,
        /// Template save to patch; defaults to `template` from the run file.
        #[arg(long)]
        template: Option<PathBuf>,
        #[arg(long, default_value = ".")]
        out_dir: PathBuf,
        #[arg(long)]
        questbook: Option<PathBuf>,
        /// Print the effects that would be applied and stop.
        #[arg(long)]
        dry_run: bool,
        /// Output file name without extension; default is a generated game-style `ManualSave_<a>_<b>_<c>` (the Load list shows the quest name only for that shape).
        #[arg(long)]
        stem: Option<String>,
        /// Development only: let `placeholder = true` effects through (they carry invented names).
        #[arg(long)]
        allow_placeholders: bool,
        /// Folder holding donor saves (`<donor stem>.sav`) for graph effects; added to the run file's `donors`. Repeatable.
        #[arg(long = "donors")]
        donors: Vec<PathBuf>,
        /// Allow NM name remapping for a donor whose names do not extend the save's (HYPOTHESIS: untested in game).
        #[arg(long)]
        allow_name_remap: bool,
        /// The Witcher 3 install whose quest graphs `synth` effects walk (read-only); overrides the run file's
        /// `game_dir`. Default: the Steam folder.
        #[arg(long)]
        game_dir: Option<PathBuf>,
        /// Without --template (and no `template` in the run file) the template is chosen from questbook/templates.toml
        /// and searched in these folders (repeatable; default: the run's `template_dirs`, else `templates` next to the questbook).
        #[arg(long = "template-dir")]
        template_dirs: Vec<PathBuf>,
    },
    /// Phase 0 dev tool: write a NEW save from a template with some facts changed, then re-read and verify it.
    EditFact {
        #[arg(long)]
        template: PathBuf,
        /// `NAME=VALUE`: FactsSet semantics (one permanent record). Repeatable.
        #[arg(long = "set")]
        sets: Vec<String>,
        /// Remove this fact from the table (what the script `FactsRemove` does). Repeatable. The fact must exist.
        /// Applied after `--match-facts-of` and before `--set`.
        #[arg(long = "remove")]
        removes: Vec<String>,
        /// First make every fact that differs from this save's exactly equal to it (copy or remove).
        #[arg(long)]
        match_facts_of: Option<PathBuf>,
        /// Output file name without extension; default is a generated `ManualSave_<a>_<b>_<c>` (a from the template).
        #[arg(long)]
        stem: Option<String>,
        #[arg(long, default_value = ".")]
        out_dir: PathBuf,
    },
    /// List the quest log (`CJournalManager` entries, tracked quest, objective counters).
    Journal {
        sav: PathBuf,
        /// Only entries with this status (`Active`, `JS_Success`, ...).
        #[arg(long)]
        status: Option<String>,
        /// Only entries whose label or GUID path contains this text (case-insensitive).
        #[arg(long)]
        grep: Option<String>,
        /// Also list every objective counter.
        #[arg(long)]
        counters: bool,
    },
    /// Print a journal quest's (or character's) title, description entries and objective titles from the
    /// local game install, in a language. For checking only; nothing is written.
    GameText {
        /// Journal quest/character GUID (4+ hex digits of its start), `.journal` path or file stem, or part
        /// of the English working name.
        key: String,
        /// Game language code (`en`, `ru`, ...).
        #[arg(long, default_value = "en")]
        lang: String,
        /// The Witcher 3 install (default: `W3AL_INSTALL`, else the Steam folder).
        #[arg(long)]
        game_dir: Option<PathBuf>,
    },
    /// Quest-log changes from save A to save B, plus a ready-to-paste questbook TOML block.
    JournalDiff {
        a: PathBuf,
        b: PathBuf,
        /// Only entries (and their counters) whose label or GUID path contains this text.
        #[arg(long)]
        grep: Option<String>,
        /// Effects key used in the TOML block.
        #[arg(long, default_value = "journal_from_diff")]
        key: String,
    },
    /// Gate-J dev tool: write a NEW save from a template with quest-log (and optional fact) edits, then re-read and verify it.
    JournalSet {
        #[arg(long)]
        template: PathBuf,
        /// `ENTRY=STATUS`, ENTRY = index from `w3al journal` or a GUID path (8+ hex digits per step, '/'-separated). Repeatable.
        #[arg(long = "set")]
        sets: Vec<String>,
        /// `DONOR.sav:ENTRY`: copy that entry from another save (appended at the end, or its status set if the template has it). Repeatable.
        #[arg(long = "append-from")]
        append_from: Vec<String>,
        /// A TOML file with `[effects.*]` tables (output of `journal-diff`); journal tables and facts are applied. Repeatable.
        #[arg(long = "from-toml")]
        from_toml: Vec<PathBuf>,
        /// `GUID=COUNT`: set or append an objective counter. Repeatable.
        #[arg(long = "counter")]
        counters: Vec<String>,
        /// GUID of the tracked quest (`JTrackedQuest`).
        #[arg(long)]
        track: Option<String>,
        /// GUID of the highlighted objective (`JHighlightedObjective`).
        #[arg(long)]
        highlight: Option<String>,
        /// `NAME=VALUE`: also set a fact (FactsSet semantics, through `Document::apply`). Repeatable.
        #[arg(long = "fact")]
        facts: Vec<String>,
        /// Append a missing status name (e.g. `JS_Failed`) to the NM table, as the game does between saves. Untested in game.
        #[arg(long)]
        allow_new_names: bool,
        /// Donor save for quest-graph ops (subtrees are copied from it). Same playthrough, later than the template.
        #[arg(long)]
        graph_donor: Option<PathBuf>,
        /// Translate NM name indices of copied subtrees (`graph::NameMode::Remap`): needed when the donor's names do not extend the
        /// template's (a second donor on the same template, another playthrough). In-game untested.
        #[arg(long)]
        graph_remap: bool,
        /// `KIND:PATH` graph op (replace, remove, swap-remove, insert, recount:PATH=of), paths as `graph-diff` prints them. Repeatable.
        #[arg(long = "graph-op")]
        graph_ops: Vec<String>,
        /// A TOML file with `[effects.*.graph]` tables (output of `graph-diff --toml`). Repeatable.
        #[arg(long = "graph-from-toml")]
        graph_from_toml: Vec<PathBuf>,
        /// A swap file (`[swap]`, docs/findings/branch-synthesis.md): rewrite the delta from one output of a decision
        /// to a sibling output (graph op paths, journal entries, fact names; the donor's waiting blocks are rekeyed). Repeatable.
        #[arg(long = "swap")]
        swaps: Vec<PathBuf>,
        /// Output file name without extension; default is a generated `ManualSave_<a>_<b>_<c>` (a from the template).
        #[arg(long)]
        stem: Option<String>,
        #[arg(long, default_value = ".")]
        out_dir: PathBuf,
    },
    /// Read the player's level, XP, skill points, vitality, crowns and item list.
    Character {
        sav: PathBuf,
        /// Also list every item record (name, quantity, durability).
        #[arg(long)]
        items: bool,
    },
    /// Write a NEW save from a template with a start kit (level, crowns, supplies), then re-read it.
    CharacterSet {
        #[arg(long)]
        template: PathBuf,
        #[arg(long)]
        level: Option<u32>,
        #[arg(long)]
        crowns: Option<u32>,
        /// Top up the recommended supplies the template already carries.
        #[arg(long)]
        gear: bool,
        #[arg(long)]
        stem: Option<String>,
        #[arg(long, default_value = ".")]
        out_dir: PathBuf,
    },
    /// Print a top-level block (`questSystem`, `community`, ...) as a name-resolved tree.
    Tree {
        sav: PathBuf,
        #[arg(long, default_value = "questSystem")]
        block: String,
        /// Only lines whose path or text contains this (printed with full paths).
        #[arg(long)]
        grep: Option<String>,
        #[arg(long)]
        depth: Option<usize>,
        /// Print only the subtree at this path (as `graph-diff` prints paths) and its hash (the guard value of graph TOML ops).
        #[arg(long)]
        path: Option<String>,
    },
    /// Keyed tree diff of top-level blocks between two saves (default: questSystem, community, gameProp, worldState, universe).
    GraphDiff {
        a: PathBuf,
        b: PathBuf,
        /// Block to compare. Repeatable.
        #[arg(long = "block")]
        blocks: Vec<String>,
        #[arg(long)]
        grep: Option<String>,
        /// Also print the changes as a replayable `[effects.<KEY>.graph]` TOML block.
        #[arg(long)]
        toml: Option<String>,
        /// Only changes under this path go into the TOML block. Repeatable.
        #[arg(long = "scope")]
        scopes: Vec<String>,
        /// Key `community > stubs > a` by their `idTag` (`a{idTag=..}`) instead of by position (`a#k`); stub order is then not reported.
        #[arg(long)]
        stub_id: bool,
    },
    /// Check graph-delta TOML ops against other saves (templates): per op, does the save hold the node the delta was captured on?
    /// B = matches the `before` guard (applies), A = already equals the donor (`after`) or already gone, - = absent, x = present but different.
    GraphCheck {
        /// TOML file(s) with `[effects.*.graph]` tables.
        #[arg(long = "toml", required = true)]
        tomls: Vec<PathBuf>,
        /// Saves to check.
        #[arg(required = true)]
        saves: Vec<PathBuf>,
        /// Print every op (default: only ops that do not apply cleanly on some save).
        #[arg(long)]
        all: bool,
        /// The template the delta was captured on: also report, per quest-graph scope the ops touch, the blocks/threads a save has
        /// there that the template had not (or lacks), beyond the ops' own nodes (state the delta does not cover: e.g. a quest's
        /// start trigger still waiting on an earlier save).
        #[arg(long)]
        base: Option<PathBuf>,
    },
    /// List `community > stubs` with their identity: idTag (the selector of `graph-diff --stub-id`), spawn set, appearance, position.
    Stubs {
        sav: PathBuf,
        /// Only stubs whose line contains this text (case-insensitive).
        #[arg(long)]
        grep: Option<String>,
    },
    /// Print the questbook as the UI receives it (used to preview ui/ in a browser).
    QuestbookJson {
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Which registered template a run gets (questbook/templates.toml), which earlier decisions are rewritten,
    /// which cannot be and which quests fall after the template (docs/plan/story-chain.md).
    ChainPlan {
        #[arg(long)]
        run: PathBuf,
        #[arg(long)]
        questbook: Option<PathBuf>,
        /// Folder holding template saves; repeatable. Default: the run's `template_dirs`, then the shipped `templates` next to the questbook.
        #[arg(long = "template-dir")]
        template_dirs: Vec<PathBuf>,
    },
    /// For every registered template: the answers it holds for each earlier window (every answer combination
    /// exported on that window's own template and matched by facts) and the readers of the deciding facts with
    /// their state in the save. Prints `[template.encodes]` blocks for questbook/templates.toml.
    TemplateScan {
        #[arg(long)]
        questbook: Option<PathBuf>,
        #[arg(long = "template-dir")]
        template_dirs: Vec<PathBuf>,
        /// Only print this template id (all windows are still built).
        #[arg(long)]
        id: Option<String>,
        /// Refuse windows with more answer combinations than this.
        #[arg(long, default_value_t = 32)]
        max: usize,
        #[arg(long = "donors")]
        donors: Vec<PathBuf>,
        #[arg(long)]
        allow_name_remap: bool,
        #[arg(long)]
        game_dir: Option<PathBuf>,
    },
    /// Edit mode: read one of your own saves and print where it stands in the questbook (quests done / active /
    /// not reached, the answer it already holds for every decision, what can be changed and why not). Read-only.
    EditPlan {
        sav: PathBuf,
        #[arg(long)]
        questbook: Option<PathBuf>,
        /// Folder holding the registered template saves (repeatable; default: `templates` next to the questbook).
        #[arg(long = "template-dir")]
        template_dirs: Vec<PathBuf>,
        #[arg(long = "donors")]
        donors: Vec<PathBuf>,
        #[arg(long)]
        allow_name_remap: bool,
        /// The Witcher 3 install whose quest graphs are read (default: the Steam folder).
        #[arg(long)]
        game_dir: Option<PathBuf>,
        /// Scan cache file (default `%LOCALAPPDATA%\w3al\edit-scan-cache.json`); the first run builds every window's
        /// answer combinations (minutes), later runs read this file.
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Edit mode: change answers, level, crowns or add the recommended kit on one of your own saves and write a NEW
    /// save in --out-dir (the opened save is never touched). Decisions the save already holds are rewritten after
    /// the fact where `w3al edit-plan` says they can be; the next undecided window is applied with its own effects.
    Edit {
        #[arg(long)]
        save: PathBuf,
        /// `quest.choice=option`, repeatable.
        #[arg(long = "set")]
        sets: Vec<String>,
        #[arg(long)]
        level: Option<u32>,
        #[arg(long)]
        crowns: Option<u32>,
        /// `recommended`: add the recommended kit.
        #[arg(long)]
        gear: Option<String>,
        /// One item of the recommended table by name (repeatable), e.g. `Swallow 1`.
        #[arg(long = "item")]
        items: Vec<String>,
        #[arg(long)]
        out_dir: PathBuf,
        /// Output file stem; default a generated game-style `ManualSave_<a>_<b>_<c>`.
        #[arg(long)]
        stem: Option<String>,
        #[arg(long)]
        questbook: Option<PathBuf>,
        #[arg(long = "template-dir")]
        template_dirs: Vec<PathBuf>,
        #[arg(long = "donors")]
        donors: Vec<PathBuf>,
        #[arg(long)]
        allow_name_remap: bool,
        #[arg(long)]
        game_dir: Option<PathBuf>,
        #[arg(long)]
        cache: Option<PathBuf>,
    },
    /// Walk a decision through the game's quest graphs from a template save and print what the game would write
    /// (facts, journal, spawn phases, the new waits); with --donor, compare with a save really played from the template.
    /// Graphs come from the player's own install (--install) or a folder of extracted .w2phase/.w2quest files (--graphs).
    SynthWalk {
        #[arg(long)]
        template: PathBuf,
        #[arg(long = "graphs")]
        graphs: Vec<PathBuf>,
        #[arg(long)]
        install: Option<PathBuf>,
        /// Block to fire, `GUID[:input]` (8 or 32 hex digits; the input defaults to the saved one). Repeatable.
        #[arg(long = "start")]
        starts: Vec<String>,
        /// Output socket to take at a block, `GUID=socket` (the decision). Repeatable.
        #[arg(long = "pick")]
        picks: Vec<String>,
        /// Choice line to take inside a scene, `GUID=section` (the name of the section the line leads to; the walk lists the
        /// candidates as MAYBE / unresolved). Repeatable.
        #[arg(long = "scene-pick")]
        scene_picks: Vec<String>,
        /// Wait to park at (the end of the decision window). Repeatable.
        #[arg(long = "stop")]
        stops: Vec<String>,
        /// Assume single-output waits satisfied (the player did them) and go on until a stop, a pick-less decision or the quest's end.
        #[arg(long)]
        through: bool,
        /// Do not fire the template's own waiting pause blocks whose fact conditions the walk made true.
        #[arg(long)]
        no_listeners: bool,
        /// A save played from the template: compare the prediction with it.
        #[arg(long)]
        donor: Option<PathBuf>,
        /// Print the template's waiting blocks with the graph's description (optionally filtered by this text).
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        show_frontier: Option<String>,
        #[arg(long)]
        verbose: bool,
        #[arg(long, default_value_t = 20000)]
        max_steps: usize,
    },
    /// Like synth-walk, then write the walk into a copy of the template (no donor; new `ManualSave_*` in --out-dir) and verify it;
    /// with --donor, report how the result differs from the played save in the touched scopes, the journal and the facts.
    SynthApply {
        #[arg(long)]
        template: PathBuf,
        #[arg(long = "graphs")]
        graphs: Vec<PathBuf>,
        #[arg(long)]
        install: Option<PathBuf>,
        #[arg(long = "start")]
        starts: Vec<String>,
        #[arg(long = "pick")]
        picks: Vec<String>,
        #[arg(long = "scene-pick")]
        scene_picks: Vec<String>,
        #[arg(long = "stop")]
        stops: Vec<String>,
        #[arg(long)]
        through: bool,
        #[arg(long)]
        no_listeners: bool,
        #[arg(long)]
        donor: Option<PathBuf>,
        #[arg(long)]
        verbose: bool,
        #[arg(long, default_value_t = 20000)]
        max_steps: usize,
        #[arg(long)]
        out_dir: PathBuf,
        /// Output stem (default: the template stem advanced by the save-name rule).
        #[arg(long)]
        stem: Option<String>,
    },
    /// Describe a dialogue scene (`.w2scene` path as the graphs name it): inputs, outputs, choices with the section names a
    /// `--scene-pick` may name, facts set; with --from/--to the facts on the way between an input and an output.
    SceneInfo {
        #[arg(long = "graphs")]
        graphs: Vec<PathBuf>,
        #[arg(long)]
        install: Option<PathBuf>,
        scene: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// Section names to follow at choices (as `--scene-pick` gives them, without the GUID). Repeatable.
        #[arg(long = "pick")]
        picks: Vec<String>,
        /// Also list the decoded events of every section.
        #[arg(long)]
        verbose: bool,
    },
    /// Describe quest-graph block GUIDs (as `graph-diff` prints them) from the game's graphs.
    /// Research: print the exports of side resources (`.journal`, `.w2comm`, `.w2scene`) whose name contains NAME.
    ResDump {
        #[arg(long)]
        install: PathBuf,
        name: String,
        #[arg(long, default_value_t = 5)]
        max: usize,
    },
    /// What the install says about a quest (journal entry, the graph blocks that write it, files, importers);
    /// with --template, its state in that save. KEY = quest GUID (8/32 hex) or part of its internal name or journal file.
    QuestInfo {
        #[arg(long)]
        install: PathBuf,
        #[arg(long)]
        template: Option<PathBuf>,
        #[arg(required = true)]
        keys: Vec<String>,
        /// One line per quest.
        #[arg(long)]
        brief: bool,
    },
    /// Auto-complete quests on a template (w3graph::complete: the walker finishes each quest to its Success and end by
    /// its own deterministic policy and lists its picks); each quest starts from the previous one's result. With
    /// --out-dir, write the result as a NEW save. KEY = journal root GROUP/QUEST, quest GUID, or part of the internal name.
    SideComplete {
        #[arg(long)]
        install: PathBuf,
        #[arg(long)]
        template: PathBuf,
        #[arg(required = true)]
        keys: Vec<String>,
        #[arg(long)]
        out_dir: Option<PathBuf>,
        #[arg(long)]
        verbose: bool,
    },
    /// Write questbook/side: one TOML per side quest (contracts, treasure and scavenger hunts, Gwent, races, fist fights,
    /// and main quests between decisions under side/main) that the auto-complete walk finishes on at least one window
    /// template of questbook/templates.toml; prints the totals per template and the refusals by reason. Identifiers
    /// only: titles are read from the install at run time.
    GenSideQuests {
        #[arg(long)]
        install: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Folders holding the registry's template saves (repeatable).
        #[arg(long = "template-dir")]
        template_dirs: Vec<PathBuf>,
        #[arg(long)]
        questbook: Option<PathBuf>,
        /// Print the totals only; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Research: the token structure under a path (`w3al tree` paths), raw parts and nodes inside opaque values included.
    NodeDump {
        sav: PathBuf,
        path: String,
        #[arg(long, default_value_t = 4)]
        depth: usize,
    },
    /// Validation: every quest done (Success) in B and not in A is auto-completed on A alone and compared with B
    /// (the quest's journal entries, its facts, its phase gone, spawn sets). One line per quest, HIT / MISS / REFUSED.
    SideCheck {
        #[arg(long)]
        install: PathBuf,
        a: PathBuf,
        b: PathBuf,
        /// Only quests whose journal file or internal name contains this.
        #[arg(long)]
        grep: Option<String>,
        #[arg(long)]
        verbose: bool,
    },
    GraphLookup {
        #[arg(long = "graphs")]
        graphs: Vec<PathBuf>,
        #[arg(long)]
        install: Option<PathBuf>,
        #[arg(required = true)]
        keys: Vec<String>,
    },
}

fn main() -> ExitCode {
    // The command enum is large: parsing it and dispatching in a debug build needs more than the 1 MB main-thread
    // stack of Windows, so the whole run happens on a thread with room to spare.
    let worker = std::thread::Builder::new().stack_size(64 << 20).spawn(|| match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    });
    match worker.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => code,
        _ => ExitCode::FAILURE,
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.cmd {
        Cmd::Dump { sav, json } => dump(&sav, json),
        Cmd::Roundtrip { sav } => roundtrip(&sav),
        Cmd::Diff { a, b } => diff(&a, &b),
        Cmd::Facts { sav, diff, grep } => facts_cmd::facts_cmd(&sav, diff.as_deref(), grep.as_deref()),
        Cmd::Lint { dir } => lint(dir),
        Cmd::CheckSave { sav } => check_save(&sav),
        Cmd::Export {
            run,
            template,
            out_dir,
            questbook,
            dry_run,
            stem,
            allow_placeholders,
            donors,
            allow_name_remap,
            game_dir,
            template_dirs,
        } => {
            let opts = export::ExportOptions { stem, allow_placeholders, donor_dirs: donors, allow_name_remap, game_dir, start_kit: None };
            export_cmd(&run, template, &out_dir, questbook, dry_run, &opts, &template_dirs)
        }
        Cmd::EditFact { template, sets, removes, match_facts_of, stem, out_dir } => edit_cmd::edit_fact_cmd(&edit_cmd::EditArgs {
            template: &template,
            sets: &sets,
            removes: &removes,
            match_facts_of: match_facts_of.as_deref(),
            stem: stem.as_deref(),
            out_dir: &out_dir,
        }),
        Cmd::Journal { sav, status, grep, counters } => {
            journal_cmd::journal_cmd(&sav, status.as_deref(), grep.as_deref(), counters)
        }
        Cmd::GameText { key, lang, game_dir } => gametext_cmd::game_text_cmd(&key, &lang, game_dir.as_deref()).map(|()| ExitCode::SUCCESS),
        Cmd::JournalDiff { a, b, grep, key } => journal_cmd::journal_diff_cmd(&a, &b, grep.as_deref(), &key),
        Cmd::JournalSet {
            template,
            sets,
            append_from,
            from_toml,
            counters,
            track,
            highlight,
            facts,
            allow_new_names,
            graph_donor,
            graph_remap,
            graph_ops,
            graph_from_toml,
            swaps,
            stem,
            out_dir,
        } => journal_cmd::journal_set_cmd(&journal_cmd::SetArgs {
            template: &template,
            sets: &sets,
            append_from: &append_from,
            from_toml: &from_toml,
            counters: &counters,
            track: track.as_deref(),
            highlight: highlight.as_deref(),
            facts: &facts,
            allow_new_names,
            graph_donor: graph_donor.as_deref(),
            graph_remap,
            graph_ops: &graph_ops,
            graph_from_toml: &graph_from_toml,
            swaps: &swaps,
            stem: stem.as_deref(),
            out_dir: &out_dir,
        }),
        Cmd::Character { sav, items } => character_cmd::character_cmd(&sav, items),
        Cmd::CharacterSet { template, level, crowns, gear, stem, out_dir } => {
            let kit = w3al_core::start_kit::StartKit {
                level,
                crowns,
                gear: if gear { w3al_core::start_kit::Gear::Recommended } else { w3al_core::start_kit::Gear::None },
                items: Vec::new(),
            };
            character_cmd::character_set_cmd(&template, &kit, stem.as_deref(), &out_dir)
        }
        Cmd::Tree { sav, block, grep, depth, path } => graph_cmd::tree_cmd(&sav, &block, grep.as_deref(), depth, path.as_deref()),
        Cmd::SynthWalk { template, graphs, install, starts, picks, scene_picks, stops, through, no_listeners, donor, show_frontier, verbose, max_steps } => {
            synth_cmd::synth_walk_cmd(&synth_cmd::WalkArgs {
                graphs: &graphs,
                install: install.as_deref(),
                template: &template,
                starts: &starts,
                picks: &picks,
                scene_picks: &scene_picks,
                stops: &stops,
                through,
                no_listeners,
                donor: donor.as_deref(),
                show_frontier: show_frontier.as_deref(),
                verbose,
                max_steps,
            })
        }
        Cmd::SynthApply { template, graphs, install, starts, picks, scene_picks, stops, through, no_listeners, donor, verbose, max_steps, out_dir, stem } => synth_cmd::synth_apply_cmd(
            &synth_cmd::WalkArgs {
                graphs: &graphs,
                install: install.as_deref(),
                template: &template,
                starts: &starts,
                picks: &picks,
                scene_picks: &scene_picks,
                stops: &stops,
                through,
                no_listeners,
                donor: donor.as_deref(),
                show_frontier: None,
                verbose,
                max_steps,
            },
            &out_dir,
            stem.as_deref(),
        ),
        Cmd::GraphLookup { graphs, install, keys } => synth_cmd::graph_lookup_cmd(&graphs, install.as_deref(), &keys),
        Cmd::ResDump { install, name, max } => side_cmd::res_dump_cmd(&install, &name, max),
        Cmd::QuestInfo { install, template, keys, brief } => side_cmd::quest_info_cmd(&install, template.as_deref(), &keys, brief),
        Cmd::SideComplete { install, template, keys, out_dir, verbose } => side_cmd::side_complete_cmd(&install, &template, &keys, out_dir.as_deref(), verbose),
        Cmd::SideCheck { install, a, b, grep, verbose } => side_cmd::side_check_cmd(&install, &a, &b, grep.as_deref(), verbose),
        Cmd::NodeDump { sav, path, depth } => side_cmd::node_dump_cmd(&sav, &path, depth),
        Cmd::GenSideQuests { install, out, template_dirs, questbook, dry_run } => {
            side_cmd::gen_side_quests_cmd(&side_cmd::GenArgs { install: &install, out: &out, template_dirs: &template_dirs, questbook, dry_run })
        }
        Cmd::SceneInfo { graphs, install, scene, from, to, picks, verbose } => synth_cmd::scene_info_cmd(&graphs, install.as_deref(), &scene, from.as_deref(), to.as_deref(), &picks, verbose),
        Cmd::GraphCheck { tomls, saves, all, base } => graph_cmd::graph_check_cmd(&tomls, &saves, all, base.as_deref()),
        Cmd::Stubs { sav, grep } => graph_cmd::stubs_cmd(&sav, grep.as_deref()),
        Cmd::GraphDiff { a, b, blocks, grep, toml, scopes, stub_id } => graph_cmd::graph_diff_cmd(&graph_cmd::DiffArgs {
            stub_id,
            a: &a,
            b: &b,
            blocks: &blocks,
            grep: grep.as_deref(),
            toml: toml.as_deref(),
            scopes: &scopes,
        }),
        Cmd::QuestbookJson { dir } => {
            let book = load_book(dir)?;
            println!("{}", serde_json::to_string_pretty(&QuestbookView::new(&book))?);
            Ok(ExitCode::SUCCESS)
        }
        Cmd::ChainPlan { run, questbook, template_dirs } => chain_cmd::chain_plan(&run, questbook, &template_dirs),
        Cmd::EditPlan { sav, questbook, template_dirs, donors, allow_name_remap, game_dir, cache, json } => {
            let opts = export::ExportOptions { donor_dirs: donors, allow_name_remap, game_dir, ..Default::default() };
            edit_save_cmd::edit_plan(&sav, &edit_save_cmd::EngineArgs { questbook, template_dirs, opts, cache }, json)
        }
        Cmd::Edit { save, sets, level, crowns, gear, items, out_dir, stem, questbook, template_dirs, donors, allow_name_remap, game_dir, cache } => {
            let opts = export::ExportOptions { donor_dirs: donors, allow_name_remap, game_dir, ..Default::default() };
            edit_save_cmd::edit(
                &edit_save_cmd::EditArgs { save: &save, sets: &sets, level, crowns, gear: gear.as_deref(), items: &items, out_dir: &out_dir, stem: stem.as_deref() },
                &edit_save_cmd::EngineArgs { questbook, template_dirs, opts, cache },
            )
        }
        Cmd::TemplateScan { questbook, template_dirs, id, max, donors, allow_name_remap, game_dir } => {
            let opts = export::ExportOptions { donor_dirs: donors, allow_name_remap, game_dir, ..Default::default() };
            chain_cmd::template_scan(questbook, &template_dirs, id, max, &opts)
        }
    }
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("reading {}", path.display()))
}

fn load_book(dir: Option<PathBuf>) -> Result<Questbook> {
    let dir = paths::locate_questbook(dir).context("no questbook directory found (use --dir or W3AL_QUESTBOOK)")?;
    Ok(Questbook::load_dir(&dir)?)
}

fn dump(path: &Path, json: bool) -> Result<ExitCode> {
    let info = w3save::inspect(&read(path)?)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(ExitCode::SUCCESS);
    }
    println!("file size      {} bytes", info.file_size);
    println!("chunks         {} (data start {})", info.chunk_count, info.data_start);
    println!("payload        {} bytes decoded", info.payload_size);
    match (&info.payload, &info.payload_error) {
        (Some(m), _) => {
            println!(
                "header ints    {:?}  (looks like 5.x: {})",
                m.header_ints, info.looks_like_5x
            );
            println!(
                "pointers       variable table @{}  NM @{}  RB @{}  pre-table bytes {:02x?}",
                m.variable_table_offset, m.nm_offset, m.rb_offset, m.pre_table_bytes
            );
            println!("variables      {}", m.variables.len());
            for (tag, n) in &info.variable_tags {
                println!("  {tag:<6} {n}");
            }
        }
        (None, Some(e)) => println!("payload map    FAILED: {e}"),
        _ => {}
    }
    match (&info.name_count, &info.name_error) {
        (Some(n), _) => println!("names (NM)     {n}"),
        (None, Some(e)) => println!("names (NM)     not read: {e}"),
        _ => {}
    }
    if let Some(n) = info.rb_count {
        println!("RB entries     {n}");
    }
    match (&info.fact_count, &info.fact_error) {
        (Some(n), _) => println!("facts          {n}"),
        (None, Some(e)) => println!("facts          not read: {e}"),
        _ => {}
    }
    print_tokens(&info);
    print_issues(&info.issues);
    Ok(ExitCode::SUCCESS)
}

/// Typed-tokenizer coverage: how many variable-table entries became typed nodes.
fn print_tokens(info: &w3save::SaveInfo) {
    match (&info.tokens, &info.tokens_error) {
        (Some(t), _) => {
            println!(
                "typed nodes    {} of {} ({:.2}%), typed round trip {}",
                t.typed,
                t.total,
                t.typed_percent(),
                info.tokens_roundtrip.map_or("?", |b| if b { "PASS" } else { "FAIL" })
            );
            for (tag, s) in &t.by_tag {
                println!("  {tag:<6} typed {:>8} raw {:>8} values decoded {:>8}", s.typed, s.raw, s.value_decoded);
            }
            println!("  top-level      {:?}", t.top_level);
            for (why, n) in &t.raw_reasons {
                println!("  raw: {why} x {n}");
            }
            let mut opaque: Vec<_> = t.opaque_types.iter().collect();
            opaque.sort_by_key(|x| std::cmp::Reverse(*x.1));
            let top: Vec<String> = opaque.iter().take(8).map(|(k, v)| format!("{k} x{v}")).collect();
            println!("  opaque values  {} types; top: {}", opaque.len(), top.join(", "));
        }
        (None, Some(e)) => println!("typed nodes    not built: {e}"),
        _ => {}
    }
}

fn print_issues(issues: &[w3save::Issue]) {
    for i in issues {
        let tag = match i.severity {
            Severity::Info => "info ",
            Severity::Warning => "warn ",
            Severity::Error => "ERROR",
        };
        println!("[{tag}] {}", i.message);
    }
}

fn roundtrip(path: &Path) -> Result<ExitCode> {
    let file = read(path)?;
    let container = Container::read(&file)?;
    let mut ok = true;

    // 1. Payload-level: parse into a document, serialize, compare decoded bytes.
    let doc = Document::parse(&container.payload, container.data_start)?;
    let same = doc.to_bytes() == container.payload;
    println!("payload parse -> serialize identical : {}", verdict(same));
    ok &= same;

    // 1b. Offsets: rebuilding the table, RB, pointer block and footer from the tree changes nothing.
    let mut rebased = doc.clone();
    let same = rebased.rebase().is_ok() && rebased.to_bytes() == container.payload;
    println!("rebase on the unedited tree is a no-op: {}", verdict(same));
    ok &= same;

    // 2. Container-level: our compressor's output must decode to the same payload.
    let rewritten = container.write()?;
    let again = Container::read(&rewritten)?;
    let same = again.payload == container.payload;
    println!("re-compressed file decodes identical : {}", verdict(same));
    ok &= same;

    // 3. Informational only: LZ4 encoders differ, so this is usually false.
    println!(
        "compressed bytes equal to original   : {} (informational; not a goal)",
        file == rewritten
    );
    println!("original {} bytes, rewritten {} bytes", file.len(), rewritten.len());

    for i in container.diagnostics() {
        println!("[note] {}", i.message);
    }
    println!("\nPhase 0 container + region round trip: {}", verdict(ok));
    println!("(Typed token tree since PHASE0 step 7; `w3al dump` shows coverage. In-game load check is step 9.)");
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn verdict(ok: bool) -> &'static str {
    if ok {
        "PASS"
    } else {
        "FAIL"
    }
}

fn diff(a: &Path, b: &Path) -> Result<ExitCode> {
    let (ca, cb) = (Container::read(&read(a)?)?, Container::read(&read(b)?)?);
    println!("payload size   {} -> {} ({:+})", ca.payload.len(), cb.payload.len(), cb.payload.len() as i64 - ca.payload.len() as i64);
    println!("chunks         {} -> {}", ca.records.len(), cb.records.len());

    let (da, db) = (
        Document::parse(&ca.payload, ca.data_start)?,
        Document::parse(&cb.payload, cb.data_start)?,
    );
    if da.map.header_ints != db.map.header_ints {
        println!("header ints    {:?} -> {:?}", da.map.header_ints, db.map.header_ints);
    }
    println!("variables      {} -> {}", da.map.variables.len(), db.map.variables.len());

    if da.regions.len() != db.regions.len() {
        println!(
            "region count differs ({} vs {}): the variable layout changed; per-region diff skipped",
            da.regions.len(),
            db.regions.len()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let mut changed = 0usize;
    for (i, (ra, rb)) in da.regions.iter().zip(&db.regions).enumerate() {
        if ra == rb {
            continue;
        }
        changed += 1;
        if changed > 60 {
            continue;
        }
        let (ba, bb) = (ra.to_bytes(), rb.to_bytes());
        let first = ba.iter().zip(&bb).position(|(x, y)| x != y).unwrap_or(ba.len().min(bb.len()));
        let label = match ra {
            Region::Variable { index, .. } => format!("variable #{index} [{}]", w3save::payload::tag_of(&ba)),
            Region::Gap(_) => "gap".to_string(),
        };
        println!("  region {i:>6} {label:<24} {} -> {} bytes, first difference at +{first}", ba.len(), bb.len());
    }
    println!("{changed} region(s) differ");
    println!("(Fact-level diff arrives with the facts parser in Phase 1.)");
    Ok(ExitCode::SUCCESS)
}

fn lint(dir: Option<PathBuf>) -> Result<ExitCode> {
    let qb = paths::locate_questbook(dir).context("no questbook directory found (use --dir or W3AL_QUESTBOOK)")?;
    let book = Questbook::load_dir(&qb)?;
    let mut diags = validate::lint(&book);
    // The template registry next to the quests (questbook/templates.toml).
    match w3al_core::templates::Registry::load_for_questbook(&qb) {
        Ok(reg) => diags.extend(reg.lint(&book)),
        Err(e) => diags.push(w3al_core::Diagnostic::error(None, e)),
    }
    for d in &diags {
        let tag = if d.is_error() { "ERROR" } else { "warn " };
        println!("[{tag}] {}: {}", d.quest.as_deref().unwrap_or("-"), d.message);
    }
    let errors = diags.iter().filter(|d| d.is_error()).count();
    println!("{} quests, {errors} error(s), {} warning(s)", book.quests.len(), diags.len() - errors);
    Ok(if errors == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn check_save(path: &Path) -> Result<ExitCode> {
    let info = w3save::inspect(&read(path)?)?;
    print_issues(&info.issues);
    let errors = info.issues.iter().filter(|i| i.severity == Severity::Error).count();
    if info.issues.is_empty() {
        println!("no problems found (container, name table and fact ids)");
    }
    Ok(if errors == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn export_cmd(
    run_path: &Path,
    template: Option<PathBuf>,
    out_dir: &Path,
    qb: Option<PathBuf>,
    dry: bool,
    opts: &export::ExportOptions,
    template_dirs: &[PathBuf],
) -> Result<ExitCode> {
    let run = Run::load(run_path)?;
    let qb_dir = qb.clone();
    let book = load_book(qb)?;
    let res = resolve(&book, &run);

    for d in &res.diagnostics {
        let tag = if d.severity == CSev::Error { "ERROR" } else { "warn " };
        println!("[{tag}] {}: {}", d.quest.as_deref().unwrap_or("-"), d.message);
    }
    println!(
        "{} completed, {} started, {} blocked, {} effect block(s)",
        res.completed.len(),
        res.started.len(),
        res.blocked.len(),
        res.effects.len()
    );
    for a in &res.effects {
        let e = &a.effect;
        let journal = e.journal_status.len() + e.journal_append.len() + e.journal_counters.len() + usize::from(e.journal_track.is_some());
        let graph = e.graph.as_ref().map_or_else(String::new, |g| {
            format!(", {} graph op(s){}", g.ops.len(), g.donor.as_ref().map(|d| format!(" from {d}")).unwrap_or_default())
        });
        let synth = e.synth.as_ref().map_or_else(String::new, |s| {
            format!(", synth start {:?} pick {:?} stop {:?}{}", s.start, s.pick, s.stop, if s.through { " through" } else { "" })
        });
        println!(
            "  {}/{}: {} fact(s), {} journal edit(s){graph}{synth}{}{}",
            a.quest,
            a.origin,
            e.facts.len(),
            journal,
            if e.quests.is_empty() && e.journal.is_empty() { "" } else { "  [LEGACY quests/journal ids: refused]" },
            if e.placeholder { "  [PLACEHOLDER]" } else { "" }
        );
    }
    if dry {
        return Ok(if res.has_errors() { ExitCode::FAILURE } else { ExitCode::SUCCESS });
    }

    // Donor folders: the run file's (relative to the run file) plus --donors.
    let run_dir = run_path.parent().unwrap_or(Path::new("."));
    let mut opts = opts.clone();
    let from_run = export::ExportOptions::for_run(&run);
    opts.donor_dirs = from_run.donor_dirs.iter().map(|d| run_dir.join(d)).chain(opts.donor_dirs).collect();
    opts.allow_name_remap |= from_run.allow_name_remap;
    opts.game_dir = opts.game_dir.or(from_run.game_dir);
    opts.start_kit = from_run.start_kit;
    // No template given: the story chain picks one from the registry (manual override otherwise).
    let Some(template) = template.or_else(|| run.template.clone().map(PathBuf::from)) else {
        return chain_cmd::story_export(run_path, &run, qb_dir, template_dirs, out_dir, &opts);
    };
    match export::export_with(&template, out_dir, &res, &opts) {
        Ok(r) => {
            for n in &r.notes {
                println!("  applied {n}");
            }
            println!("wrote {}", r.sav.display());
            for s in r.sidecars {
                println!("      {}", s.display());
            }
            println!("VERIFIED: re-parsed, table and post-order hold, check-save clean, copied subtrees equal their donors, journal and facts as requested, nothing else changed; file re-read");
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => bail!("{e}"),
    }
}
