//! `w3al synth-walk` / `w3al synth-apply`: walk a decision through the game's quest graphs (read
//! from the player's own install or a folder of extracted graphs) from a template save, print what
//! the game would write (facts, journal, spawn phases, new waits), compare that with a save a human
//! really played from the template (`--donor`), and write it into a copy of the template without
//! any donor. Design: docs/plan/no-human-pipeline.md.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Result};
use w3al_core::export;
use w3graph::community;
use w3graph::frontier::{self, Frontier};
use w3graph::quest::{guid_hex, short, BlockKind, Library};
use w3graph::synth;
use w3graph::walk::{Event, Walk};
use w3save::graph;
use w3save::journal;
use w3save::{Container, Document};

use crate::graph_cmd::load_doc;

pub struct WalkArgs<'a> {
    pub graphs: &'a [PathBuf],
    pub install: Option<&'a Path>,
    pub template: &'a Path,
    pub starts: &'a [String],
    pub picks: &'a [String],
    pub scene_picks: &'a [String],
    pub stops: &'a [String],
    pub through: bool,
    pub no_listeners: bool,
    pub donor: Option<&'a Path>,
    pub show_frontier: Option<&'a str>,
    pub verbose: bool,
    pub max_steps: usize,
}

pub fn load_library(graphs: &[PathBuf], install: Option<&Path>) -> Result<Library> {
    if let Some(i) = install {
        return Library::load_install(i).map_err(|e| anyhow!("{e}"));
    }
    if graphs.is_empty() {
        bail!("give --graphs DIR (extracted .w2phase/.w2quest files) or --install DIR (the game folder)");
    }
    Library::load_dirs(graphs).map_err(|e| anyhow!("{e}"))
}

struct Built {
    lib: Library,
    template: Document,
    tf: Frontier,
    walk: Walk,
}

fn build(a: &WalkArgs) -> Result<Option<Built>> {
    let lib = load_library(a.graphs, a.install)?;
    eprintln!("graphs: {} files, {} blocks", lib.files.len(), lib.blocks.len());
    let template = load_doc(a.template)?;
    let tf = frontier::read(&template).map_err(|e| anyhow!("{e}"))?;
    eprintln!("template frontier: {} waiting blocks in {} scopes, {} registered scenes", tf.blocks.len(), tf.scopes.len(), tf.scenes.len());

    if let Some(g) = a.show_frontier {
        for l in frontier::describe_frontier(&lib, &tf, if g.is_empty() { None } else { Some(g) }) {
            println!("{l}");
        }
        if a.starts.is_empty() {
            return Ok(None);
        }
    }

    let spec = synth::WalkSpec {
        starts: a.starts.to_vec(),
        picks: a.picks.to_vec(),
        scene_picks: a.scene_picks.to_vec(),
        stops: a.stops.to_vec(),
        through: a.through,
        listeners: !a.no_listeners,
        max_steps: a.max_steps,
        require_waiting_starts: false,
    };
    if spec.starts.is_empty() {
        bail!("give at least one --start GUID[:input] (a block waiting in the template, see --show-frontier)");
    }
    let (walk, notes) = synth::plan(&lib, &template, &tf, &spec).map_err(|e| anyhow!("{e}"))?;
    for n in notes {
        eprintln!("note: {n}");
    }
    Ok(Some(Built { lib, template, tf, walk }))
}

