//! `w3al journal`, `w3al journal-diff`, `w3al journal-set`: the quest log (`CJournalManager` and
//! `CWitcherJournalManager`). Research and gate-J tools; the export path still refuses journal
//! effects (docs/findings/journal-editor-design.md).
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use w3al_core::{export, journal_effect, Effect};
use w3save::journal::{self, guid_hex, Entry, Guid, Journal, JournalDiff, JournalEdit, StatusChange, STATUSES};
use w3save::graph::{self, GraftOp};
use w3save::{Container, Document, Edits, Severity};

use crate::graph_cmd;

fn load_doc(path: &Path) -> Result<Document> {
    let file = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let c = Container::read(&file)?;
    Ok(Document::parse(&c.payload, c.data_start)?)
}

fn load_journal(path: &Path) -> Result<Journal> {
    let d = load_doc(path)?;
    journal::read(&d).with_context(|| format!("journal of {}", path.display()))
}

fn short(st: &str) -> &str {
    st.strip_prefix("JS_").unwrap_or(st)
}

fn wanted(e: &Entry, grep: Option<&str>) -> bool {
    grep.is_none_or(|g| {
        let g = g.to_lowercase();
        e.label().to_lowercase().contains(&g) || e.key_hex().contains(&g)
    })
}

/// Label of the entry whose last GUID is `g` (tracked quest / objective / counter).
fn owner(j: &Journal, g: &Guid) -> String {
    j.entries
        .iter()
        .find(|e| e.path.last().is_some_and(|s| &s.guid == g))
        .map_or_else(|| "(no entry)".into(), Entry::label)
}

fn opt_guid(g: &Option<Guid>) -> String {
    g.as_ref().map_or_else(|| "none".into(), guid_hex)
}

