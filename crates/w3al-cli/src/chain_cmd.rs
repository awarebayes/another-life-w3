//! `w3al chain-plan`, `w3al template-scan` and the story path of `w3al export` (no `--template`):
//! the template registry and the rewriting of earlier decisions (w3al_core::chain, docs/plan/story-chain.md).
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use w3al_core::chain::{self, Sources};
use w3al_core::export::ExportOptions;
use w3al_core::templates::{self, Baked, Choice, Registry};
use w3al_core::{paths, Questbook, Run};

pub fn questbook_dir(dir: Option<PathBuf>) -> Result<PathBuf> {
    paths::locate_questbook(dir).context("no questbook directory found (use --questbook or W3AL_QUESTBOOK)")
}

/// The template folders: `--template-dir` options, else the run's `template_dirs` (relative to the run
/// file), else `templates` next to the questbook.
pub fn template_dirs(cli: &[PathBuf], run: &Run, run_dir: &Path, qb: &Path) -> Vec<PathBuf> {
    if cli.is_empty() {
        templates::search_dirs(run, run_dir, qb)
    } else {
        cli.to_vec()
    }
}

fn answers_text(m: &std::collections::BTreeMap<String, String>) -> String {
    if m.is_empty() {
        "done".into()
    } else {
        m.iter().map(|(c, o)| format!("{c}={o}")).collect::<Vec<_>>().join(", ")
    }
}

pub fn print_choice(c: &Choice) {
    println!("template: {} (line {}) {}", c.template.id, c.template.line, c.path.as_ref().map_or("NOT FOUND".to_string(), |p| p.display().to_string()));
    println!("  window quests applied: {}", if c.applied.is_empty() { "-".into() } else { c.applied.join(", ") });
    for b in &c.baked {
        match &b.action {
            Baked::Same => println!("  = {}: as in the template", b.quest),
            Baked::Unchecked => println!("  ? {}: not recorded for this template (kept as the template has it)", b.quest),
            Baked::Rewrite { from, to } => println!("  ~ {}: REWRITE {} -> {}", b.quest, answers_text(from), answers_text(to)),
            Baked::Fixed { from, to, why } => println!("  ! {}: FIXED {} (wanted {}): {why}", b.quest, answers_text(from), answers_text(to)),
        }
    }
    for q in &c.beyond {
        println!("  > {q}: after this template on the line; not in the save (play it in game)");
    }
    for p in &c.passed_over {
        println!("  passed over: {p}");
    }
    for n in &c.notes {
        println!("  note: {n}");
    }
}

pub fn chain_plan(run_path: &Path, qb: Option<PathBuf>, dirs: &[PathBuf]) -> Result<ExitCode> {
    let run = Run::load(run_path)?;
    let qb = questbook_dir(qb)?;
    let book = Questbook::load_dir(&qb)?;
    let reg = Registry::load_for_questbook(&qb).map_err(anyhow::Error::msg)?;
    let dirs = template_dirs(dirs, &run, run_path.parent().unwrap_or(Path::new(".")), &qb);
    println!("template folders: {}", dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", "));
    match templates::choose(&book, &reg, &run, &|t| templates::find_file(&dirs, &t.stem)) {
        Ok(c) => {
            print_choice(&c);
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => bail!("{e}"),
    }
}

