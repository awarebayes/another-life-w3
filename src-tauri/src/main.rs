// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri shell: thin `invoke` commands over `w3save` and `w3al-core`. The backend is
//! stateless; the frontend holds the whole run as one JSON object. Everything about the
//! save format lives in the crates; this file only reads, summarizes and forwards.
mod wikicache;

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use w3al_core::export::{self, ExportError, ExportOptions};
use w3al_core::{paths, resolve, Questbook, QuestbookView, Resolution, Run};
use w3save::{graph, journal, Container, Document, Issue};

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn questbook_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    // Installed builds bundle `questbook/` as a resource; dev checkouts find it by walking up.
    let bundled = app.path().resource_dir().ok().map(|d| d.join("questbook"));
    Ok(paths::locate_questbook(bundled).ok_or("questbook directory not found")?)
}

fn load_book(app: &AppHandle) -> CmdResult<Questbook> {
    Questbook::load_dir(&questbook_dir(app)?).map_err(err)
}

// ---------------------------------------------------------------------------------------------
// Story chain: the template is chosen from questbook/templates.toml (docs/plan/story-chain.md)

/// What the Welcome screen shows about the automatic template choice.
#[derive(Debug, Serialize)]
struct TemplatePlan {
    /// Folders searched for the registry's template saves.
    dirs: Vec<String>,
    /// The choice, or `None` with `error` when no registered template fits / is found (the app then asks for a file).
    choice: Option<w3al_core::templates::Choice>,
    error: Option<String>,
    /// Every registered template and whether its file is in the folders.
    templates: Vec<TemplateFound>,
}

#[derive(Debug, Serialize)]
struct TemplateFound {
    id: String,
    line: u32,
    stem: String,
    quests: Vec<String>,
    found: bool,
}

fn story_sources(app: &AppHandle, run: &Run) -> CmdResult<(Questbook, w3al_core::templates::Registry, Vec<PathBuf>)> {
    let qb = questbook_dir(app)?;
    let book = Questbook::load_dir(&qb).map_err(err)?;
    let reg = w3al_core::templates::Registry::load_for_questbook(&qb)?;
    let mut dirs = w3al_core::templates::search_dirs(run, Path::new("."), &qb);
    // Installed builds bundle `templates/` as a resource beside `questbook/`.
    if let Some(res) = app.path().resource_dir().ok().map(|d| d.join(w3al_core::templates::DEFAULT_DIR)) {
        if !dirs.contains(&res) {
            dirs.push(res);
        }
    }
    Ok((book, reg, dirs))
}

/// Which template the run gets (latest window template at or before its stop point), which earlier answers are
/// rewritten, which cannot be and which quests fall after the save. Used when the run names no template.
#[tauri::command]
fn template_plan(app: AppHandle, run: Run) -> CmdResult<TemplatePlan> {
    let (book, reg, dirs) = story_sources(&app, &run)?;
    let found = |t: &w3al_core::templates::Template| w3al_core::templates::find_file(&dirs, &t.stem);
    let (choice, error) = match w3al_core::templates::choose(&book, &reg, &run, &found) {
        Ok(c) => (Some(c), None),
        Err(e) => (None, Some(e)),
    };
    let templates = reg
        .templates
        .iter()
        .map(|t| TemplateFound { id: t.id.clone(), line: t.line, stem: t.stem.clone(), quests: t.quests.clone(), found: found(t).is_some() })
        .collect();
    Ok(TemplatePlan { dirs: dirs.iter().map(|d| d.display().to_string()).collect(), choice, error, templates })
}

/// The Side quests page: which side quests the run's template (picked, or chosen from the registry) already has
/// done; those are shown checked and disabled.
#[derive(Debug, Default, Serialize)]
struct SideStates {
    template: Option<String>,
    template_id: Option<String>,
    done: Vec<String>,
    error: Option<String>,
}

#[tauri::command]
async fn side_quest_states(app: AppHandle, run: Run) -> CmdResult<SideStates> {
    let (book, reg, dirs) = story_sources(&app, &run)?;
    blocking(move || {
        let mut out = SideStates::default();
        let path = match run.template.as_ref().filter(|t| !t.trim().is_empty()) {
            Some(t) => Some(PathBuf::from(t)),
            None => match w3al_core::templates::choose(&book, &reg, &run, &|t| w3al_core::templates::find_file(&dirs, &t.stem)) {
                Ok(c) => {
                    out.template_id = Some(c.template.id.clone());
                    c.path
                }
                Err(e) => {
                    out.error = Some(e);
                    None
                }
            },
        };
        let Some(p) = path else { return out };
        out.template = Some(p.display().to_string());
        match std::fs::read(&p).map_err(err).and_then(|b| w3al_core::side::done_in_save(&book.side, &b)) {
            Ok(d) => out.done = d,
            Err(e) => out.error = Some(e),
        }
        out
    })
    .await
}

/// Run blocking work (reading and building multi-MB saves) off the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)
}

// ---------------------------------------------------------------------------------------------
// Template save summary

/// What the Welcome screen shows about a save. A compact view of `w3save::inspect` plus a few
/// read-only lookups (journal size, tracked quest, world name). Each lookup is optional: a save
/// whose payload does not parse still gets the container facts and the issues.
#[derive(Debug, Serialize)]
struct SaveSummary {
    path: String,
    file_size: usize,
    chunk_count: usize,
    /// The three ints after `SAV3` (67 / 29 / 164 on 5.01).
    header_ints: Option<[i32; 3]>,
    /// Informational hint only (see `w3save::inspect`).
    looks_like_5x: bool,
    payload_error: Option<String>,
    fact_count: Option<usize>,
    name_count: Option<usize>,
    /// `worldInfo/world` (docs/findings/position-world.md section 3), as stored.
    world: Option<String>,
    journal_entries: Option<usize>,
    journal_counters: Option<usize>,
    /// Label of the journal entry the tracked-quest GUID points at.
    tracked_quest: Option<String>,
    journal_error: Option<String>,
    issues: Vec<Issue>,
}