pub fn journal_cmd(sav: &Path, status: Option<&str>, grep: Option<&str>, counters: bool) -> Result<ExitCode> {
    let j = load_journal(sav)?;
    let want_status = status.map(|s| if s.starts_with("JS_") { s.to_owned() } else { format!("JS_{s}") });
    println!("{} journal entries (JActiveEntries Size {}) in {}", j.entries.len(), j.size, sav.display());
    println!("  #  status    unread depth  label  |  path GUIDs");
    let mut shown = 0usize;
    for (i, e) in j.entries.iter().enumerate() {
        if want_status.as_ref().is_some_and(|s| *s != e.status) || !wanted(e, grep) {
            continue;
        }
        shown += 1;
        println!(
            "{i:>4} {:<9} {:<6} {}  {}  |  {}",
            short(&e.status),
            if e.unread { "unread" } else { "" },
            e.path.len(),
            e.label(),
            e.key_hex()
        );
    }
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &j.entries {
        *by.entry(e.status.as_str()).or_default() += 1;
    }
    println!("{shown} shown; by status: {by:?}; duplicate keys: {}", j.duplicate_keys());
    if j.size as usize != j.entries.len() {
        println!("[warn ] Size {} != {} entries", j.size, j.entries.len());
    }
    println!("tracked quest         {} {}", opt_guid(&j.tracked_quest), j.tracked_quest.map(|g| owner(&j, &g)).unwrap_or_default());
    println!(
        "highlighted objective {} {}",
        opt_guid(&j.highlighted_objective),
        j.highlighted_objective.map(|g| owner(&j, &g)).unwrap_or_default()
    );
    println!("objective counters    {} (Size {:?})", j.counters.len(), j.counters_size);
    if counters {
        for c in &j.counters {
            println!("  {} count={} {}", guid_hex(&c.guid), c.count, owner(&j, &c.guid));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

fn stem(p: &Path) -> String {
    p.file_stem().map_or_else(|| p.display().to_string(), |s| s.to_string_lossy().into_owned())
}

/// The diff restricted to entries matching `grep` (counters follow their entry; tracking only without grep).
fn filtered(d: &JournalDiff, b: &Journal, grep: Option<&str>) -> JournalDiff {
    let Some(_) = grep else { return d.clone() };
    let mut f = JournalDiff { size: d.size, append_only: d.append_only, counters_order_kept: d.counters_order_kept, ..Default::default() };
    f.changed = d.changed.iter().filter(|c| wanted(&c.after, grep)).cloned().collect();
    f.added = d.added.iter().filter(|(_, e)| wanted(e, grep)).cloned().collect();
    f.removed = d.removed.iter().filter(|(_, e)| wanted(e, grep)).cloned().collect();
    let keep = |g: &Guid| b.entries.iter().any(|e| e.path.last().is_some_and(|s| &s.guid == g) && wanted(e, grep));
    f.counters_changed = d.counters_changed.iter().filter(|c| keep(&c.0)).copied().collect();
    f.counters_added = d.counters_added.iter().filter(|c| keep(&c.1.guid)).copied().collect();
    f
}

/// A ready-to-paste `[effects.<key>]` block for the questbook (questbook/SCHEMA.md, journal effects).
pub fn diff_toml(d: &JournalDiff, b: &Journal, key: &str, source: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("[effects.{key}]\nsource = {}\nplaceholder = false\n", toml_str(source)));
    for c in &d.changed {
        s.push_str(&format!("\n[[effects.{key}.journal_status]]\nlabel = {}\n", toml_str(&c.after.label())));
        let path: Vec<String> = c.after.key().iter().map(|g| toml_str(&guid_hex(g))).collect();
        s.push_str(&format!("path = [{}]\nfrom = {}\nto = {}\n", path.join(", "), toml_str(&c.before.status), toml_str(&c.after.status)));
        if c.before.unread != c.after.unread {
            s.push_str(&format!("unread = {}\n", c.after.unread));
        }
    }
    for (_, e) in &d.added {
        s.push_str(&format!("\n[[effects.{key}.journal_append]]\nlabel = {}\n", toml_str(&e.label())));
        s.push_str(&format!("status = {}\nunread = {}\nsteps = [\n", toml_str(&e.status), e.unread));
        for st in &e.path {
            s.push_str(&format!("  {{ guid = {}, resource = {} }},\n", toml_str(&guid_hex(&st.guid)), toml_str(&st.resource)));
        }
        s.push_str("]\n");
    }
    for c in d.to_edit().map(|e| e.counters).unwrap_or_default() {
        s.push_str(&format!(
            "\n[[effects.{key}.journal_counters]]   # {}\nguid = {}\ncount = {}\n",
            owner(b, &c.guid),
            toml_str(&guid_hex(&c.guid)),
            c.count
        ));
        if let Some(at) = c.at {
            s.push_str(&format!("at = {at}   # position in the 'after' save (hash order); drop it for another template\n"));
        }
    }
    if d.tracked_quest.is_some() || d.highlighted_objective.is_some() {
        s.push_str(&format!("\n[effects.{key}.journal_track]\n"));
        if let Some((_, Some(g))) = d.tracked_quest {
            s.push_str(&format!("quest = {}   # {}\n", toml_str(&guid_hex(&g)), owner(b, &g)));
        }
        if let Some((_, Some(g))) = d.highlighted_objective {
            s.push_str(&format!("objective = {}   # {}\n", toml_str(&guid_hex(&g)), owner(b, &g)));
        }
    }
    s
}

pub fn journal_diff_cmd(a: &Path, b: &Path, grep: Option<&str>, key: &str) -> Result<ExitCode> {
    let (da, db) = (load_doc(a)?, load_doc(b)?);
    let ja = journal::read(&da).with_context(|| format!("journal of {}", a.display()))?;
    let jb = journal::read(&db).with_context(|| format!("journal of {}", b.display()))?;
    let unmodelled = journal::unmodelled_changes(&da, &db);
    let full = journal::diff(&ja, &jb);
    let d = filtered(&full, &jb, grep);
    println!("A: {} entries (Size {}) {}", ja.entries.len(), ja.size, a.display());
    println!("B: {} entries (Size {}) {}", jb.entries.len(), jb.size, b.display());
    println!(
        "entries: changed={} added={} removed={}; append-only={} (A is an in-order prefix of B)",
        d.changed.len(),
        d.added.len(),
        d.removed.len(),
        full.append_only
    );
    for c in &d.changed {
        let unread = if c.before.unread != c.after.unread { format!(" unread {} -> {}", c.before.unread, c.after.unread) } else { String::new() };
        let path = if c.path_changed() { " [PATH DETAILS CHANGED]" } else { "" };
        println!(
            "~ #{}->#{} {} -> {}{unread}{path}  {}",
            c.a,
            c.b,
            short(&c.before.status),
            short(&c.after.status),
            c.after.label()
        );
    }
    for (j, e) in &d.added {
        println!("+ #{j} {}{}  {}", short(&e.status), if e.unread { " unread" } else { "" }, e.label());
    }
    for (i, e) in &d.removed {
        println!("- #{i} {}  {}", short(&e.status), e.label());
    }
    println!(
        "counters: {} -> {} (Size {:?} -> {:?}), changed={} added={} removed={}; old counters keep their order={}",
        ja.counters.len(),
        jb.counters.len(),
        ja.counters_size,
        jb.counters_size,
        d.counters_changed.len(),
        d.counters_added.len(),
        full.counters_removed.len(),
        full.counters_order_kept
    );
    for (g, x, y) in &d.counters_changed {
        println!("~ counter {} {x} -> {y}  {}", guid_hex(g), owner(&jb, g));
    }
    for (j, c) in &d.counters_added {
        println!("+ counter #{j} {} count={}  {}", guid_hex(&c.guid), c.count, owner(&jb, &c.guid));
    }
    if let Some((x, y)) = full.tracked_quest {
        println!("tracked quest:         {} -> {}  ({} -> {})", opt_guid(&x), opt_guid(&y), x.map(|g| owner(&ja, &g)).unwrap_or_default(), y.map(|g| owner(&jb, &g)).unwrap_or_default());
    }
    if let Some((x, y)) = full.highlighted_objective {
        println!("highlighted objective: {} -> {}  ({} -> {})", opt_guid(&x), opt_guid(&y), x.map(|g| owner(&ja, &g)).unwrap_or_default(), y.map(|g| owner(&jb, &g)).unwrap_or_default());
    }
    if !unmodelled.is_empty() {
        println!("note: CWitcherJournalManager tables not modelled (not in the TOML): {} changed", unmodelled.join(", "));
    }
    for n in ["JS_Inactive", "JS_Active", "JS_Success", "JS_Failed"] {
        if da.name_index(n).is_none() && jb.entries.iter().any(|e| e.status == n) {
            println!("note: A's name table has no {n}; journal-set needs --allow-new-names to write it");
        }
    }
    if grep.is_some() {
        println!("(filtered by --grep; tracking changes are not part of the TOML below)");
    }
    if d.is_empty() {
        println!("no journal differences");
        return Ok(ExitCode::SUCCESS);
    }
    let source = format!("save diff {} -> {} (w3al journal-diff)", stem(a), stem(b));
    println!("\n# ---- questbook TOML (paste into the quest file; rename the effects key) ----");
    print!("{}", diff_toml(&d, &jb, key, &source));
    if !d.removed.is_empty() || d.changed.iter().any(|c| c.path_changed()) {
        println!("# WARNING: the diff removes entries or changes path details; journal-set cannot reproduce that");
    }
    Ok(ExitCode::SUCCESS)
}

/// `[effects.*]` tables of a TOML snippet (the output of `journal-diff`, or a quest file).
#[derive(Deserialize)]
struct Snippet {
    #[serde(default)]
    effects: BTreeMap<String, Effect>,
}

pub struct SetArgs<'a> {
    pub template: &'a Path,
    pub sets: &'a [String],
    pub append_from: &'a [String],
    pub from_toml: &'a [std::path::PathBuf],
    pub counters: &'a [String],
    pub track: Option<&'a str>,
    pub highlight: Option<&'a str>,
    pub facts: &'a [String],
    pub allow_new_names: bool,
    pub graph_donor: Option<&'a Path>,
    pub graph_remap: bool,
    pub graph_ops: &'a [String],
    pub graph_from_toml: &'a [std::path::PathBuf],
    pub swaps: &'a [std::path::PathBuf],
    pub stem: Option<&'a str>,
    pub out_dir: &'a Path,
}

/// `IDX` or a `/`-separated GUID path whose elements may be unique prefixes (8+ hex digits).
fn resolve_entry(j: &Journal, spec: &str) -> Result<usize> {
    if let Ok(i) = spec.parse::<usize>() {
        if i < j.entries.len() {
            return Ok(i);
        }
        bail!("entry index {i} out of range (0..{})", j.entries.len());
    }
    let parts: Vec<String> = spec.split('/').map(|s| s.trim().to_lowercase()).collect();
    if parts.iter().any(|p| p.len() < 8 || !p.bytes().all(|c| c.is_ascii_hexdigit())) {
        bail!("{spec:?}: expected an entry index or GUIDs (8+ hex digits each) separated by '/'");
    }
    let hits: Vec<usize> = j
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.path.len() == parts.len() && e.path.iter().zip(&parts).all(|(s, p)| guid_hex(&s.guid).starts_with(p.as_str())))
        .map(|(i, _)| i)
        .collect();
    match hits[..] {
        [i] => Ok(i),
        [] => bail!("no journal entry matches {spec:?}"),
        _ => bail!("{spec:?} matches {} entries; give longer GUIDs", hits.len()),
    }
}

