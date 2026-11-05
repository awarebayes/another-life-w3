//! `w3al tree`, `w3al graph-diff`: name-aware views of whole top-level blocks (`questSystem`,
//! `community`, ...), and the "graph delta" TOML that `journal-set --graph-from-toml` replays.
//! Research tools for the quest graph (docs/findings/quest-graph-design.md).
use std::path::Path;
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use w3al_core::graph_effect::{self, GuardedOp};
use w3save::graph::{self, Change, GraftOp};
use w3save::{Container, Document};

pub fn load_doc(path: &Path) -> Result<Document> {
    let file = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let c = Container::read(&file)?;
    Ok(Document::parse(&c.payload, c.data_start)?)
}

fn wanted(text: &str, grep: Option<&str>) -> bool {
    grep.is_none_or(|g| text.to_lowercase().contains(&g.to_lowercase()))
}

fn stem(p: &Path) -> String {
    p.file_stem().map_or_else(|| p.display().to_string(), |s| s.to_string_lossy().into_owned())
}

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

pub fn tree_cmd(sav: &Path, block: &str, grep: Option<&str>, depth: Option<usize>, path: Option<&str>) -> Result<ExitCode> {
    let d = load_doc(sav)?;
    let n = match path {
        Some(p) => {
            let loc = graph::locate(&d, p)?.with_context(|| format!("no node at {p}"))?;
            let n = graph::node_at(&d, &loc);
            println!("# {p}  hash {}", graph::short_hash(graph::fingerprint(&d, n).as_bytes()));
            n
        }
        None => graph::top_block(&d, block).with_context(|| format!("no top-level block {block:?}; blocks: {}", graph::top_block_names(&d).join(", ")))?,
    };
    for l in graph::render(&d, n) {
        if depth.is_some_and(|m| l.depth > m) || !wanted(&format!("{} {}", l.path, l.text), grep) {
            continue;
        }
        if grep.is_some() {
            println!("{}  {}", l.path, l.text);
        } else {
            println!("{}{}", "  ".repeat(l.depth), l.text);
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub const DEFAULT_BLOCKS: [&str; 5] = ["questSystem", "community", "gameProp", "worldState", "universe"];

pub struct DiffArgs<'a> {
    pub a: &'a Path,
    pub b: &'a Path,
    pub blocks: &'a [String],
    pub grep: Option<&'a str>,
    /// Print a replayable TOML block under `[effects.<key>.graph]`.
    pub toml: Option<&'a str>,
    /// Only changes under these paths go into the TOML block.
    pub scopes: &'a [String],
    /// Key stubs by `idTag` (`graph::DiffOptions::stub_identity`).
    pub stub_id: bool,
}

pub fn graph_diff_cmd(a: &DiffArgs) -> Result<ExitCode> {
    let (da, db) = (load_doc(a.a)?, load_doc(a.b)?);
    let blocks: Vec<String> = if a.blocks.is_empty() { DEFAULT_BLOCKS.iter().map(|s| (*s).to_owned()).collect() } else { a.blocks.to_vec() };
    println!("graph diff {} -> {}", a.a.display(), a.b.display());
    println!("NM names: A {} B {}; A's names a prefix of B's: {}", name_count(&da), name_count(&db), graph::names_prefix(&da, &db));
    let mut all = Vec::new();
    for name in &blocks {
        let Some(changes) = graph::diff_top_with(&da, &db, name, graph::DiffOptions { stub_identity: a.stub_id })? else {
            println!("\n## {name}: in neither save");
            continue;
        };
        let shown: Vec<&Change> = changes.iter().filter(|c| wanted(&format!("{c:?}"), a.grep)).collect();
        let (sa, sb) = (graph::top_block(&da, name).map_or(0, |n| n.encoded_len()), graph::top_block(&db, name).map_or(0, |n| n.encoded_len()));
        let same = graph::top_block(&da, name).map(|n| n.to_bytes()) == graph::top_block(&db, name).map(|n| n.to_bytes());
        println!("\n## {name}: {} changes ({} shown), {sa} -> {sb} bytes, byte-identical: {same}", changes.len(), shown.len());
        for c in shown {
            match c {
                Change::Removed { path, node } => println!("- {path}  {node}"),
                Change::Added { path, node } => println!("+ {path}  {node}"),
                Change::Changed { path, a, b } => println!("~ {path}  {a}  ->  {b}"),
                Change::Reordered { path } => println!("o {path}  (children reordered)"),
            }
        }
        all.extend(changes);
    }
    if let Some(key) = a.toml {
        let ops = graph::ops_from_diff(&all, a.scopes);
        println!("\n# ---- graph delta: {} op(s); replay with `w3al journal-set --graph-from-toml FILE --graph-donor <B.sav>`", ops.len());
        println!("{}", ops_toml(key, &ops, &da, &db, a.a, a.b)?);
    }
    Ok(ExitCode::SUCCESS)
}

fn name_count(d: &Document) -> usize {
    d.names.as_ref().map_or(0, Vec::len)
}

fn hash_at(d: &Document, path: &str) -> Result<Option<String>> {
    Ok(graph_effect::guard_hash(d, path)?)
}

fn line_at(d: &Document, path: &str) -> Result<String> {
    Ok(graph::locate(d, path)?.map_or_else(|| "(absent)".into(), |l| graph::node_line(d, graph::node_at(d, &l))))
}

/// The ops as TOML. `before` = hash of the template's node (A), `after` = hash of the donor's node (B);
/// `journal-set` refuses a template or donor whose node hashes differ (wrong save).
pub fn ops_toml(key: &str, ops: &[GraftOp], da: &Document, db: &Document, a: &Path, b: &Path) -> Result<String> {
    let mut s = format!(
        "[effects.{key}.graph]\nsource = {}\ndonor = {}\nops = [\n",
        toml_str(&format!("save diff {} -> {} (w3al graph-diff)", stem(a), stem(b))),
        toml_str(&stem(b))
    );
    for op in ops {
        let p = op.path();
        let (before, after) = (hash_at(da, p)?, hash_at(db, p)?);
        let opt = |h: Option<String>| h.map_or_else(|| "\"\"".into(), |h| toml_str(&h));
        let (kind, extra) = match op {
            GraftOp::Replace { .. } => ("replace", format!(", before = {}, after = {}", opt(before), opt(after))),
            GraftOp::Remove { .. } => ("remove", format!(", before = {}", opt(before))),
            GraftOp::SwapRemove { .. } => ("swap-remove", format!(", before = {}", opt(before))),
            GraftOp::Insert { .. } => ("insert", format!(", after = {}", opt(after))),
            GraftOp::Recount { of, .. } => ("recount", format!(", of = {}", toml_str(of))),
        };
        s.push_str(&format!("  # A: {}\n  # B: {}\n", line_at(da, p)?, line_at(db, p)?));
        s.push_str(&format!("  {{ op = \"{kind}\", path = {}{extra} }},\n", toml_str(p)));
    }
    s.push_str("]\n");
    Ok(s)
}

/// Ops of every `[effects.*.graph]` table in a file, plus the donor stems they name. The parser is
/// `w3al_core::graph_effect`, shared with the questbook and export.
pub fn ops_from_file(f: &Path) -> Result<(Vec<GuardedOp>, Vec<String>)> {
    let text = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
    let tables = graph_effect::tables_from_toml(&text).map_err(|m| anyhow!("parsing {}: {m}", f.display()))?;
    let (mut ops, mut donors) = (Vec::new(), Vec::new());
    for (key, g) in tables {
        donors.extend(g.donor.clone());
        ops.extend(g.to_ops().map_err(|m| anyhow!("{}: effects `{key}`: {m}", f.display()))?);
    }
    Ok((ops, donors))
}

/// `w3al stubs`: one line per NPC stub: list index, idTag (selector), spawn set file, appearance, state, position.
pub fn stubs_cmd(sav: &Path, grep: Option<&str>) -> Result<ExitCode> {
    let d = load_doc(sav)?;
    let all = graph::stubs(&d);
    let mut shown = 0;
    for s in &all {
        let pos = s.position.map_or_else(|| "-".into(), |p| format!("({:.1}, {:.1}, {:.1})", p[0], p[1], p[2]));
        let file = s.community.rsplit(['\\', '/']).next().unwrap_or(&s.community);
        let line = format!(
            "a#{:<3} {} {} app={} state={} pos={} entity={}",
            s.index,
            s.id_tag.as_deref().unwrap_or("-"),
            if file.is_empty() { "-" } else { file },
            s.appearance,
            s.state.map_or_else(|| "-".into(), |x| x.to_string()),
            pos,
            s.entries[2].get(..8).unwrap_or("-"),
        );
        if wanted(&line, grep) {
            println!("{line}");
            shown += 1;
        }
    }
    println!("# {} stubs, {shown} shown", all.len());
    Ok(ExitCode::SUCCESS)
}

/// Status of one guarded op on one save (see `w3al graph-check --help`).
fn op_status(d: &Document, g: &GuardedOp) -> Result<char> {
    let p = g.op.path();
    let here = hash_at(d, p)?;
    let parent_ok = || -> Result<bool> { Ok(p.rsplit_once('/').map(|x| graph::locate(d, x.0)).transpose()?.flatten().is_some()) };
    Ok(match &g.op {
        GraftOp::Replace { .. } => match here {
            None => '-',
            Some(h) if g.before.as_deref() == Some(h.as_str()) => 'B',
            Some(h) if g.after.as_deref() == Some(h.as_str()) => 'A',
            Some(_) => 'x',
        },
        GraftOp::Remove { .. } | GraftOp::SwapRemove { .. } => match here {
            None => 'A',
            Some(h) if g.before.is_none() || g.before.as_deref() == Some(h.as_str()) => 'B',
            Some(_) => 'x',
        },
        GraftOp::Insert { .. } => match here {
            None if parent_ok()? => 'B',
            None => '-',
            Some(h) if g.after.as_deref() == Some(h.as_str()) => 'A',
            Some(_) => 'x',
        },
        GraftOp::Recount { .. } => {
            if parent_ok()? {
                'B'
            } else {
                '-'
            }
        }
    })
}

/// `w3al graph-check`: a matrix of op status per save, and a per-save summary.
pub fn graph_check_cmd(tomls: &[std::path::PathBuf], saves: &[std::path::PathBuf], all: bool, base: Option<&Path>) -> Result<ExitCode> {
    let mut ops = Vec::new();
    for t in tomls {
        ops.extend(ops_from_file(t)?.0);
    }
    let docs: Vec<Document> = saves.iter().map(|s| load_doc(s)).collect::<Result<_>>()?;
    for (i, s) in saves.iter().enumerate() {
        println!("save {i}: {}", stem(s));
    }
    let mut table: Vec<Vec<char>> = Vec::new();
    for g in &ops {
        table.push(docs.iter().map(|d| op_status(d, g)).collect::<Result<_>>()?);
    }
    println!("\nstatus per save (B applies, A already done, - absent, x different):");
    for (g, row) in ops.iter().zip(&table) {
        if all || row.iter().any(|c| *c != 'B') {
            let kind = match g.op {
                GraftOp::Replace { .. } => "replace",
                GraftOp::Remove { .. } => "remove",
                GraftOp::SwapRemove { .. } => "swap-remove",
                GraftOp::Insert { .. } => "insert",
                GraftOp::Recount { .. } => "recount",
            };
            println!("  {}  {kind} {}", row.iter().collect::<String>(), g.op.path());
        }
    }
    println!("\nsummary ({} ops):", ops.len());
    for (i, s) in saves.iter().enumerate() {
        let count = |c: char| table.iter().filter(|r| r[i] == c).count();
        println!("  save {i} {}: B {} A {} - {} x {}", stem(s), count('B'), count('A'), count('-'), count('x'));
    }
    if let Some(b) = base {
        scope_drift(&load_doc(b)?, &ops, saves, &docs)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Per quest-graph parent the ops touch: blocks/threads a save has there beyond the template's and the ops' own.
fn scope_drift(base: &Document, ops: &[GuardedOp], saves: &[std::path::PathBuf], docs: &[Document]) -> Result<()> {
    let mut parents: Vec<&str> = Vec::new();
    for g in ops {
        if let Some((p, _)) = g.op.path().rsplit_once('/') {
            if p.starts_with("questSystem/quest[") && !matches!(g.op, GraftOp::Recount { .. }) && !parents.contains(&p) {
                parents.push(p);
            }
        }
    }
    let own: Vec<&str> = ops.iter().map(|g| g.op.path()).collect();
    let is_graph = |s: &String| s.starts_with("questBlock[") || s.starts_with("questThread[");
    println!("\nscope drift against the template (+ only in the save, - only in the template; the ops' own nodes left out):");
    for p in parents {
        let Some(bk) = graph::child_steps(base, p)? else { continue };
        let mine = |k: &String| own.contains(&format!("{p}/{k}").as_str());
        for (s, d) in saves.iter().zip(docs) {
            let line = match graph::child_steps(d, p)? {
                None => "scope absent".to_owned(),
                Some(tk) => {
                    let plus: Vec<&String> = tk.iter().filter(|k| is_graph(k) && !bk.contains(k) && !mine(k)).collect();
                    let minus: Vec<&String> = bk.iter().filter(|k| is_graph(k) && !tk.contains(k) && !mine(k)).collect();
                    if plus.is_empty() && minus.is_empty() {
                        continue;
                    }
                    let f = |v: Vec<&String>, c: char| v.iter().map(|k| format!("{c}{k}")).collect::<Vec<_>>().join(" ");
                    format!("{} {}", f(plus, '+'), f(minus, '-'))
                }
            };
            println!("  {}  {}: {line}", stem(s), p.rsplit('/').nth(1).unwrap_or(p));
        }
    }
    // Stubs the ops address by idTag that a save lacks: the same NPC may be there under another idTag (a respawn).
    let base_stubs = graph::stubs(base);
    let mut seen: Vec<String> = Vec::new();
    for g in ops {
        let Some(rest) = g.op.path().strip_prefix("community/stubs/a{idTag=") else { continue };
        let Some((id, _)) = rest.split_once('}') else { continue };
        if seen.iter().any(|x| x == id) {
            continue;
        }
        seen.push(id.to_owned());
        let Some(b) = base_stubs.iter().find(|s| s.id_tag.as_deref() == Some(id)) else { continue };
        for (s, d) in saves.iter().zip(docs) {
            let theirs = graph::stubs(d);
            if theirs.iter().any(|t| t.id_tag.as_deref() == Some(id)) {
                continue;
            }
            let same: Vec<String> = theirs
                .iter()
                .filter(|t| t.community == b.community && t.entries[2] == b.entries[2])
                .map(|t| t.id_tag.clone().unwrap_or_default())
                .collect();
            let file = b.community.rsplit(['\\', '/']).next().unwrap_or_default();
            let what = if same.is_empty() { "not there".to_owned() } else { format!("same spawn-set entry under idTag {}", same.join(", ")) };
            println!("  {}  stub {id} ({file}): {what}", stem(s));
        }
    }
    Ok(())
}