fn summarize(path: &Path) -> CmdResult<SaveSummary> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let info = w3save::inspect(&bytes).map_err(err)?;
    let mut s = SaveSummary {
        path: path.display().to_string(),
        file_size: info.file_size,
        chunk_count: info.chunk_count,
        header_ints: info.payload.as_ref().map(|p| p.header_ints),
        looks_like_5x: info.looks_like_5x,
        payload_error: info.payload_error.clone(),
        fact_count: info.fact_count,
        name_count: info.name_count,
        world: None,
        journal_entries: None,
        journal_counters: None,
        tracked_quest: None,
        journal_error: None,
        issues: info.issues,
    };
    let doc = Container::read(&bytes).and_then(|c| Document::parse(&c.payload, c.data_start));
    let Ok(doc) = doc else { return Ok(s) };
    s.world = graph::locate(&doc, "worldInfo/world")
        .ok()
        .flatten()
        .map(|loc| match &graph::node_at(&doc, &loc).kind {
            w3save::Kind::Var(v) => match &v.value {
                w3save::Value::Str(s) => s.clone(),
                other => graph::value_text(&doc, other),
            },
            _ => graph::node_line(&doc, graph::node_at(&doc, &loc)),
        });
    match journal::read(&doc) {
        Ok(j) => {
            s.journal_entries = Some(j.entries.len());
            s.journal_counters = j.counters_size.map(|n| n as usize);
            s.tracked_quest = j.tracked_quest.map(|g| {
                j.entries
                    .iter()
                    .find(|e| e.path.last().is_some_and(|st| st.guid == g))
                    .map_or_else(|| format!("{} (no journal entry)", journal::guid_hex(&g)), |e| e.label())
            });
        }
        Err(e) => s.journal_error = Some(e.to_string()),
    }
    Ok(s)
}

/// Read a save the player picked and report what we can tell about it: container, header ints,
/// whether it looks like a 5.x save, damage signatures, world, journal size and tracked quest.
#[tauri::command]
async fn detect_game(save_path: String) -> CmdResult<SaveSummary> {
    blocking(move || summarize(Path::new(&save_path))).await?
}

/// Paths the UI offers as defaults.
#[derive(Debug, Serialize)]
struct DefaultPaths {
    /// `Documents\The Witcher 3\gamesaves`; the export screen's default destination.
    gamesaves: Option<String>,
    gamesaves_exists: bool,
    /// The game install synthesized effects read when the run sets no `game_dir` (the Steam default).
    game_dir: String,
}

#[tauri::command]
fn default_paths(app: AppHandle) -> DefaultPaths {
    let dir = app.path().document_dir().ok().map(|d| d.join("The Witcher 3").join("gamesaves"));
    DefaultPaths {
        gamesaves_exists: dir.as_ref().is_some_and(|d| d.is_dir()),
        gamesaves: dir.map(|d| d.display().to_string()),
        game_dir: export::DEFAULT_GAME_DIR.to_owned(),
    }
}

// ---------------------------------------------------------------------------------------------
// Questbook and run

#[tauri::command]
fn load_questbook(app: AppHandle) -> CmdResult<QuestbookView> {
    Ok(QuestbookView::new(&load_book(&app)?))
}