fn status_name(s: &str) -> Result<String> {
    let s = if s.starts_with("JS_") { s.to_owned() } else { format!("JS_{s}") };
    if !STATUSES.contains(&s.as_str()) {
        bail!("unknown status {s:?} (expected one of {STATUSES:?})");
    }
    Ok(s)
}

fn split_last<'a>(s: &'a str, sep: char, what: &str) -> Result<(&'a str, &'a str)> {
    s.rsplit_once(sep).with_context(|| format!("{what} {s:?}: expected ...{sep}..."))
}

fn parse_fact(s: &str) -> Result<(String, i64)> {
    let (n, v) = split_last(s, '=', "--fact")?;
    Ok((n.to_owned(), v.trim().parse().with_context(|| format!("--fact {s:?}: value is not an integer"))?))
}

/// Everything `journal-set` was asked to do, as one edit plus facts.
fn build_edit(a: &SetArgs, template: &Journal) -> Result<(JournalEdit, Vec<(String, i64)>)> {
    let mut edit = JournalEdit::default();
    for s in a.sets {
        let (spec, st) = split_last(s, '=', "--set")?;
        let i = resolve_entry(template, spec)?;
        edit.status.push(StatusChange { key: template.entries[i].key(), from: None, to: status_name(st)?, unread: None });
    }
    let mut donors: BTreeMap<String, Journal> = BTreeMap::new();
    for s in a.append_from {
        let (path, spec) = split_last(s, ':', "--append-from")?;
        if !donors.contains_key(path) {
            donors.insert(path.to_owned(), load_journal(Path::new(path))?);
        }
        let dj = &donors[path];
        let i = resolve_entry(dj, spec)?;
        edit.append.push(dj.entries[i].clone());
    }
    let mut facts = Vec::new();
    for f in a.from_toml {
        let text = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        let snip: Snippet = toml::from_str(&text).with_context(|| format!("parsing {}", f.display()))?;
        if snip.effects.is_empty() {
            bail!("{}: no [effects.*] tables", f.display());
        }
        for (k, e) in &snip.effects {
            if e.placeholder {
                bail!("{}: effects `{k}` is a placeholder", f.display());
            }
            if !e.quests.is_empty() || !e.journal.is_empty() {
                bail!("{}: effects `{k}` has quests/journal ids, which are not supported", f.display());
            }
            journal_effect::merge(&mut edit, e.journal_edit().map_err(|m| anyhow::anyhow!("{}: effects `{k}`: {m}", f.display()))?);
            facts.extend(e.facts.iter().map(|(n, v)| (n.clone(), *v)));
        }
    }
    for c in a.counters {
        let (g, n) = split_last(c, '=', "--counter")?;
        edit.counters.push(journal::CounterEdit { guid: journal::parse_guid(g)?, count: n.trim().parse().context("--counter count")?, at: None });
    }
    if let Some(t) = a.track {
        edit.tracked_quest = Some(journal::parse_guid(t)?);
    }
    if let Some(h) = a.highlight {
        edit.highlighted_objective = Some(journal::parse_guid(h)?);
    }
    for f in a.facts {
        facts.push(parse_fact(f)?);
    }
    edit.allow_new_names = a.allow_new_names;
    Ok((edit, facts))
}