fn print_walk(lib: &Library, walk: &Walk, verbose: bool) -> usize {
    println!("# walk: {} steps, {} events", walk.steps, walk.events.len());
    let mut unresolved = 0;
    for e in &walk.events {
        let line = match e {
            Event::Fired { block, input } => {
                if !verbose {
                    continue;
                }
                format!("  fired    {} <- {input:?}", lib.describe(*block))
            }
            Event::Assumed { block, socket, why } => format!("  assumed  {} -> {socket:?} ({why})", lib.describe(*block)),
            Event::Fact { id, value, .. } => format!("  FACT     {id} += {value}"),
            Event::FactRemoved { id, .. } => format!("  FACT     {id} removed"),
            Event::Journal { quest, path, status, block } => {
                let p = path.iter().map(|s| short(&s.guid)).collect::<Vec<_>>().join("/");
                format!("  JOURNAL  {status} {}{p}  ({})", if *quest { "quest " } else { "entry " }, lib.describe(*block))
            }
            Event::Reward { name, .. } => format!("  reward   {name}"),
            Event::Spawn { action, .. } => format!("  SPAWN    {action:?}"),
            Event::Script { function, .. } => {
                if !verbose {
                    continue;
                }
                format!("  script   {function}")
            }
            Event::Enter { phase, socket } => format!("  ENTER    {} via {socket:?}", lib.describe(*phase)),
            Event::Exit { phase, socket } => format!("  EXIT     {} -> {socket:?}", lib.describe(*phase)),
            Event::Wait { block, input } => format!("  WAIT     {} <- {input:?}", lib.describe(*block)),
            Event::Unresolved { block, why } => {
                unresolved += 1;
                format!("  ??       {}: {why}", lib.describe(*block))
            }
            Event::Skipped { block, input } => {
                if !verbose {
                    continue;
                }
                format!("  skipped  {} {input}", lib.describe(*block))
            }
            Event::Maybe { block, id, via } => format!("  MAYBE    {id}  (set in {via}, behind an unpicked choice of {}; not set)", lib.describe(*block)),
            Event::Note { block, text } => format!("  note     {}: {text}", lib.describe(*block)),
        };
        println!("{line}");
    }
    println!("# facts set: {}", walk.facts.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "));
    println!("# scopes exited: {}", walk.exited.iter().map(|i| lib.describe(*i)).collect::<Vec<_>>().join("; "));
    println!("# scopes entered: {}", walk.entered.iter().map(|i| lib.describe(*i)).collect::<Vec<_>>().join("; "));
    println!("# new frontier ({} waits):", walk.waits.len());
    for w in &walk.waits {
        let scope = w.scope.iter().map(short).collect::<Vec<_>>().join(" > ");
        println!("  {} input={:?} conds={}  in {scope}", lib.describe(w.block), w.input, w.n_conds);
    }
    if unresolved > 0 {
        println!("# {unresolved} unresolved point(s): add --pick GUID=socket or --stop GUID");
    }
    unresolved
}

pub fn synth_walk_cmd(a: &WalkArgs) -> Result<ExitCode> {
    let Some(b) = build(a)? else { return Ok(ExitCode::SUCCESS) };
    print_walk(&b.lib, &b.walk, a.verbose);
    let pred = frontier::predict(&b.lib, &b.walk, &b.tf);
    if let Some(d) = a.donor {
        let donor = load_doc(d)?;
        let r = frontier::compare(&b.lib, &pred, &b.template, &donor).map_err(|e| anyhow!("{e}"))?;
        println!("# compare with {}", d.display());
        for l in &r.lines {
            println!("{l}");
        }
    } else {
        println!("# prediction: {} blocks removed, {} added, {} facts, {} journal items", pred.removed.len(), pred.added.len(), pred.facts.len(), pred.journal.len());
    }
    Ok(ExitCode::SUCCESS)
}

/// Write the walk into a copy of the template (no donor), verify, and with `--donor` show how
/// the result differs from the played save in the touched scopes, the journal and the facts.
pub fn synth_apply_cmd(a: &WalkArgs, out_dir: &Path, stem: Option<&str>) -> Result<ExitCode> {
    let Some(b) = build(a)? else { return Ok(ExitCode::SUCCESS) };
    let unresolved = print_walk(&b.lib, &b.walk, a.verbose);
    if unresolved > 0 {
        bail!("{unresolved} unresolved point(s) in the walk; nothing written");
    }
    let (doc, rep) = synth::apply(&b.lib, &b.walk, &b.template).map_err(|e| anyhow!("{e}"))?;
    println!("# synthesized");
    for (what, v) in [
        ("removed block", &rep.removed_blocks),
        ("removed scope", &rep.removed_scopes),
        ("added block", &rep.added_blocks),
        ("added scope", &rep.added_scopes),
        ("input", &rep.input_changed),
        ("scene removed", &rep.scenes_removed),
        ("scene added", &rep.scenes_added),
    ] {
        for p in v {
            let tail: Vec<&str> = p.rsplit('/').take(3).collect();
            println!("  {what}  {}", tail.into_iter().rev().collect::<Vec<_>>().join("/"));
        }
    }
    println!(
        "  journal: {} status set, {} appended, {} counters added; facts set {:?}, removed {:?}; NM names appended {}",
        rep.journal.status_set, rep.journal.appended, rep.journal.counters_appended, rep.facts_set, rep.facts_removed, rep.names_added
    );
    let c = &rep.community;
    for (set, from, to) in &c.phases_set {
        println!("  community: {set} phase {from} -> {to}");
    }
    for s in &c.sets_added {
        println!("  community: set added {s}");
    }
    for s in &c.sets_removed {
        println!("  community: set removed {s}");
    }
    for (tag, set) in &c.stubs_removed {
        println!("  community: stub removed {tag} ({set})");
    }
    if !c.entities_removed.is_empty() {
        println!("  community: {} entityData entries removed with their stubs", c.entities_removed.len());
    }
    for n in rep.notes.iter().chain(&c.notes) {
        println!("  note: {n}");
    }

    let template_bytes = std::fs::read(a.template)?;
    let mut container = Container::read(&template_bytes)?;
    container.payload = doc.to_bytes();
    let bytes = container.write()?;
    let stem = stem.map_or_else(|| export::default_stem(a.template, out_dir), str::to_owned);
    let written = export::write_save_set_as(a.template, out_dir, &stem, &bytes)?;
    println!("wrote {}", written.sav.display());

    // Re-read from disk and check.
    let disk = std::fs::read(&written.sav)?;
    let c = Container::read(&disk)?;
    let re = Document::parse(&c.payload, c.data_start)?;
    if re.to_bytes() != c.payload {
        bail!("the written payload does not round-trip");
    }
    if let Some(d) = a.donor {
        let donor = load_doc(d)?;
        compare_saves(&re, &donor, &rep, &community::touched_sets(&synth::spawn_actions(&b.walk)))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// questSystem differences in the scopes the synthesis touched, journal, fact and spawn-set differences.
fn compare_saves(synth: &Document, donor: &Document, rep: &synth::SynthReport, sets: &[String]) -> Result<()> {
    let touched: Vec<String> = rep
        .removed_blocks
        .iter()
        .chain(&rep.removed_scopes)
        .chain(&rep.added_blocks)
        .chain(&rep.added_scopes)
        .filter_map(|p| p.rsplit_once('/').map(|x| x.0.to_owned()))
        .map(|p| p.trim_end_matches("/questThread").to_owned())
        .collect();
    let changes = graph::diff_top(synth, donor, "questSystem")?.unwrap_or_default();
    let (mut inside, mut outside) = (Vec::new(), 0);
    for c in &changes {
        let p = c.path();
        if touched.iter().any(|t| p.starts_with(t.as_str())) || p.starts_with("questSystem/questExternalScenePlayers") {
            inside.push(c);
        } else {
            outside += 1;
        }
    }
    println!("# vs donor: questSystem changes in touched scopes: {} ({} elsewhere)", inside.len(), outside);
    for c in inside {
        let text = match c {
            graph::Change::Removed { path, .. } => format!("  synth has, donor lacks: {path}"),
            graph::Change::Added { path, .. } => format!("  donor has, synth lacks: {path}"),
            graph::Change::Changed { path, a, b } => format!("  differ: {path}\n      synth {a}\n      donor {b}"),
            graph::Change::Reordered { path } => format!("  reordered: {path}"),
        };
        println!("{}", text.replace("questSystem/quest[witcher3_quest.w2quest]/questThread/", "Q/"));
    }
    let sj = journal::read(synth)?;
    let dj = journal::read(donor)?;
    let d = journal::diff(&sj, &dj);
    println!(
        "# vs donor: journal entries changed={} added={} removed={}; counters changed={} added={}",
        d.changed.len(),
        d.added.len(),
        d.removed.len(),
        d.counters_changed.len(),
        d.counters_added.len()
    );
    for c in &d.changed {
        println!("  differ: {} synth {} donor {}", c.after.label(), c.before.status, c.after.status);
    }
    for (_, e) in &d.added {
        println!("  donor has, synth lacks: {} {}", e.status, e.label());
    }
    for (_, e) in &d.removed {
        println!("  synth has, donor lacks: {} {}", e.status, e.label());
    }
    let sf = frontier::fact_sums(synth).map_err(|e| anyhow!("{e}"))?;
    let df = frontier::fact_sums(donor).map_err(|e| anyhow!("{e}"))?;
    let only_s: Vec<&String> = sf.keys().filter(|k| !df.contains_key(*k) && !frontier::is_noise_fact(k)).collect();
    let only_d: Vec<&String> = df.keys().filter(|k| !sf.contains_key(*k) && !frontier::is_noise_fact(k)).collect();
    let differ: Vec<&String> = sf.keys().filter(|k| df.get(*k).is_some_and(|v| v != &sf[*k]) && !frontier::is_noise_fact(k)).collect();
    println!("# vs donor: facts only in synth {only_s:?}; only in donor {only_d:?}; values differ {differ:?}");
    for (j, c) in &d.counters_added {
        println!("  donor has counter, synth lacks: {} count={} (donor position {j})", guid_hex(&c.guid), c.count);
    }
    for c in &d.counters_removed {
        println!("  synth has counter, donor lacks: {} count={}", guid_hex(&c.guid), c.count);
    }
    let comm = community::compare(synth, donor, sets);
    println!("# vs donor: community differences in the {} spawn set(s) the walk touched: {}", sets.len(), comm.len());
    for l in &comm {
        println!("  {l}");
    }
    Ok(())
}

/// `w3al scene-info`: a scene's inputs, outputs, choices (with the section names a `--scene-pick`
/// may name) and the facts it sets; with `--from/--to`, the outcome between an input and an output.
pub fn scene_info_cmd(graphs: &[PathBuf], install: Option<&Path>, scene: &str, from: Option<&str>, to: Option<&str>, picks: &[String], verbose: bool) -> Result<ExitCode> {
    let lib = load_library(graphs, install)?;
    let s = lib.scene(scene).map_err(|e| anyhow!("{e}"))?;
    println!("{}: {} nodes", s.file, s.nodes.len());
    for i in &s.inputs {
        println!("  input  #{i} {:?}", s.nodes[*i].name);
    }
    for o in &s.outputs {
        println!("  output #{o} {:?}", s.nodes[*o].name);
    }
    for (i, n) in s.nodes.iter().enumerate() {
        let lines: Vec<String> = n
            .links
            .iter()
            .filter_map(|l| match l {
                w3graph::scene::Link::Choice { line, .. } => Some(format!("#{line} -> {:?}", s.nodes[*line].name)),
                _ => None,
            })
            .collect();
        if !lines.is_empty() {
            println!("  choice at #{i} {:?}: {}", n.name, lines.join(" | "));
        }
        for f in &n.facts {
            println!("  fact   #{i} {} {:?}: {f:?}", n.class.trim_start_matches("CStoryScene"), n.name);
        }
        if verbose && !n.events.is_empty() {
            println!("  events #{i} {:?}: {}", n.name, n.events.join(" "));
        }
        if let Some(c) = &n.cond {
            println!("  cond   #{i}: {c:?}");
        }
    }
    if let (Some(from), Some(to)) = (from, to) {
        let o = s.outcome(from, to, picks, &|_| None).map_err(|e| anyhow!("{e}"))?;
        println!("# outcome {from:?} -> {to:?} (conditions unknown, picks {picks:?})");
        for f in &o.certain {
            println!("  certain {f:?}");
        }
        for (f, via) in &o.maybe {
            println!("  maybe   {f:?} (via {via})");
        }
        for c in &o.choices {
            println!("  choice  {c:?}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `w3al graph-lookup`: describe GUIDs (from `graph-diff` output) with the game's graphs.
pub fn graph_lookup_cmd(graphs: &[PathBuf], install: Option<&Path>, keys: &[String]) -> Result<ExitCode> {
    let lib = load_library(graphs, install)?;
    for k in keys {
        let hits = if k.len() == 32 { lib.get(&w3graph::quest::parse_guid(k).map_err(|e| anyhow!("{e}"))?).into_iter().collect::<Vec<_>>() } else { lib.by_short(k).to_vec() };
        if hits.is_empty() {
            println!("{k}: not in the quest graphs");
        }
        for i in hits {
            let b = &lib.blocks[i];
            let extra = match &b.kind {
                BlockKind::Pause { conds } => format!(" waits on {conds:?}"),
                BlockKind::Fact { id, value } => format!(" sets {id} += {value}"),
                BlockKind::Journal { path, .. } => format!(" journal {}", path.iter().map(|s| short(&s.guid)).collect::<Vec<_>>().join("/")),
                BlockKind::Scene { file } | BlockKind::Interaction { file, .. } => format!(" scene {file}"),
                BlockKind::Phase { import: Some(f), .. } => format!(" imports {f}"),
                BlockKind::Script { function, params } => format!(" script {function}{params:?}"),
                _ => String::new(),
            };
            let outs: BTreeMap<&str, Vec<String>> = b.outs.iter().fold(BTreeMap::new(), |mut m, c| {
                m.entry(c.socket.as_str()).or_default().push(format!("{}({})", lib.blocks[c.target].short(), c.input));
                m
            });
            println!("{} {} {} [{}]{extra}", guid_hex(&b.guid), b.file, b.class, b.label);
            for (s, t) in outs {
                println!("    {s:?} -> {}", t.join(" "));
            }
            // Who feeds it (same graph), per input socket: what an AndBlock waits for.
            for input in lib.input_sockets(i) {
                let mut from: Vec<String> = Vec::new();
                for x in lib.blocks.iter().filter(|x| x.file == b.file && x.graph == b.graph) {
                    for c in x.outs.iter().filter(|c| c.target == i && c.input == input) {
                        from.push(format!("{}({})", x.short(), c.socket));
                    }
                }
                println!("    {input:?} <- {}", from.join(" "));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