/// One illustration link from `questbook/images.toml` (DECISIONS.md "Illustrations are linked, not stored").
#[derive(Debug, serde::Deserialize, Serialize)]
struct ImageLink {
    /// A quest id, `act:<n>` or `region:<id>`.
    quest: String,
    image: String,
    page: String,
    title: String,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ImageFile {
    #[serde(default)]
    image: Vec<ImageLink>,
}

/// The illustration links; a missing or unreadable file just means no illustrations.
#[tauri::command]
fn load_images(app: AppHandle) -> Vec<ImageLink> {
    let Ok(dir) = questbook_dir(&app) else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(dir.join("images.toml")) else { return Vec::new() };
    toml::from_str::<ImageFile>(&text).map(|f| f.image).unwrap_or_default()
}

/// Opens an illustration's wiki page in the system browser. Only witcher.fandom.com pages are allowed.
#[tauri::command]
fn open_wiki_page(url: String) -> CmdResult<()> {
    if !url.starts_with("https://witcher.fandom.com/wiki/") || url.chars().any(|c| c.is_control() || c == ' ' || c == '"') {
        return Err("only Witcher Wiki pages can be opened".into());
    }
    let opener = if cfg!(target_os = "windows") {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener).arg(&url).spawn().map(|_| ()).map_err(err)
}

/// Official names from the player's own game install, for the UI. Nothing is cached on disk.
#[derive(Debug, Default, Serialize)]
struct GameNames {
    /// An install with the strings was found and read.
    found: bool,
    /// Quest id -> official title in the asked language.
    quests: std::collections::BTreeMap<String, String>,
    /// Item name as the save stores it -> official name (only the items that could be matched).
    items: std::collections::BTreeMap<String, String>,
    /// Side quest id (questbook/side) -> official title, by the quest's `title_id`.
    side: std::collections::BTreeMap<String, String>,
}

/// `lang` is a game language code (`en`, `ru`, ...). `game_dir` falls back to `W3AL_INSTALL`, then the
/// Steam default. A missing install, language or any read error returns an empty answer: the UI keeps
/// its own titles.
#[tauri::command]
async fn game_names(app: AppHandle, lang: String, game_dir: Option<String>) -> CmdResult<GameNames> {
    let book = load_book(&app)?;
    blocking(move || {
        let dir = game_dir.filter(|d| !d.trim().is_empty()).or_else(|| std::env::var("W3AL_INSTALL").ok()).unwrap_or_else(|| export::DEFAULT_GAME_DIR.to_owned());
        let install = Path::new(&dir);
        let lang: String = lang.chars().filter(char::is_ascii_alphanumeric).collect();
        let mut out = GameNames::default();
        if lang.is_empty() || !w3graph::strings::path_of(install, &lang).is_file() {
            return out;
        }
        let titles: Vec<String> = book.quests.iter().map(|q| q.title.clone()).collect();
        let Ok(by_title) = w3graph::names::names_in(install, &lang, &titles, &["CJournalQuest"]) else { return out };
        out.found = true;
        for q in &book.quests {
            if let Some(n) = by_title.get(&q.title) {
                out.quests.insert(q.id.clone(), n.clone());
            }
        }
        let mut items: Vec<String> = [1, 11, 18, 26, 34].iter().flat_map(|&l| w3al_core::start_kit::kit_for(l)).map(|k| k.name.to_owned()).collect();
        items.sort();
        items.dedup();
        if let Ok(m) = w3graph::names::item_names_in(install, &lang, &items) {
            out.items.extend(m);
        }
        let ids: Vec<u32> = book.side.quests.iter().filter_map(|q| q.title_id).collect();
        if let Ok(m) = w3graph::names::texts_by_id(install, &lang, &ids) {
            out.side.extend(book.side.quests.iter().filter_map(|q| q.title_id.and_then(|i| m.get(&i)).map(|t| (q.id.clone(), t.clone()))));
        }
        out
    })
    .await
}

/// One journal quest's texts for the UI (empty strings dropped).
#[derive(Debug, Serialize)]
struct QuestTexts {
    title: String,
    /// Description entries in tree order: `[index, text]`. The game shows the entry for the journal's stage,
    /// so entries are stages (0 = the opening), not consecutive paragraphs of one text.
    descriptions: Vec<(u8, String)>,
    objectives: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CharacterTexts {
    name: String,
    descriptions: Vec<(u8, String)>,
}

/// Journal texts of the player's install in one language, read at run time (never stored).
#[derive(Debug, Default, Serialize)]
struct GameTexts {
    found: bool,
    /// Journal quest GUID (32 lower-case hex, the first element of a save's journal path) -> texts.
    by_guid: std::collections::BTreeMap<String, QuestTexts>,
    /// Questbook quest id -> journal quest GUID, matched on the English official title (unambiguous ones only).
    guid_of: std::collections::BTreeMap<String, String>,
    /// Journal character GUID -> name and descriptions.
    characters: std::collections::BTreeMap<String, CharacterTexts>,
}

/// Quest descriptions, objective titles and character names in `lang` (`en`, `ru`, ...), same fallbacks as
/// `game_names`: no install or language means an empty answer.
#[tauri::command]
async fn game_texts(app: AppHandle, lang: String, game_dir: Option<String>) -> CmdResult<GameTexts> {
    let book = load_book(&app)?;
    blocking(move || {
        let dir = game_dir.filter(|d| !d.trim().is_empty()).or_else(|| std::env::var("W3AL_INSTALL").ok()).unwrap_or_else(|| export::DEFAULT_GAME_DIR.to_owned());
        let install = Path::new(&dir);
        let lang: String = lang.chars().filter(char::is_ascii_alphanumeric).collect();
        let mut out = GameTexts::default();
        if lang.is_empty() || !w3graph::strings::path_of(install, &lang).is_file() {
            return out;
        }
        let Ok(t) = w3graph::names::journal_texts(install, &lang) else { return out };
        let en = if lang == "en" { Some(t.clone()) } else { w3graph::names::journal_texts(install, "en").ok() };
        out.found = true;
        let pairs = |d: &[w3graph::names::DescText]| d.iter().filter(|d| !d.text.is_empty()).map(|d| (d.index, d.text.clone())).collect::<Vec<_>>();
        for q in &t.quests {
            let q_out = QuestTexts { title: q.title.clone(), descriptions: pairs(&q.descriptions), objectives: q.objectives.iter().filter(|o| !o.title.is_empty()).map(|o| o.title.clone()).collect() };
            if !q_out.title.is_empty() || !q_out.descriptions.is_empty() {
                out.by_guid.insert(q.guid.clone(), q_out);
            }
        }
        for c in &t.characters {
            if !c.name.is_empty() {
                out.characters.insert(c.guid.clone(), CharacterTexts { name: c.name.clone(), descriptions: pairs(&c.descriptions) });
            }
        }
        if let Some(en) = en {
            let titles: Vec<String> = book.quests.iter().map(|q| q.title.clone()).collect();
            let by_title = w3graph::names::quest_guids_by_title(&en, &titles);
            for q in &book.quests {
                if let Some(g) = by_title.get(&q.title).filter(|g| out.by_guid.contains_key(*g)) {
                    out.guid_of.insert(q.id.clone(), g.clone());
                }
            }
        }
        out
    })
    .await
}

/// Resolve the run into effects and report problems, without touching any file.
#[tauri::command]
fn validate_run(app: AppHandle, run: Run) -> CmdResult<Resolution> {
    Ok(resolve(&load_book(&app)?, &run))
}

#[tauri::command]
fn save_run(path: String, run: Run) -> CmdResult<()> {
    run.save(Path::new(&path)).map_err(err)
}

#[tauri::command]
fn load_run(path: String) -> CmdResult<Run> {
    Run::load(Path::new(&path)).map_err(err)
}

/// One donor save the run's graph effects name.
#[derive(Debug, Serialize)]
struct DonorStatus {
    stem: String,
    /// Effects (`quest/origin`) naming it.
    used_by: Vec<String>,
    /// False when every op of those effects only removes or recounts: the file is not read.
    needed: bool,
    found: Option<String>,
    problem: Option<String>,
}

/// Which donor saves the current run needs and whether the run's donor folders hold them.
#[tauri::command]
fn donor_status(app: AppHandle, run: Run) -> CmdResult<Vec<DonorStatus>> {
    Ok(donor_report(&load_book(&app)?, &run))
}

fn donor_report(book: &Questbook, run: &Run) -> Vec<DonorStatus> {
    let res = resolve(book, run);
    let dirs = ExportOptions::for_run(run).donor_dirs;
    let mut out: Vec<DonorStatus> = Vec::new();
    for a in &res.effects {
        let Some(g) = &a.effect.graph else { continue };
        let Some(stem) = &g.donor else { continue };
        let label = format!("{}/{}", a.quest, a.origin);
        if let Some(d) = out.iter_mut().find(|d| &d.stem == stem) {
            d.used_by.push(label);
            d.needed |= g.needs_donor();
            continue;
        }
        out.push(DonorStatus { stem: stem.clone(), used_by: vec![label], needed: g.needs_donor(), found: None, problem: None });
    }
    for d in &mut out {
        match export::find_donor(&dirs, &d.stem) {
            Ok(p) => d.found = Some(p.display().to_string()),
            Err(e) => d.problem = Some(e.to_string()),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Export

/// A refused or failed export, in plain words for the export screen.
#[derive(Debug, Serialize)]
struct ExportFailure {
    /// Short machine tag: `invalid`, `template`, `placeholder`, `donor`, `guard`, `overlap`,
    /// `names`, `effect`, `verify`, `unsupported`, `exists`, `io`, `setup`.
    kind: &'static str,
    /// One plain sentence.
    title: String,
    /// What the player can do about it.
    hint: String,
    /// The exact error text from the core, for bug reports.
    detail: String,
}

fn failure(kind: &'static str, title: &str, hint: &str, detail: impl Into<String>) -> ExportFailure {
    ExportFailure { kind, title: title.into(), hint: hint.into(), detail: detail.into() }
}

fn explain(e: &ExportError) -> ExportFailure {
    let d = e.to_string();
    match e {
        ExportError::Invalid(_) => failure(
            "invalid",
            "The run has errors, so nothing was built.",
            "Open Review: the errors are listed there. Usually a quest needs another quest that was skipped.",
            d,
        ),
        ExportError::Template(_) => failure(
            "template",
            "The template save is damaged (it carries the patch 5.0.0c chunk-table damage).",
            "Pick another save on the Welcome screen.",
            d,
        ),
        ExportError::Placeholder(_) => failure(
            "placeholder",
            "Some chosen effects are placeholders: they are not traced to the game yet and must never reach a save.",
            "Mark those quests as Skipped (they are named below), or wait for the content to be finished.",
            d,
        ),
        ExportError::BadStem(_) => failure("io", "The output file name is not allowed.", "Report this as a bug.", d),
        ExportError::Graphs(_) => failure(
            "game",
            "Some choices are built from the game's own quest data, and the game folder was not found.",
            "On the Export screen set \"Game folder\" to your Witcher 3 install (the folder holding `content` and `bin`). It is only read.",
            d,
        ),
        ExportError::Donor(_) => failure(
            "donor",
            "A donor save is missing (or found twice).",
            "On the Welcome screen add the folder that holds the named .sav file (exactly one folder may hold it).",
            d,
        ),
        ExportError::Effect(m) if m.contains("overlaps") => failure(
            "overlap",
            "Two of your choices edit the same part of the quest graph from different donors.",
            "These two outcomes cannot be combined yet; change one of the two quests named below.",
            d,
        ),
        ExportError::Effect(m) if m.contains("not the one the delta was captured from") => failure(
            "guard",
            "Guard mismatch: this template save is not the save the effect was captured on.",
            "Effects only fit the template (and donor) they were recorded with. Pick the matching template save, or a donor of the right version.",
            d,
        ),
        ExportError::Effect(m) if m.contains("name remapping") => failure(
            "names",
            "A donor's name table differs from the save's; copying needs name remapping.",
            "Turn on \"Allow name remap\" above and export again.",
            d,
        ),
        ExportError::Effect(_) => failure(
            "effect",
            "An effect does not fit this template save.",
            "The quest named below was captured on a different save. Pick another template or skip that quest.",
            d,
        ),
        ExportError::Verify(_) => failure(
            "verify",
            "The built save failed its own safety checks.",
            "Do not load any file this export may have left behind. Please report the details below.",
            d,
        ),
        ExportError::Save(w3save::Error::Unverified(_)) => failure(
            "unsupported",
            "The run uses an edit this version cannot write yet.",
            "Skip the quest named below; its effect type is not proven in game.",
            d,
        ),
        ExportError::Save(_) => failure("template", "The template save could not be read.", "Pick another save on the Welcome screen.", d),
        ExportError::Io(io) if io.kind() == std::io::ErrorKind::AlreadyExists => failure(
            "exists",
            "A file with the new save's name already exists; nothing was overwritten.",
            "Export again in a moment (the name carries the time), or choose another folder.",
            d,
        ),
        ExportError::Io(_) => failure("io", "A file could not be read or written.", "Check the folder exists and is writable.", d),
    }
}

/// What the export screen shows after a dry run or a real export.
#[derive(Debug, Serialize)]
struct ExportReport {
    /// False for a dry run (nothing written).
    written: bool,
    effects_applied: usize,
    /// One line per applied effect.
    notes: Vec<String>,
    verified: String,
    sav: Option<String>,
    sidecars: Vec<String>,
}

const VERIFIED: &str = "VERIFIED: re-parsed, table and post-order hold, check-save clean, copied subtrees equal their donors, \
                        journal and facts as requested, nothing else changed";

/// Resolve a run against a questbook and build its save: in memory only (`out = None`, nothing is
/// written) or written into `out` with `create_new`. Donor folders and the name-remap switch come
/// from the run (`donors`, `allow_name_remap`), and so does the game folder synth effects read (`game_dir`).
fn run_export(book: &Questbook, run: &Run, out: Option<&Path>) -> Result<ExportReport, ExportFailure> {
    let template = run.template.clone().map(PathBuf::from).ok_or_else(|| {
        failure("setup", "No template save is selected.", "Go back to the Welcome screen and pick a save.", "")
    })?;
    run_export_on(book, run, &template, out)
}

/// The story path (no manual template): choose the registered template, rewrite earlier decisions, export.
fn run_story_export(app: &AppHandle, run: &Run, out: Option<&Path>) -> Result<ExportReport, ExportFailure> {
    use w3al_core::chain::{self, Sources};
    let (book, reg, dirs) = story_sources(app, run).map_err(|e| failure("setup", "The questbook could not be loaded.", "Reinstall the app.", e))?;
    let src = Sources { book: &book, registry: &reg, template_dirs: dirs, opts: ExportOptions::for_run(run) };
    let choice = w3al_core::templates::choose(&book, &reg, run, &|t| w3al_core::templates::find_file(&src.template_dirs, &t.stem)).map_err(|e| {
        failure("setup", "No registered template save fits this story.", "Set the Templates folder on the Welcome screen, or pick a save by hand.", e)
    })?;
    match out {
        None => {
            let path = choice.path.clone().unwrap_or_default();
            let bytes = std::fs::read(&path).map_err(|e| failure("io", "The template save could not be read.", "Check the Templates folder.", format!("{}: {e}", path.display())))?;
            let (_, notes, rewrites) = chain::build_story(&src, run, &choice, &bytes, None).map_err(|e| explain(&e))?;
            Ok(ExportReport {
                written: false,
                effects_applied: rewrites.len() + choice.applied.len(),
                notes,
                verified: format!("{VERIFIED}; earlier decisions rewritten on {} (in memory; no file written)", choice.template.id),
                sav: None,
                sidecars: Vec::new(),
            })
        }
        Some(out) => {
            let s = chain::export_story(&src, run, out).map_err(|e| explain(&e))?;
            Ok(ExportReport {
                written: true,
                effects_applied: s.result.effects_applied,
                notes: s.result.notes,
                verified: format!("{VERIFIED}; earlier decisions rewritten on {}; file re-read", s.choice.template.id),
                sav: Some(s.result.sav.display().to_string()),
                sidecars: s.result.sidecars.iter().map(|p| p.display().to_string()).collect(),
            })
        }
    }
}

fn run_export_on(book: &Questbook, run: &Run, template: &Path, out: Option<&Path>) -> Result<ExportReport, ExportFailure> {
    let template = template.to_path_buf();
    let res = resolve(book, run);
    let opts = ExportOptions::for_run(run);
    match out {
        None => {
            let bytes = std::fs::read(&template).map_err(|e| {
                failure("io", "The template save could not be read.", "Pick the save again.", format!("{}: {e}", template.display()))
            })?;
            let built = export::build_verified(&bytes, &res, &opts).map_err(|e| explain(&e))?;
            Ok(ExportReport {
                written: false,
                effects_applied: res.effects.len(),
                notes: built.notes,
                verified: format!("{VERIFIED} (in memory; no file written)"),
                sav: None,
                sidecars: Vec::new(),
            })
        }
        Some(out) => {
            let r = export::export_with(&template, out, &res, &opts).map_err(|e| explain(&e))?;
            Ok(ExportReport {
                written: true,
                effects_applied: r.effects_applied,
                notes: r.notes,
                verified: format!("{VERIFIED}; file re-read"),
                sav: Some(r.sav.display().to_string()),
                sidecars: r.sidecars.iter().map(|p| p.display().to_string()).collect(),
            })
        }
    }
}

fn book_for_export(app: &AppHandle) -> Result<Questbook, ExportFailure> {
    load_book(app).map_err(|e| failure("setup", "The questbook could not be loaded.", "Reinstall the app.", e))
}

/// Build and verify the save in memory without writing anything.
#[tauri::command]
async fn check_export(app: AppHandle, run: Run) -> Result<ExportReport, ExportFailure> {
    let book = book_for_export(&app)?;
    blocking(move || if run.template.is_some() { run_export(&book, &run, None) } else { run_story_export(&app, &run, None) })
        .await
        .map_err(|e| failure("io", "Internal error.", "Report this as a bug.", e))?
}

/// Build the new save set. Never overwrites: files are opened with `create_new`.
#[tauri::command]
async fn export_save(app: AppHandle, run: Run, out_dir: String) -> Result<ExportReport, ExportFailure> {
    if out_dir.trim().is_empty() {
        return Err(failure("setup", "No destination folder is chosen.", "Choose a folder.", ""));
    }
    let book = book_for_export(&app)?;
    let out = PathBuf::from(out_dir);
    blocking(move || if run.template.is_some() { run_export(&book, &run, Some(&out)) } else { run_story_export(&app, &run, Some(&out)) })
        .await
        .map_err(|e| failure("io", "Internal error.", "Report this as a bug.", e))?
}

// ---------------------------------------------------------------------------------------------
// Edit mode: open one of the player's own saves, read where it stands, write a NEW changed save
// (w3al_core::edit). The opened save is only ever read.

/// What the engine was built for; a change of any of it loads a new engine.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EngineKey {
    game_dir: Option<String>,
    template_dirs: Vec<String>,
    donors: Vec<String>,
}

/// The folders edit mode reads besides the save (the game install, the template saves, the donor saves).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditFolders {
    #[serde(default)]
    game_dir: Option<String>,
    #[serde(default)]
    template_dirs: Vec<String>,
    #[serde(default)]
    donors: Vec<String>,
}

/// The loaded edit engine (quest graphs and window scans), kept between commands.
#[derive(Default)]
struct EditState(std::sync::Mutex<Option<(EngineKey, std::sync::Arc<w3al_core::edit::Engine>)>>);

fn edit_failure(e: &ExportError) -> ExportFailure {
    match e {
        ExportError::Effect(m) => failure(
            "refused",
            "This change cannot be made on this save.",
            "The reason is below. Decisions whose consequences already played out in the save cannot be rewritten.",
            m.clone(),
        ),
        ExportError::Invalid(m) => failure("invalid", "There is nothing to change.", "Pick at least one change first.", m.clone()),
        other => explain(other),
    }
}

/// Load (or reuse) the engine for the folders the player set. The first load of a questbook / template set
/// builds every window's answer combinations (minutes) and caches them on disk; progress goes out as
/// `edit-progress` events.
fn engine_for(app: &AppHandle, state: &EditState, game_dir: Option<String>, template_dirs: Vec<String>, donors: Vec<String>) -> Result<std::sync::Arc<w3al_core::edit::Engine>, ExportFailure> {
    use tauri::Emitter;
    let key = EngineKey { game_dir: game_dir.clone().filter(|d| !d.trim().is_empty()), template_dirs: template_dirs.clone(), donors: donors.clone() };
    if let Some((k, e)) = state.0.lock().map_err(|e| failure("io", "Internal error.", "Report this as a bug.", e.to_string()))?.as_ref() {
        if *k == key {
            return Ok(e.clone());
        }
    }
    let setup = |m: &str, d: String| failure("setup", m, "Check the Game folder and the Templates folder on the first screen of Edit mode.", d);
    let qb = questbook_dir(app).map_err(|e| setup("The questbook could not be loaded.", e))?;
    let book = Questbook::load_dir(&qb).map_err(|e| setup("The questbook could not be loaded.", e.to_string()))?;
    let reg = w3al_core::templates::Registry::load_for_questbook(&qb).map_err(|e| setup("The template registry could not be loaded.", e))?;
    let dirs: Vec<PathBuf> = if key.template_dirs.is_empty() {
        vec![qb.parent().unwrap_or(&qb).join(w3al_core::templates::DEFAULT_DIR)]
    } else {
        key.template_dirs.iter().map(PathBuf::from).collect()
    };
    let opts = ExportOptions {
        game_dir: key.game_dir.clone().map(PathBuf::from),
        allow_name_remap: true,
        donor_dirs: key.donors.iter().map(PathBuf::from).collect(),
        ..ExportOptions::default()
    };
    let cache = app.path().app_local_data_dir().ok().map(|d| d.join("edit-scan-cache.json"));
    let handle = app.clone();
    let engine = w3al_core::edit::Engine::load(book, reg, dirs, opts, cache.as_deref(), &move |m| {
        let _ = handle.emit("edit-progress", m);
    })
    .map_err(|e| failure("game", "The save could not be compared with the story.", "Edit mode reads the quest graphs of your Witcher 3 install: set the Game folder (the folder holding `content` and `bin`). It is only read.", e))?;
    let engine = std::sync::Arc::new(engine);
    *state.0.lock().map_err(|e| failure("io", "Internal error.", "Report this as a bug.", e.to_string()))? = Some((key, engine.clone()));
    Ok(engine)
}

/// The `.sav` files in a folder, newest first. Read-only.
#[tauri::command]
fn list_saves(dir: String) -> CmdResult<Vec<w3al_core::edit::SaveEntry>> {
    w3al_core::edit::list_saves(Path::new(&dir))
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(char::from(BASE64[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A save's thumbnail as a `data:` URL (the UI loads them one by one for the save list). Only `.png` files.
#[tauri::command]
async fn save_thumbnail(path: String) -> CmdResult<Option<String>> {
    blocking(move || {
        let p = Path::new(&path);
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")) {
            return None;
        }
        let bytes = std::fs::read(p).ok().filter(|b| b.len() < 4_000_000)?;
        Some(format!("data:image/png;base64,{}", base64(&bytes)))
    })
    .await
}

/// What the save list shows under a thumbnail: level, crowns and world, read from the save itself.
#[derive(Debug, Default, Serialize)]
struct SaveBrief {
    level: Option<i32>,
    crowns: Option<u32>,
    world: Option<String>,
    error: Option<String>,
}

#[tauri::command]
async fn save_brief(path: String) -> CmdResult<SaveBrief> {
    blocking(move || {
        let mut b = SaveBrief::default();
        match summarize(Path::new(&path)) {
            Ok(s) => b.world = s.world,
            Err(e) => {
                b.error = Some(e);
                return b;
            }
        }
        let doc = std::fs::read(&path).map_err(err).and_then(|bytes| Container::read(&bytes).map_err(err).and_then(|c| Document::parse(&c.payload, c.data_start).map_err(err)));
        match doc.and_then(|d| w3save::character::read(&d).map_err(err)) {
            Ok(c) => {
                b.level = Some(c.level.level);
                b.crowns = c.crowns();
            }
            Err(e) => b.error = Some(e),
        }
        b
    })
    .await
}

/// Where the opened save stands in the story and what can be changed (`w3al_core::edit::Plan`).
#[tauri::command]
async fn edit_plan(
    app: AppHandle,
    state: tauri::State<'_, EditState>,
    save_path: String,
    game_dir: Option<String>,
    template_dirs: Vec<String>,
    donors: Vec<String>,
) -> Result<w3al_core::edit::Plan, ExportFailure> {
    let engine = engine_for_blocking(&app, &state, game_dir, template_dirs, donors).await?;
    blocking(move || {
        let bytes = std::fs::read(&save_path).map_err(|e| failure("io", "The save could not be read.", "Pick it again.", format!("{save_path}: {e}")))?;
        let stem = Path::new(&save_path).file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
        engine.plan(&bytes, &stem, true).map_err(|e| failure("template", "This save could not be read.", "It may be damaged or from another game version.", e))
    })
    .await
    .map_err(|e| failure("io", "Internal error.", "Report this as a bug.", e))?
}

/// What the edit screen shows after the new save is written.
#[derive(Debug, Serialize)]
struct EditReport {
    sav: String,
    sidecars: Vec<String>,
    notes: Vec<String>,
    rewritten: Vec<String>,
    applied: Vec<String>,
    verified: String,
}

/// Write the changed save as a NEW file in `out_dir` (`create_new`; the opened save is never modified).
#[tauri::command]
async fn edit_apply(
    app: AppHandle,
    state: tauri::State<'_, EditState>,
    save_path: String,
    changes: w3al_core::edit::Changes,
    out_dir: String,
    folders: EditFolders,
) -> Result<EditReport, ExportFailure> {
    if out_dir.trim().is_empty() {
        return Err(failure("setup", "No destination folder is chosen.", "Choose a folder.", ""));
    }
    let engine = engine_for_blocking(&app, &state, folders.game_dir, folders.template_dirs, folders.donors).await?;
    blocking(move || {
        let o = engine.apply(Path::new(&save_path), Path::new(&out_dir), &changes, None).map_err(|e| edit_failure(&e))?;
        Ok(EditReport {
            sav: o.sav.display().to_string(),
            sidecars: o.sidecars.iter().map(|p| p.display().to_string()).collect(),
            notes: o.notes,
            rewritten: o.rewritten,
            applied: o.applied,
            verified: o.verified,
        })
    })
    .await
    .map_err(|e| failure("io", "Internal error.", "Report this as a bug.", e))?
}

/// [`engine_for`] off the UI thread (it can take minutes the first time).
async fn engine_for_blocking(
    app: &AppHandle,
    state: &tauri::State<'_, EditState>,
    game_dir: Option<String>,
    template_dirs: Vec<String>,
    donors: Vec<String>,
) -> Result<std::sync::Arc<w3al_core::edit::Engine>, ExportFailure> {
    let cached = {
        let key = EngineKey { game_dir: game_dir.clone().filter(|d| !d.trim().is_empty()), template_dirs: template_dirs.clone(), donors: donors.clone() };
        state.0.lock().ok().and_then(|g| g.as_ref().filter(|(k, _)| *k == key).map(|(_, e)| e.clone()))
    };
    if let Some(e) = cached {
        return Ok(e);
    }
    let app2 = app.clone();
    let built = blocking(move || {
        let st = app2.state::<EditState>();
        engine_for(&app2, &st, game_dir, template_dirs, donors)
    })
    .await
    .map_err(|e| failure("io", "Internal error.", "Report this as a bug.", e))??;
    Ok(built)
}

// ---------------------------------------------------------------------------------------------
// Illustration cache (`wikiimg` scheme, see `wikicache`)

fn wiki_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("wiki-images"))
}

fn image_response(status: u16, mime: &str, body: Vec<u8>) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Cache-Control", "max-age=31536000")
        .header("Access-Control-Allow-Origin", "*")
        .body(body)
        .unwrap_or_default()
}

/// Background download of every linked image: the questbook's quests first, then `journal:*`, `region:*`, `act:*`.
fn start_prefetch(app: &AppHandle) {
    use tauri::Emitter;
    let (Some(dir), Ok(qb)) = (wiki_cache_dir(app), questbook_dir(app)) else { return };
    let handle = app.clone();
    std::thread::spawn(move || {
        let Ok(text) = std::fs::read_to_string(qb.join("images.toml")) else { return };
        let Ok(file) = toml::from_str::<ImageFile>(&text) else { return };
        let rank = |q: &str| if q.starts_with("journal:") { 1 } else if q.starts_with("region:") { 2 } else if q.starts_with("act:") { 3 } else { 0 };
        let mut links = file.image;
        links.sort_by_key(|l| rank(&l.quest)); // stable: file order within a class
        let urls = links.into_iter().map(|l| l.image).collect();
        wikicache::prefetch(&dir, urls, 16, move |done, total| {
            let _ = handle.emit("images-progress", serde_json::json!({ "done": done, "total": total }));
        });
    });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol("wikiimg", |ctx, request, responder| {
            let dir = wiki_cache_dir(ctx.app_handle());
            let path = request.uri().path().to_owned();
            std::thread::spawn(move || {
                let reply = (|| {
                    let url = wikicache::url_from_request_path(&path).map_err(|e| (400, e))?;
                    let dir = dir.ok_or((500, "no cache folder".to_owned()))?;
                    wikicache::get(&dir, &wikicache::agent(), &url).map_err(|e| (502, e))
                })();
                responder.respond(match reply {
                    Ok((bytes, mime)) => image_response(200, mime, bytes),
                    Err((status, msg)) => image_response(status, "text/plain", msg.into_bytes()),
                });
            });
        })
        .setup(|app| {
            start_prefetch(app.handle());
            Ok(())
        })
        .manage(EditState::default())
        .invoke_handler(tauri::generate_handler![
            list_saves,
            save_thumbnail,
            save_brief,
            edit_plan,
            edit_apply,
            detect_game,
            default_paths,
            load_questbook,
            load_images,
            open_wiki_page,
            game_names,
            game_texts,
            validate_run,
            save_run,
            load_run,
            donor_status,
            template_plan,
            side_quest_states,
            check_export,
            export_save
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Another Life app");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `W3AL_FIXTURES` (a folder of save copies): every save summarizes without panicking.
    #[test]
    fn summarize_fixtures() {
        let Ok(dir) = std::env::var(paths::FIXTURES_ENV) else { return };
        let mut seen = 0;
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "sav") {
                let s = summarize(&p).unwrap();
                println!("{}: world {:?}, journal {:?}, tracked {:?}", p.display(), s.world, s.journal_entries, s.tracked_quest);
                seen += 1;
            }
        }
        println!("{seen} saves");
    }

    #[test]
    fn explains_every_refusal() {
        let f = explain(&ExportError::Effect(
            "mq/done: template node questSystem/x is not the one the delta was captured from (hash None, want #00000000)".into(),
        ));
        assert_eq!(f.kind, "guard");
        let f = explain(&ExportError::Effect("b: its graph op on a overlaps a of c (different donors)".into()));
        assert_eq!(f.kind, "overlap");
        let f = explain(&ExportError::Io(std::io::Error::from(std::io::ErrorKind::AlreadyExists)));
        assert_eq!(f.kind, "exists");
        assert_eq!(explain(&ExportError::Donor("x".into())).kind, "donor");
    }

    fn find_sav(dir: &Path, stem: &str) -> Option<PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = find_sav(&p, stem) {
                    return Some(f);
                }
            } else if p.file_name().is_some_and(|n| n.to_string_lossy() == format!("{stem}.sav")) {
                return Some(p);
            }
        }
        None
    }