pub fn journal_set_cmd(a: &SetArgs) -> Result<ExitCode> {
    if let Some(s) = a.stem {
        export::check_stem(s)?;
    }
    let template_bytes = std::fs::read(a.template).with_context(|| format!("reading {}", a.template.display()))?;
    let tc = Container::read(&template_bytes)?;
    let tdoc = Document::parse(&tc.payload, tc.data_start)?;
    let before = journal::read(&tdoc)?;
    let facts_before = tdoc.facts_table()?;
    let (mut edit, mut facts) = build_edit(a, &before)?;
    let mut graph_ops = Vec::new();
    let mut toml_donors = Vec::new();
    for s in a.graph_ops {
        graph_ops.push(w3al_core::graph_effect::parse_op(s).map_err(|m| anyhow::anyhow!("--graph-op {s:?}: {m}"))?);
    }
    for f in a.graph_from_toml {
        let (ops, donors) = graph_cmd::ops_from_file(f)?;
        graph_ops.extend(ops);
        toml_donors.extend(donors);
    }
    let donor = match (a.graph_donor, graph_ops.is_empty()) {
        (Some(p), false) => {
            if let Some(d) = toml_donors.iter().find(|d| **d != stem(p)) {
                bail!("the graph delta was captured from {d}, but --graph-donor is {}", stem(p));
            }
            Some(graph_cmd::load_doc(p)?)
        }
        (None, false) => bail!("graph ops need --graph-donor (the save the subtrees are copied from)"),
        (Some(_), true) => bail!("--graph-donor without --graph-op or --graph-from-toml"),
        (None, true) => None,
    };
    if let Some(d) = &donor {
        w3al_core::graph_effect::check_guards(&graph_ops, &tdoc, Some(d)).map_err(|m| anyhow::anyhow!("{m}"))?;
    }
    // Output swaps: the guards above checked the captured delta (output A); now rewrite it to output B
    // and copy from a donor whose waiting blocks are rekeyed.
    let mut donor = donor;
    for f in a.swaps {
        let text = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        let swap = w3al_core::branch_swap::from_toml(&text).map_err(|m| anyhow::anyhow!("{}: {m}", f.display()))?;
        let s = swap.apply(&graph_ops, &edit, &facts).map_err(|m| anyhow::anyhow!("{}: {m}", f.display()))?;
        if !s.rekeys.is_empty() {
            let d = donor.as_ref().context("a swap that rekeys graph blocks needs --graph-donor")?;
            donor = Some(w3al_core::branch_swap::synth_donor(d, &s.rekeys)?);
        }
        println!("swap {} ({} rekeyed donor node(s)):", f.display(), s.rekeys.len());
        for l in &s.log {
            println!("  {l}");
        }
        w3al_core::graph_effect::check_guards(&s.ops, &tdoc, donor.as_ref()).map_err(|m| anyhow::anyhow!("{m}"))?;
        (graph_ops, edit, facts) = (s.ops, s.journal, s.facts);
    }
    let plain_ops: Vec<GraftOp> = graph_ops.iter().map(|g| g.op.clone()).collect();
    if edit.is_empty() && facts.is_empty() && plain_ops.is_empty() {
        bail!("nothing to do: give --set, --append-from, --from-toml, --counter, --track, --highlight, --fact or graph ops");
    }

    let mut report = None;
    let mut graph_report = None;
    let bytes = export::build_save_from_document(&template_bytes, |doc| {
        // Graph first: it may append the donor's NM names, which the journal edit then finds.
        if let Some(d) = &donor {
            let mode = if a.graph_remap { graph::NameMode::Remap } else { graph::NameMode::Prefix };
            graph_report = Some(graph::transplant_with(doc, d, &plain_ops, mode)?);
        }
        if !edit.is_empty() {
            report = Some(journal::apply(doc, &edit)?);
        }
        doc.apply(&Edits { facts: facts.clone(), ..Default::default() })
    })?;
    if let Some(r) = &graph_report {
        println!(
            "graph: {} replaced, {} removed, {} inserted, {} recounted; {} donor NM names appended; table {} -> {} entries",
            r.replaced,
            r.removed,
            r.inserted,
            r.recounted.len(),
            r.names_added,
            r.nodes.0,
            r.nodes.1
        );
        for (p, o, n) in &r.recounted {
            println!("  recount {p}: {o} -> {n}");
        }
    }
    if let Some(r) = &report {
        if !r.names_added.is_empty() {
            println!("NM names appended (HYPOTHESIS, untested in game): {}", r.names_added.join(", "));
        }
        println!(
            "journal: {} status set ({} already so), {} appended ({} already present), counters {} set / {} appended, {} tracking guid(s); Size {} -> {}; table {} -> {} entries",
            r.status_set, r.status_unchanged, r.appended, r.already_present, r.counters_set, r.counters_appended, r.tracking_set, r.size.0, r.size.1, r.nodes.0, r.nodes.1
        );
    }

    let stem = a.stem.map_or_else(|| export::default_stem(a.template, a.out_dir), str::to_owned);
    let written = export::write_save_set_as(a.template, a.out_dir, &stem, &bytes)?;
    println!("wrote {}", written.sav.display());
    for s in &written.sidecars {
        println!("      {}", s.display());
    }

    // Re-read what is on disk, independently of the in-memory result.
    let disk = std::fs::read(&written.sav)?;
    let c = Container::read(&disk)?;
    let doc = Document::parse(&c.payload, c.data_start)?;
    let mut problems = Vec::new();
    if doc.to_bytes() != c.payload {
        problems.push("typed re-parse does not serialize back to the payload".to_string());
    }
    if doc.table() != doc.map.variables {
        problems.push("variable table recomputed from the tree differs from the one on disk".into());
    }
    if !doc.indices_are_post_order() {
        problems.push("table indices are not in post-order".into());
    }
    for i in w3save::inspect(&disk)?.issues.iter().filter(|i| i.severity == Severity::Error) {
        problems.push(format!("check-save: {}", i.message));
    }
    if let Some(d) = &donor {
        // Every copied subtree on disk renders like the donor's.
        for op in plain_ops.iter().filter(|o| matches!(o, GraftOp::Replace { .. } | GraftOp::Insert { .. })) {
            let p = op.path();
            // Remap mode moves name indices, so compare by names (resolved fingerprint) there.
            let fp = |x: &Document| {
                graph::locate(x, p).ok().flatten().map(|l| {
                    let n = graph::node_at(x, &l);
                    if a.graph_remap {
                        graph::resolved_fingerprint(x, n).unwrap_or_default()
                    } else {
                        graph::fingerprint(x, n)
                    }
                })
            };
            if fp(&doc).is_none() || fp(&doc) != fp(d) {
                problems.push(format!("graph node {p} on disk differs from the donor's"));
            }
        }
    }
    let after = journal::read(&doc)?;
    // Applying the same edit again must change nothing (everything requested is in the file).
    let mut again = doc.clone();
    if !edit.is_empty() {
        let r = journal::apply(&mut again, &edit)?;
        if r.status_set != 0 || r.appended != 0 || r.counters_appended != 0 || again.to_bytes() != doc.to_bytes() {
            problems.push(format!("re-applying the journal edit still changes the file ({r:?})"));
        }
    }
    let fa = doc.facts_table()?;
    for (n, v) in &facts {
        if fa.get(n).map(|f| f.sum()) != Some(*v) {
            problems.push(format!("fact {n} is not {v}"));
        }
    }
    let changed_facts = facts_before.differing(&fa);
    for n in &changed_facts {
        if !facts.iter().any(|f| &f.0 == n) {
            problems.push(format!("fact {n} changed without being asked"));
        }
    }
    let d = journal::diff(&before, &after);
    println!(
        "re-read: {} entries (Size {}), {} counters, tracked {}, highlighted {}; vs template: {} changed, {} added, {} removed; {} fact(s) changed",
        after.entries.len(),
        after.size,
        after.counters.len(),
        opt_guid(&after.tracked_quest),
        opt_guid(&after.highlighted_objective),
        d.changed.len(),
        d.added.len(),
        d.removed.len(),
        changed_facts.len()
    );
    if problems.is_empty() {
        println!("VERIFIED: the file parses, table and post-order numbering hold, the edit is all there, nothing else in journal/facts changed, no check-save errors");
        Ok(ExitCode::SUCCESS)
    } else {
        for p in &problems {
            println!("PROBLEM: {p}");
        }
        bail!("verification failed: {}", problems[0])
    }
}