/// `w3al export` without a template: the story chain.
pub fn story_export(run_path: &Path, run: &Run, qb: Option<PathBuf>, dirs: &[PathBuf], out_dir: &Path, opts: &ExportOptions) -> Result<ExitCode> {
    let qb = questbook_dir(qb)?;
    let book = Questbook::load_dir(&qb)?;
    let reg = Registry::load_for_questbook(&qb).map_err(anyhow::Error::msg)?;
    let dirs = template_dirs(dirs, run, run_path.parent().unwrap_or(Path::new(".")), &qb);
    let src = Sources { book: &book, registry: &reg, template_dirs: dirs, opts: opts.clone() };
    let choice = templates::choose(&book, &reg, run, &|t| templates::find_file(&src.template_dirs, &t.stem)).map_err(anyhow::Error::msg)?;
    print_choice(&choice);
    match chain::export_story(&src, run, out_dir) {
        Ok(s) => {
            for n in &s.result.notes {
                println!("  applied {n}");
            }
            println!("wrote {}", s.result.sav.display());
            for x in &s.result.sidecars {
                println!("      {}", x.display());
            }
            println!("VERIFIED: earlier decisions rewritten and checked (facts, quest log, untouched blocks), window effects applied and verified as `export` does; file re-read");
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => bail!("{e}"),
    }
}

/// `w3al template-scan`: for each registered template (or `--id`), what it holds for every earlier
/// window: the best-matching answer combination (built on the earlier window's own template) and the
/// readers of the facts that tell the answers apart, with their state in this save.
pub fn template_scan(qb: Option<PathBuf>, dirs: &[PathBuf], only: Option<String>, max: usize, opts: &ExportOptions) -> Result<ExitCode> {
    let qb = questbook_dir(qb)?;
    let book = Questbook::load_dir(&qb)?;
    let reg = Registry::load_for_questbook(&qb).map_err(anyhow::Error::msg)?;
    let dirs = if dirs.is_empty() { templates::search_dirs(&Run::default(), Path::new("."), &qb) } else { dirs.to_vec() };
    let lib = chain::load_library(opts)?;
    let src = Sources { book: &book, registry: &reg, template_dirs: dirs, opts: opts.clone() };
    let mut windows: Vec<&templates::Template> = reg.templates.iter().collect();
    windows.sort_by_key(|t| t.line);
    let mut scans = Vec::new();
    for w in &windows {
        eprintln!("building the answer combinations of {} ...", w.id);
        match chain::scan_window(&src, Some(&lib), w, max) {
            Ok(c) => scans.push((*w, c)),
            Err(e) => println!("# {}: not scanned ({e})", w.id),
        }
    }
    for t in windows.iter().filter(|t| only.as_deref().is_none_or(|o| o == t.id)) {
        let Some(path) = templates::find_file(&src.template_dirs, &t.stem) else {
            println!("# {}: {}.sav not found", t.id, t.stem);
            continue;
        };
        let doc = {
            let bytes = std::fs::read(&path)?;
            let c = w3save::Container::read(&bytes)?;
            w3save::Document::parse(&c.payload, c.data_start)?
        };
        println!("\n# ---- {} (line {}) {}", t.id, t.line, path.display());
        println!("[template.encodes]   # w3al template-scan");
        for (w, cands) in scans.iter().filter(|(w, _)| w.line < t.line) {
            let failed = cands.iter().filter(|c| c.facts.is_none()).count();
            match chain::match_window(cands, &doc) {
                Ok(m) => {
                    for (q, a) in &m.answers {
                        let body = a.iter().map(|(c, o)| format!("{c} = \"{o}\"")).collect::<Vec<_>>().join(", ");
                        println!("{q} = {{ {body} }}");
                    }
                    println!(
                        "#   window {}: {} combination(s) ({} do not export), {} varying fact(s); best mismatches {} [{}], next best {:?}",
                        w.id,
                        cands.len(),
                        failed,
                        m.varying.len(),
                        m.mismatches.len(),
                        m.mismatches.join("; "),
                        m.next_best
                    );
                    let rd = chain::readers(&lib, &m.varying, &doc).map_err(anyhow::Error::msg)?;
                    let mut by_state: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
                    for r in &rd {
                        by_state.entry(r.state.as_str()).or_default().push(format!("{} {} ({})", r.fact, r.block, r.file.rsplit('/').next().unwrap_or(&r.file)));
                    }
                    for (s, v) in by_state {
                        println!("#   readers {s}: {}", v.join(", "));
                    }
                }
                Err(e) => println!("#   window {}: {e}", w.id),
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