    /// The app's export path on the gate R1 test questbook (`W3AL_FIXTURES` = lukwraith saves,
    /// `W3AL_DONORS` = folder with donor S). Writes only into a fresh temp folder.
    #[test]
    fn export_path_on_gate_r1() {
        let (Ok(fx), Ok(donors)) = (std::env::var(paths::FIXTURES_ENV), std::env::var("W3AL_DONORS")) else { return };
        let Some(t21) = find_sav(Path::new(&fx), "ManualSave_fd3e1_7ea4d800_3f4cf08") else { return };
        let book_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/w3al-core/tests/data/gate_r1");
        let book = Questbook::load_dir(&book_dir).unwrap();
        let mut run = Run { template: Some(t21.display().to_string()), donors: vec![donors.clone()], ..Run::default() };
        for q in &book.quests {
            run.quests.insert(q.id.clone(), w3al_core::run::QuestEntry { status: w3al_core::Status::Completed, answers: Default::default() });
        }

        let ds = donor_report(&book, &run);
        println!("{ds:#?}");
        assert!(ds.iter().any(|d| d.needed && d.found.is_some()));
        assert!(ds.iter().all(|d| !d.needed || d.found.is_some()));

        let check = run_export(&book, &run, None).unwrap();
        assert!(!check.written && check.effects_applied == 2 && check.notes.len() == 2, "{check:#?}");

        let out = std::env::temp_dir().join(format!("w3al-app-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let a = run_export(&book, &run, Some(&out)).unwrap();
        println!("{a:#?}");
        assert!(Path::new(a.sav.as_deref().unwrap()).is_file());
        let b = run_export(&book, &run, Some(&out)).unwrap();
        assert_ne!(a.sav, b.sav, "a second export gets a new name");

        let mut no_donors = run.clone();
        no_donors.donors.clear();
        let f = run_export(&book, &no_donors, None).unwrap_err();
        println!("{f:#?}");
        assert_eq!(f.kind, "donor");

        let mut no_template = run.clone();
        no_template.template = None;
        assert_eq!(run_export(&book, &no_template, None).unwrap_err().kind, "setup");

        if let Some(d22) = find_sav(Path::new(&fx), "ManualSave_8559a_7ea4d800_4038706") {
            let mut other = run.clone();
            other.template = Some(d22.display().to_string());
            let f = run_export(&book, &other, None).unwrap_err();
            println!("{f:#?}");
            assert!(["guard", "effect"].contains(&f.kind));
        }
        std::fs::remove_dir_all(&out).unwrap();
    }
}
