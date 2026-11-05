//! The story chain: one save for the player's whole story up to the stop point.
//!
//! 1. [`crate::templates::choose`] picks the window template (the latest at or before the stop point
//!    whose earlier answers fit).
//! 2. Every earlier window where the player answered differently from the pack line the template holds
//!    is **rewritten after the fact** ([`window_delta`] + [`apply_delta`]): both answers are exported on
//!    that window's own template (the *references*, built by the normal export pipeline: synth walks,
//!    graph donors, journal tables, facts), and the difference between the two references is carried
//!    onto the later template:
//!    * facts: the decision facts of the pack answer leave, the player's are set (a later change of the
//!      same fact in the template is kept as an offset);
//!    * quest log: entries only the pack answer has are removed (the game removes entries itself: gate Y2),
//!      entries only the player's answer has are appended (Active children of a quest the template has
//!      finished are written as Success), entries both have with another status are set when the template
//!      still holds the pack status; objective counters likewise;
//!    * quest graph and spawn sets, only where the pack answer's state is still **live** in the template
//!      (a wait the decision created that nobody consumed yet, e.g. Keira's Kaer Morhen listener until the
//!      battle): those waits and scopes leave and the player's answer's waits are written with the
//!      synthesis writer (`w3graph::synth::apply` of a walk that only swaps blocks).
//! 3. The window's own quests are exported on the rewritten template as before (`export`).
//!
//! What is rewritable is decided per template in the registry (`fixed`): a decision whose consequence
//! has already played out in the template (a quest that read it has run: the Baron finale for the spirit,
//! the battle for the allies) is not, and the choice falls back to an earlier template. Evidence labels
//! and the per-decision table: docs/plan/story-chain.md.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use w3graph::frontier::{self, FrontierBlock};
use w3graph::quest::{Guid, Library, SpawnAction};
use w3graph::walk::{Event, Walk, WaitAt};
use w3save::journal::{self, CounterEdit, Entry, JournalEdit, StatusChange};
use w3save::{Container, Document, Kind, Severity};

use crate::export::{self, ExportError, ExportOptions, ExportResult};
use crate::questbook::Questbook;
use crate::resolve::{resolve_only, Resolution};
use crate::run::{QuestEntry, Run, Status};
use crate::templates::{self, Choice, Registry, Template};

fn doc_of(bytes: &[u8]) -> Result<Document, String> {
    let c = Container::read(bytes).map_err(|e| e.to_string())?;
    Document::parse(&c.payload, c.data_start).map_err(|e| e.to_string())
}

/// A fact's value for comparisons: its sum, `None` when absent or without records.
fn fact_value(t: &w3save::facts::FactTable, name: &str) -> Option<i64> {
    t.get(name).filter(|f| !f.records.is_empty()).map(w3save::facts::Fact::sum)
}

/// What one answer differs from another by, on a window's own template.
#[derive(Debug, Clone, Default)]
pub struct Delta {
    /// `(fact, value with answer A, value with answer B)`; values differ.
    pub facts: Vec<(String, Option<i64>, Option<i64>)>,
    /// Entries only A's reference has.
    pub journal_a: Vec<Entry>,
    /// Entries only B's reference has.
    pub journal_b: Vec<Entry>,
    /// Entries both have with another status or unread flag: `(A's, B's)`.
    pub journal_changed: Vec<(Entry, Entry)>,
    /// `(objective GUID, count with A, count with B)`.
    pub counters: Vec<(Guid, Option<i32>, Option<i32>)>,
    /// Waiting blocks only A's / only B's reference has.
    pub frontier_a: Vec<FrontierBlock>,
    pub frontier_b: Vec<FrontierBlock>,
    /// Running scopes only A's / only B's reference has.
    pub scopes_a: Vec<Guid>,
    pub scopes_b: Vec<Guid>,
    /// Waits both answers created (in both references, not in the window template): the decision's
    /// common aftermath. When the template still holds one, the window has not been consumed by later
    /// play and the player's own new waits are written even where the pack answer left nothing live.
    pub common_new: Vec<Guid>,
    /// `(spawn set, phase with A, phase with B)`; `None` = the set is not there.
    pub sets: Vec<(String, Option<String>, Option<String>)>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
            && self.journal_a.is_empty()
            && self.journal_b.is_empty()
            && self.journal_changed.is_empty()
            && self.counters.is_empty()
            && self.frontier_a.is_empty()
            && self.frontier_b.is_empty()
            && self.sets.is_empty()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} fact(s), journal -{} +{} ~{}, {} counter(s), waits -{} +{}, scopes -{} +{}, spawn sets {}",
            self.facts.len(),
            self.journal_a.len(),
            self.journal_b.len(),
            self.journal_changed.len(),
            self.counters.len(),
            self.frontier_a.len(),
            self.frontier_b.len(),
            self.scopes_a.len(),
            self.scopes_b.len(),
            self.sets.len()
        )
    }
}

/// The difference between reference A and reference B (both built from the same window template).
pub fn window_delta(w: &Document, a: &Document, b: &Document) -> Result<Delta, String> {
    let mut d = Delta::default();
    let (fa, fb) = (a.facts_table().map_err(|e| e.to_string())?, b.facts_table().map_err(|e| e.to_string())?);
    let mut names: BTreeSet<&str> = fa.facts().iter().map(|f| f.name.as_str()).collect();
    names.extend(fb.facts().iter().map(|f| f.name.as_str()));
    for n in names {
        if frontier::is_noise_fact(n) {
            continue;
        }
        let (va, vb) = (fact_value(&fa, n), fact_value(&fb, n));
        if va != vb {
            d.facts.push((n.to_owned(), va, vb));
        }
    }
    let (ja, jb) = (journal::read(a).map_err(|e| e.to_string())?, journal::read(b).map_err(|e| e.to_string())?);
    let jd = journal::diff(&ja, &jb);
    d.journal_a = jd.removed.into_iter().map(|x| x.1).collect();
    d.journal_b = jd.added.into_iter().map(|x| x.1).collect();
    d.journal_changed = jd.changed.into_iter().filter(|c| c.before.status != c.after.status).map(|c| (c.before, c.after)).collect();
    let ca: HashMap<Guid, i32> = ja.counters.iter().map(|c| (c.guid, c.count)).collect();
    let cb: HashMap<Guid, i32> = jb.counters.iter().map(|c| (c.guid, c.count)).collect();
    let mut keys: BTreeSet<Guid> = ca.keys().copied().collect();
    keys.extend(cb.keys().copied());
    for g in keys {
        let (x, y) = (ca.get(&g).copied(), cb.get(&g).copied());
        if x != y {
            d.counters.push((g, x, y));
        }
    }
    let (ra, rb) = (frontier::read(a).map_err(|e| e.to_string())?, frontier::read(b).map_err(|e| e.to_string())?);
    let (ga, gb) = (ra.guids(), rb.guids());
    d.frontier_a = ra.blocks.iter().filter(|x| !gb.contains(&x.guid)).cloned().collect();
    d.frontier_b = rb.blocks.iter().filter(|x| !ga.contains(&x.guid)).cloned().collect();
    let (sa, sb) = (ra.scope_guids(), rb.scope_guids());
    d.scopes_a = ra.scopes.iter().map(|s| s.0).filter(|g| !sb.contains(g)).collect();
    d.scopes_b = rb.scopes.iter().map(|s| s.0).filter(|g| !sa.contains(g)).collect();
    let gw = frontier::read(w).map_err(|e| e.to_string())?.guids();
    d.common_new = ga.intersection(&gb).filter(|g| !gw.contains(*g)).copied().collect();
    let (ca, cb) = (w3graph::community::state(a), w3graph::community::state(b));
    let pa: BTreeMap<&str, &str> = ca.sets.iter().map(|(s, p)| (s.as_str(), p.as_str())).collect();
    let pb: BTreeMap<&str, &str> = cb.sets.iter().map(|(s, p)| (s.as_str(), p.as_str())).collect();
    let mut sets: BTreeSet<&str> = pa.keys().copied().collect();
    sets.extend(pb.keys().copied());
    for s in sets {
        let (x, y) = (pa.get(s).map(|v| (*v).to_owned()), pb.get(s).map(|v| (*v).to_owned()));
        if x != y {
            d.sets.push((s.to_owned(), x, y));
        }
    }
    Ok(d)
}

/// What [`apply_delta`] did and what it expects of the result (for its verification).
#[derive(Debug, Clone, Default, Serialize)]
pub struct RewriteReport {
    pub facts_set: Vec<(String, i64)>,
    pub facts_removed: Vec<String>,
    /// Fact differences left as the template has them (a later change the delta cannot place), one line each.
    pub facts_kept: Vec<String>,
    pub journal_removed: usize,
    pub journal_appended: usize,
    pub journal_status_set: usize,
    pub journal_kept: Vec<String>,
    pub counters_removed: usize,
    pub counters_set: usize,
    /// The pack answer's waits / scopes still live in the template, swapped for the player's.
    pub live_blocks: Vec<String>,
    pub blocks_added: Vec<String>,
    pub sets_changed: Vec<String>,
    pub notes: Vec<String>,
}

impl RewriteReport {
    pub fn summary(&self) -> String {
        format!(
            "facts set [{}], removed [{}]; journal -{} +{} ~{}, counters -{} ~{}; waits removed [{}] added [{}]; spawn sets [{}]{}",
            self.facts_set.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "),
            self.facts_removed.join(" "),
            self.journal_removed,
            self.journal_appended,
            self.journal_status_set,
            self.counters_removed,
            self.counters_set,
            self.live_blocks.join(" "),
            self.blocks_added.join(" "),
            self.sets_changed.join(" "),
            self.notes.iter().chain(&self.facts_kept).chain(&self.journal_kept).map(|n| format!("; note: {n}")).collect::<String>()
        )
    }
}

/// Quest-graph files that hold listeners for the whole game (seen in every save's frontier), not one quest's state.
pub fn is_hub_file(file: &str) -> bool {
    ["character_journal_entries", "quest_npcs_default_dialogues", "bugfixing_phase", "bob_bugfix_phase", "persistent_content", "traveling_merchants", "witcher3_structure"]
        .iter()
        .any(|h| file.contains(h))
}

/// Hub files other than the character-journal listeners (which are per decision fact and are swapped).
pub fn is_world_hub_file(file: &str) -> bool {
    is_hub_file(file) && !file.contains("character_journal_entries")
}

/// Root entry key (group + quest) of an entry, when the path is that deep.
fn root_key(e: &Entry) -> Option<Vec<Guid>> {
    let k = e.key();
    (k.len() > 2).then(|| k[..2].to_vec())
}

/// Carry `d` (answer A -> answer B, measured on the window template) onto `t`, a later save that holds
/// answer A. Returns the new document, verified: facts, quest log and (when swapped) frontier as planned,
/// every other top-level block byte-identical.
pub fn apply_delta(lib: Option<&Library>, t: &Document, d: &Delta) -> Result<(Document, RewriteReport), String> {
    let mut rep = RewriteReport::default();
    let mut doc = t.clone();
    doc.rebase().map_err(|e| e.to_string())?;

    // 1. Live quest-graph and spawn-set state of answer A.
    //
    // Three kinds of waits: (a) the decision's own quest files: the window is *live* here when one of A's own waits,
    // or a wait both answers created (the common aftermath), still waits in the template; (b) character-journal
    // listeners (`character_journal_entries`), one per decision fact, swapped whenever A's is still here (Keira's
    // Kaer Morhen listener lives until the battle); (c) world hubs (persistent content, default dialogues, bug-fix
    // phases, merchants), never touched: they re-arm for the whole game and the game fires the ones whose facts the
    // rewrite makes true on load (gate J2). Spawn sets and A's own-file waits are only swapped while the window is live.
    let tf = frontier::read(&doc).map_err(|e| e.to_string())?;
    let tg = tf.guids();
    let ts = tf.scope_guids();
    #[derive(PartialEq)]
    enum Home {
        Own,
        Listener,
        World,
    }
    let home_of = |g: &Guid| -> Home {
        match lib.and_then(|l| l.get(g).map(|i| l.blocks[i].file.as_str())) {
            Some(f) if is_world_hub_file(f) => Home::World,
            Some(f) if is_hub_file(f) => Home::Listener,
            _ => Home::Own,
        }
    };
    let here_a: Vec<&FrontierBlock> = d.frontier_a.iter().filter(|b| tg.contains(&b.guid)).collect();
    let scopes_here: Vec<Guid> = d.scopes_a.iter().copied().filter(|g| ts.contains(g)).collect();
    let common_here: Vec<String> = d.common_new.iter().filter(|g| tg.contains(*g) && home_of(g) == Home::Own).map(w3graph::quest::short).collect();
    let window_live = here_a.iter().any(|b| home_of(&b.guid) == Home::Own) || scopes_here.iter().any(|g| home_of(g) == Home::Own) || !common_here.is_empty();
    let listeners_here = here_a.iter().any(|b| home_of(&b.guid) == Home::Listener) || scopes_here.iter().any(|g| home_of(g) == Home::Listener);
    let kept_world: Vec<String> = here_a.iter().filter(|b| home_of(&b.guid) == Home::World).map(|b| w3graph::quest::short(&b.guid)).collect();
    if !kept_world.is_empty() {
        rep.notes.push(format!("world-hub wait(s) of the pack answer left in place for the game: {}", kept_world.join(" ")));
    }
    if window_live && here_a.iter().all(|b| home_of(&b.guid) != Home::Own) && !common_here.is_empty() {
        rep.notes.push(format!("the window's common aftermath still waits here ({}): the player's own waits are written", common_here.join(" ")));
    }
    let swap = |h: &Home| *h == Home::Listener || (window_live && *h == Home::Own);
    let live: Vec<&FrontierBlock> = here_a.iter().copied().filter(|b| swap(&home_of(&b.guid))).collect();
    let live_scopes: Vec<Guid> = scopes_here.iter().copied().filter(|g| swap(&home_of(g))).collect();
    let mut actions = Vec::new();
    if window_live {
        let tc = w3graph::community::state(&doc);
        let t_sets: BTreeMap<&str, &str> = tc.sets.iter().map(|(s, p)| (s.as_str(), p.as_str())).collect();
        for (s, a, b) in &d.sets {
            let here = t_sets.get(s.as_str()).map(|p| (*p).to_owned());
            let act = match (a, b) {
                (Some(_), _) if here != *a => None, // the template moved this set on: leave it
                (_, Some(pb)) => Some(SpawnAction::Activate { spawnset: s.clone(), phase: pb.clone() }),
                (Some(_), None) => Some(SpawnAction::Deactivate { spawnset: s.clone() }),
                (None, None) => None,
            };
            if let Some(x) = act {
                actions.push(x);
                rep.sets_changed.push(format!("{s}: {} -> {}", a.as_deref().unwrap_or("-"), b.as_deref().unwrap_or("removed")));
            }
        }
    }
    let adds: Vec<&FrontierBlock> = d
        .frontier_b
        .iter()
        .filter(|b| match home_of(&b.guid) {
            Home::Own => window_live,
            Home::Listener => window_live || listeners_here,
            Home::World => false,
        })
        .collect();
    if !live.is_empty() || !live_scopes.is_empty() || !adds.is_empty() || !actions.is_empty() {
        let lib = lib.ok_or("the decision's quest-graph state is still live in the template; rewriting it needs the game's quest graphs")?;
        let idx = |g: &Guid| lib.get(g).ok_or_else(|| format!("block {} is not in the game's quest graphs", w3graph::quest::guid_hex(g)));
        let mut walk = Walk::default();
        for b in &live {
            walk.consumed.insert(idx(&b.guid)?);
            rep.live_blocks.push(w3graph::quest::short(&b.guid));
        }
        for g in &live_scopes {
            walk.exited.push(idx(g)?);
            rep.live_blocks.push(format!("scope {}", w3graph::quest::short(g)));
        }
        for b in adds {
            // The outermost scope must run in the template (and stay) for the wait to have a home.
            let home = b.scope.first().is_none_or(|g| ts.contains(g) && !live_scopes.contains(g));
            if !home {
                rep.notes.push(format!("wait {} of the player's answer has no running scope here; left to the game", w3graph::quest::short(&b.guid)));
                continue;
            }
            let bi = idx(&b.guid)?;
            let and = lib.blocks[bi].class == "CQuestAndBlock";
            walk.waits.push(WaitAt { block: bi, input: b.input.clone(), scope: b.scope.clone(), n_conds: b.n_conds, and_inputs: if and { b.inputs.clone() } else { Vec::new() } });
            rep.blocks_added.push(w3graph::quest::short(&b.guid));
        }
        walk.events = actions.into_iter().map(|action| Event::Spawn { block: 0, action }).collect();
        let (new_doc, sr) = w3graph::synth::apply(lib, &walk, &doc).map_err(|e| format!("live state swap: {e}"))?;
        rep.notes.extend(sr.notes);
        rep.notes.extend(sr.community.notes);
        doc = new_doc;
    }
    if !window_live && (!d.frontier_a.is_empty() || !d.sets.is_empty()) {
        rep.notes.push(format!(
            "the window's own state ({} wait(s), {} spawn-set change(s)) is no longer in the template (consumed by later play); only journal listeners swapped",
            d.frontier_a.len(),
            d.sets.len()
        ));
    }
    let graph_touched = !rep.live_blocks.is_empty() || !rep.blocks_added.is_empty() || !rep.sets_changed.is_empty();

    // 2. Quest log: removals, then status changes and appends, then counters.
    let tj = journal::read(&doc).map_err(|e| e.to_string())?;
    let remove_keys: Vec<Vec<Guid>> = d.journal_a.iter().map(Entry::key).filter(|k| !tj.find(k).is_empty()).collect();
    let remove_counters: Vec<Guid> =
        d.counters.iter().filter(|(_, _, b)| b.is_none()).map(|(g, _, _)| *g).filter(|g| tj.counters.iter().any(|c| c.guid == *g)).collect();
    let r = journal::remove(&mut doc, &remove_keys, &remove_counters).map_err(|e| format!("journal removal: {e}"))?;
    rep.journal_removed = r.entries_removed;
    rep.counters_removed = r.counters_removed;
    let tj = journal::read(&doc).map_err(|e| e.to_string())?;
    let status_of = |k: &[Guid]| tj.find(k).first().map(|&i| tj.entries[i].status.clone());
    // A quest root the template has finished: what the player's Active entries under it would be by now.
    let finished = |e: &Entry| root_key(e).and_then(|r| status_of(&r)).filter(|s| s == "JS_Success");
    let mut edit = JournalEdit::default();
    for (ea, eb) in &d.journal_changed {
        match status_of(&ea.key()) {
            Some(s) if s == ea.status => {
                let to = if eb.status == "JS_Active" { finished(eb).unwrap_or_else(|| eb.status.clone()) } else { eb.status.clone() };
                if to != s {
                    edit.status.push(StatusChange { key: eb.key(), from: Some(s), to, unread: Some(true) });
                }
            }
            Some(s) => rep.journal_kept.push(format!("{} is {s} here (the pack answer left it {}); kept", ea.label(), ea.status)),
            None => {}
        }
    }
    for eb in &d.journal_b {
        if !tj.find(&eb.key()).is_empty() {
            continue;
        }
        let mut e = eb.clone();
        if e.status == "JS_Active" {
            if let Some(s) = finished(&e) {
                e.status = s;
            }
        }
        edit.append.push(e);
    }
    for (g, _, b) in &d.counters {
        if let Some(n) = b {
            if tj.counters.iter().find(|c| c.guid == *g).map(|c| c.count) != Some(*n) {
                edit.counters.push(CounterEdit { guid: *g, count: *n, at: None });
            }
        }
    }
    if !edit.is_empty() {
        let r = journal::apply(&mut doc, &edit).map_err(|e| format!("journal: {e}"))?;
        rep.journal_appended = r.appended;
        rep.journal_status_set = r.status_set;
        rep.counters_set = r.counters_set + r.counters_appended;
    }

    // 3. Facts.
    let base = t.facts_table().map_err(|e| e.to_string())?;
    let mut sets: Vec<(String, i64)> = Vec::new();
    let mut removes: Vec<String> = Vec::new();
    for (n, a, b) in &d.facts {
        let here = fact_value(&base, n);
        match (a, b, here) {
            (_, None, Some(_)) => removes.push(n.clone()),
            (_, None, None) => {}
            (Some(av), Some(bv), Some(tv)) => {
                let v = tv - av + bv;
                sets.push((n.clone(), if v > 0 { v } else { *bv }));
            }
            (Some(_), Some(bv), None) => rep.facts_kept.push(format!("fact {n} is gone here (later play removed it); the player's value {bv} not set")),
            (None, Some(bv), None) => sets.push((n.clone(), *bv)),
            (None, Some(bv), Some(tv)) => rep.facts_kept.push(format!("fact {n} is {tv} here although the pack answer did not set it; the player's value {bv} not set")),
        }
    }
    let time = base.latest_time();
    doc.edit_facts(|tab| {
        for (n, v) in &sets {
            tab.set_value(n, i32::try_from(*v).unwrap_or(i32::MAX), time)?;
        }
        for n in &removes {
            tab.remove(n);
        }
        Ok(())
    })
    .map_err(|e| format!("facts: {e}"))?;
    rep.facts_set = sets;
    rep.facts_removed = removes;

    let reread = Document::parse(&doc.to_bytes(), doc.base).map_err(|e| e.to_string())?;
    let problems = verify_rewrite(t, &reread, &rep, &remove_keys, &edit, graph_touched);
    if !problems.is_empty() {
        return Err(format!("the rewrite did not verify: {}", problems.join("; ")));
    }
    Ok((reread, rep))
}

/// Facts, quest log and untouched blocks after a rewrite: exactly the planned changes.
fn verify_rewrite(t: &Document, r: &Document, rep: &RewriteReport, removed: &[Vec<Guid>], edit: &JournalEdit, graph: bool) -> Vec<String> {
    let mut problems = Vec::new();
    if r.table() != r.map.variables || !r.indices_are_post_order() {
        problems.push("variable table / post-order".into());
    }
    match (t.facts_table(), r.facts_table()) {
        (Ok(a), Ok(b)) => {
            let want: HashMap<&str, Option<i64>> = rep
                .facts_set
                .iter()
                .map(|(k, v)| (k.as_str(), Some(*v)))
                .chain(rep.facts_removed.iter().map(|k| (k.as_str(), None)))
                .collect();
            for n in a.differing(&b) {
                match want.get(n.as_str()) {
                    None => problems.push(format!("fact {n} changed without being planned")),
                    Some(v) if fact_value(&b, &n) != *v => problems.push(format!("fact {n} is not {v:?}")),
                    _ => {}
                }
            }
        }
        _ => problems.push("facts do not read".into()),
    }
    match (journal::read(t), journal::read(r)) {
        (Ok(a), Ok(b)) => {
            let jd = journal::diff(&a, &b);
            for (_, e) in &jd.removed {
                if !removed.contains(&e.key()) {
                    problems.push(format!("journal entry {} removed without being planned", e.label()));
                }
            }
            for (_, e) in &jd.added {
                if !edit.append.iter().any(|x| x.key() == e.key()) {
                    problems.push(format!("journal entry {} appended without being planned", e.label()));
                }
            }
            for c in &jd.changed {
                if !edit.status.iter().any(|s| s.key == c.after.key()) && !edit.append.iter().any(|x| x.key() == c.after.key()) {
                    problems.push(format!("journal entry {} changed without being planned", c.after.label()));
                }
            }
        }
        _ => problems.push("journal does not read".into()),
    }
    let mut touched = vec!["facts", "CJournalManager", "CWitcherJournalManager"];
    if graph {
        touched.extend(["questSystem", "community", "universe"]);
    }
    let (a, b): (Vec<_>, Vec<_>) = (t.top_nodes().collect(), r.top_nodes().collect());
    if a.len() != b.len() {
        problems.push(format!("top-level node count {} -> {}", a.len(), b.len()));
    } else {
        for (x, y) in a.iter().zip(&b) {
            let name = w3save::graph::node_name(t, x);
            if matches!(x.kind, Kind::Manu(_)) || touched.contains(&name.as_str()) {
                continue;
            }
            if x.to_bytes() != y.to_bytes() {
                problems.push(format!("top-level {name} changed although the rewrite does not edit it"));
            }
        }
    }
    if !w3save::graph::names_prefix(t, r) {
        problems.push("NM names changed (not appended)".into());
    }
    problems
}

// ---------------------------------------------------------------------------------------------
// References and the whole run

/// Everything a story export needs besides the run.
pub struct Sources<'a> {
    pub book: &'a Questbook,
    pub registry: &'a Registry,
    /// Template folders (see [`templates::search_dirs`]).
    pub template_dirs: Vec<PathBuf>,
    pub opts: ExportOptions,
}

/// One rewritten window, for the report.
#[derive(Debug, Clone, Serialize)]
pub struct WindowRewrite {
    pub window: String,
    pub quests: Vec<String>,
    pub delta: String,
    pub report: RewriteReport,
}

fn run_with(run: &Run, quests: &[String], answer: impl Fn(&str) -> Option<QuestEntry>) -> Run {
    let mut r = run.clone();
    r.stop_after = None;
    for q in quests {
        match answer(q) {
            Some(e) => {
                r.quests.insert(q.clone(), e);
            }
            None => {
                r.quests.remove(q);
            }
        }
    }
    r
}

fn window_resolution(book: &Questbook, run: &Run, w: &Template) -> Result<Resolution, String> {
    let res = resolve_only(book, run, |q| w.quests.contains(&q.id));
    if res.has_errors() {
        let m: Vec<_> = res.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
        return Err(m.join("; "));
    }
    Ok(res)
}

/// Export one resolution on a window template (a reference), with the shared quest graphs.
fn reference(src: &Sources, lib: Option<&Library>, wbytes: &[u8], res: &Resolution) -> Result<Document, String> {
    let donors = export::load_donors(res, &src.opts.donor_dirs).map_err(|e| e.to_string())?;
    let built = export::build_with_sources(wbytes, res, &src.opts, &donors, lib).map_err(|e| e.to_string())?;
    doc_of(&built.bytes)
}

/// The two references of window `w` for a run on `chosen`: A = the window's quests with the answers
/// `chosen` holds (the pack line), B = with the player's answers. Returns `(window template, A, B)`.
pub fn references(src: &Sources, lib: Option<&Library>, run: &Run, chosen: &Template, w: &Template) -> Result<(Document, Document, Document), String> {
    let path = templates::find_file(&src.template_dirs, &w.stem)
        .ok_or_else(|| format!("window template {} ({}.sav) is needed to rewrite {} but is not in the template folders", w.id, w.stem, w.quests.join(", ")))?;
    let wbytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let run_a = run_with(run, &w.quests, |q| match chosen.encodes.get(q) {
        Some(enc) => Some(QuestEntry { status: Status::Completed, answers: enc.clone() }),
        None => run.quests.get(q).cloned(),
    });
    let run_b = run_with(run, &w.quests, |q| run.quests.get(q).cloned());
    let res_a = window_resolution(src.book, &run_a, w).map_err(|e| format!("{} (pack answers): {e}", w.id))?;
    let res_b = window_resolution(src.book, &run_b, w).map_err(|e| format!("{} (your answers): {e}", w.id))?;
    let a = reference(src, lib, &wbytes, &res_a).map_err(|e| format!("{} with the pack answers: {e}", w.id))?;
    let b = reference(src, lib, &wbytes, &res_b).map_err(|e| format!("{} with your answers: {e}", w.id))?;
    Ok((doc_of(&wbytes)?, a, b))
}

/// Rewrite every earlier window the choice marks for rewriting, in line order, on `doc` (the chosen
/// template). Returns the rewritten document and one entry per window.
pub fn rewrite_all(src: &Sources, lib: Option<&Library>, run: &Run, choice: &Choice, doc: &Document) -> Result<(Document, Vec<WindowRewrite>), String> {
    let mut windows: Vec<&Template> = Vec::new();
    for b in choice.rewrites() {
        let w = src.registry.window_of(&b.quest).ok_or_else(|| format!("{}: no window template to rewrite it from", b.quest))?;
        if !windows.iter().any(|x| x.id == w.id) {
            windows.push(w);
        }
    }
    windows.sort_by_key(|w| w.line);
    let mut cur = doc.clone();
    let mut out = Vec::new();
    for w in windows {
        let (wd, a, b) = references(src, lib, run, &choice.template, w)?;
        let d = window_delta(&wd, &a, &b)?;
        let (next, report) = apply_delta(lib, &cur, &d).map_err(|e| format!("{}: {e}", w.id))?;
        out.push(WindowRewrite {
            window: w.id.clone(),
            quests: choice.rewrites().filter(|b| w.quests.contains(&b.quest)).map(|b| b.quest.clone()).collect(),
            delta: d.summary(),
            report,
        });
        cur = next;
    }
    Ok((cur, out))
}

/// The result of a story export.
#[derive(Debug, Serialize)]
pub struct StoryExport {
    pub choice: Choice,
    pub rewrites: Vec<WindowRewrite>,
    pub result: ExportResult,
}

fn needs_graphs(res: &Resolution, choice: &Choice) -> bool {
    choice.rewrites().next().is_some() || res.effects.iter().any(|a| a.effect.has_synth())
}

/// Load the quest graphs from `opts.game_dir` (the same check as `export::load_graphs`).
pub fn load_library(opts: &ExportOptions) -> Result<Library, ExportError> {
    let dir = opts.game_dir();
    if !dir.join("content").join("content0").join("bundles").is_dir() {
        return Err(ExportError::Graphs(format!(
            "rewriting earlier decisions and synthesized effects read the quest graphs of your Witcher 3 install, but {} is not one; set the game folder",
            dir.display()
        )));
    }
    let lib = Library::load_install(&dir).map_err(|e| ExportError::Graphs(format!("{}: {e}", dir.display())))?;
    if lib.blocks.is_empty() {
        return Err(ExportError::Graphs(format!("{}: no quest graphs found in the bundles", dir.display())));
    }
    Ok(lib)
}

/// The built save, the notes (rewrites first, then the window's effects) and the rewrites.
pub type StoryBytes = (Vec<u8>, Vec<String>, Vec<WindowRewrite>);

/// Build the story save's bytes for a chosen template: rewrite the earlier windows, then export the
/// window's quests on the result. `lib` = the quest graphs when already loaded.
pub fn build_story(src: &Sources, run: &Run, choice: &Choice, template: &[u8], lib: Option<&Library>) -> Result<StoryBytes, ExportError> {
    let mut res = resolve_only(src.book, run, |q| choice.applied.contains(&q.id));
    // The side quests the run completes go on after the window's quests (`crate::side`), on the chosen template.
    res.effects.extend(crate::side::resolve(&src.book.side, run));
    let owned;
    let lib = match lib {
        Some(l) => Some(l),
        None if needs_graphs(&res, choice) => {
            owned = load_library(&src.opts)?;
            Some(&owned)
        }
        None => None,
    };
    let mut container = Container::read(template)?;
    let damage: Vec<_> = container.diagnostics().into_iter().filter(|i| i.severity == Severity::Error).map(|i| i.message).collect();
    if !damage.is_empty() {
        return Err(ExportError::Template(damage.join("; ")));
    }
    let doc = Document::parse(&container.payload, container.data_start)?;
    let (rewritten, rewrites) = rewrite_all(src, lib, run, choice, &doc).map_err(|e| ExportError::Effect(format!("rewriting earlier decisions: {e}")))?;
    let base_bytes = if rewrites.is_empty() {
        template.to_vec()
    } else {
        container.payload = rewritten.to_bytes();
        let bytes = container.write()?;
        // The rewritten template must itself be a clean save before the window's effects go on top.
        let info = w3save::inspect(&bytes)?;
        let bad: Vec<_> = info.issues.iter().filter(|i| i.severity == Severity::Error).map(|i| i.message.clone()).collect();
        if !bad.is_empty() {
            return Err(ExportError::Verify(format!("rewritten template: {}", bad.join("; "))));
        }
        bytes
    };
    let donors = export::load_donors(&res, &src.opts.donor_dirs)?;
    let built = export::build_with_sources(&base_bytes, &res, &src.opts, &donors, lib)?;
    let mut notes: Vec<String> = rewrites.iter().map(|w| format!("rewrote {} ({}): {}", w.window, w.quests.join(", "), w.report.summary())).collect();
    notes.extend(built.notes);
    Ok((built.bytes, notes, rewrites))
}

/// The whole story pipeline: choose the template (unless the run names one), rewrite, export, write
/// (`create_new`), re-read.
pub fn export_story(src: &Sources, run: &Run, out_dir: &Path) -> Result<StoryExport, ExportError> {
    let choice = templates::choose(src.book, src.registry, run, &|t| templates::find_file(&src.template_dirs, &t.stem)).map_err(ExportError::Template)?;
    if let Some(stem) = &src.opts.stem {
        export::check_stem(stem)?;
    }
    let path = choice.path.clone().ok_or_else(|| ExportError::Template("the chosen template file was not found".into()))?;
    let template = fs::read(&path)?;
    let (bytes, notes, rewrites) = build_story(src, run, &choice, &template, None)?;
    let stem = src.opts.stem.clone().unwrap_or_else(|| export::default_stem(&path, out_dir));
    let mut result = export::write_save_set_as(&path, out_dir, &stem, &bytes)?;
    if fs::read(&result.sav)? != bytes {
        return Err(ExportError::Verify(format!("{} on disk differs from the verified bytes; do not use it", result.sav.display())));
    }
    result.notes = notes;
    result.effects_applied = rewrites.len() + resolve_only(src.book, run, |q| choice.applied.contains(&q.id)).effects.len();
    Ok(StoryExport { choice, rewrites, result })
}

// ---------------------------------------------------------------------------------------------
// Scan: what a later template holds for an earlier window (the registry's `encodes`), and who reads
// the decision facts (evidence for `fixed`).

/// Every combination of the window's answers: quest -> (choice -> option).
fn combos(book: &Questbook, w: &Template) -> Vec<BTreeMap<String, BTreeMap<String, String>>> {
    let mut out: Vec<BTreeMap<String, BTreeMap<String, String>>> = vec![BTreeMap::new()];
    for q in w.quests.iter().filter_map(|q| book.get(q)) {
        let mut per_quest: Vec<BTreeMap<String, String>> = vec![BTreeMap::new()];
        for c in &q.choices {
            per_quest = per_quest
                .into_iter()
                .flat_map(|m| {
                    c.options.iter().map(move |o| {
                        let mut m = m.clone();
                        m.insert(c.id.clone(), o.id.clone());
                        m
                    })
                })
                .collect();
        }
        out = out
            .into_iter()
            .flat_map(|m| {
                per_quest.iter().map(move |a| {
                    let mut m = m.clone();
                    m.insert(q.id.clone(), a.clone());
                    m
                })
            })
            .collect();
    }
    out
}

/// One answer combination of a window, built on its template.
pub struct ScanCandidate {
    pub answers: BTreeMap<String, BTreeMap<String, String>>,
    /// `None` = the combination does not export (e.g. a reward choice without the ride to Emhyr).
    pub facts: Option<BTreeMap<String, i64>>,
    pub error: Option<String>,
}

/// Build every answer combination of window `w` on its template (references), for [`match_window`].
pub fn scan_window(src: &Sources, lib: Option<&Library>, w: &Template, max: usize) -> Result<Vec<ScanCandidate>, String> {
    let path = templates::find_file(&src.template_dirs, &w.stem).ok_or_else(|| format!("{}.sav not in the template folders", w.stem))?;
    let wbytes = fs::read(&path).map_err(|e| e.to_string())?;
    let all = combos(src.book, w);
    if all.len() > max {
        return Err(format!("{} has {} answer combinations (more than {max})", w.id, all.len()));
    }
    let mut out = Vec::new();
    for answers in all {
        let mut run = Run::default();
        for (q, a) in &answers {
            run.quests.insert(q.clone(), QuestEntry { status: Status::Completed, answers: a.clone() });
        }
        let built = window_resolution(src.book, &run, w).and_then(|res| reference(src, lib, &wbytes, &res));
        match built {
            Ok(d) => {
                let t = d.facts_table().map_err(|e| e.to_string())?;
                let facts = t.facts().iter().filter(|f| !f.records.is_empty() && !frontier::is_noise_fact(&f.name)).map(|f| (f.name.clone(), f.sum())).collect();
                out.push(ScanCandidate { answers, facts: Some(facts), error: None });
            }
            Err(e) => out.push(ScanCandidate { answers, facts: None, error: Some(e) }),
        }
    }
    Ok(out)
}

/// How well a candidate matches a save: the facts that vary between candidates and differ here.
#[derive(Debug, Clone, Serialize)]
pub struct WindowMatch {
    pub answers: BTreeMap<String, BTreeMap<String, String>>,
    pub mismatches: Vec<String>,
    /// Facts that vary between the candidates.
    pub varying: Vec<String>,
    /// The runner-up's distance (to judge how clear the match is).
    pub next_best: Option<usize>,
}

pub fn match_window(cands: &[ScanCandidate], t: &Document) -> Result<WindowMatch, String> {
    let ok: Vec<(&ScanCandidate, &BTreeMap<String, i64>)> = cands.iter().filter_map(|c| c.facts.as_ref().map(|f| (c, f))).collect();
    if ok.is_empty() {
        return Err("no answer combination of the window exports".into());
    }
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for (_, f) in &ok {
        names.extend(f.keys().map(String::as_str));
    }
    let varying: Vec<&str> = names.into_iter().filter(|n| ok.iter().any(|(_, f)| f.get(*n) != ok[0].1.get(*n))).collect();
    let tf = t.facts_table().map_err(|e| e.to_string())?;
    let mut scored: Vec<(usize, &ScanCandidate, Vec<String>)> = ok
        .iter()
        .map(|(c, f)| {
            let mm: Vec<String> = varying
                .iter()
                .filter_map(|n| {
                    let (want, here) = (f.get(*n).copied(), fact_value(&tf, n));
                    (want.is_some() != here.is_some()).then(|| format!("{n}: answer {want:?}, save {here:?}"))
                })
                .collect();
            (mm.len(), *c, mm)
        })
        .collect();
    scored.sort_by_key(|x| x.0);
    Ok(WindowMatch {
        answers: scored[0].1.answers.clone(),
        mismatches: scored[0].2.clone(),
        varying: varying.iter().map(|s| (*s).to_owned()).collect(),
        next_best: scored.get(1).map(|x| x.0),
    })
}

/// Who reads a fact in the quest graphs, and where that reader stands in a save.
#[derive(Debug, Clone, Serialize)]
pub struct Reader {
    pub fact: String,
    pub block: String,
    pub file: String,
    /// `waiting` (the reader waits in the save), `file running` (another block of its file waits),
    /// `file ran` (the save holds facts that file sets), `not started`.
    pub state: String,
}

fn cond_facts(c: &w3graph::quest::Cond, out: &mut Vec<String>) {
    match c {
        w3graph::quest::Cond::Fact { id, .. } => out.push(id.clone()),
        w3graph::quest::Cond::Logic { inner, .. } => inner.iter().for_each(|i| cond_facts(i, out)),
        _ => {}
    }
}

/// Readers of `facts` (fact conditions of pause / condition blocks) outside the files that set them,
/// with their state in `t`. Scene-file conditions are not covered (docs/plan/story-chain.md).
pub fn readers(lib: &Library, facts: &[String], t: &Document) -> Result<Vec<Reader>, String> {
    use w3graph::quest::BlockKind;
    let tf = frontier::read(t).map_err(|e| e.to_string())?;
    let waiting = tf.guids();
    let running_files: BTreeSet<&str> = tf.blocks.iter().filter_map(|b| lib.get(&b.guid)).map(|i| lib.blocks[i].file.as_str()).collect();
    let tfacts = t.facts_table().map_err(|e| e.to_string())?;
    let mut setters: HashMap<&str, BTreeSet<&str>> = HashMap::new(); // fact -> files
    for b in &lib.blocks {
        if let BlockKind::Fact { id, .. } = &b.kind {
            setters.entry(id.as_str()).or_default().insert(b.file.as_str());
        }
    }
    // A file "ran" when the save holds a fact that only that file's fact blocks set.
    let mut file_ran: HashMap<&str, bool> = HashMap::new();
    for (id, files) in &setters {
        if files.len() == 1 && !frontier::is_noise_fact(id) && fact_value(&tfacts, id).is_some() {
            file_ran.insert(files.iter().next().copied().unwrap_or_default(), true);
        }
    }
    let mut out = Vec::new();
    for b in &lib.blocks {
        let mut fs_ = Vec::new();
        match &b.kind {
            BlockKind::Pause { conds } => conds.iter().for_each(|c| cond_facts(c, &mut fs_)),
            BlockKind::Condition { cond: Some(c) } => cond_facts(c, &mut fs_),
            _ => continue,
        }
        for f in fs_.iter().filter(|f| facts.contains(f)) {
            if setters.get(f.as_str()).is_some_and(|s| s.contains(b.file.as_str())) {
                continue; // the decision's own file
            }
            let state = if waiting.contains(&b.guid) {
                "waiting"
            } else if running_files.contains(b.file.as_str()) {
                "file running"
            } else if file_ran.get(b.file.as_str()).copied().unwrap_or(false) {
                "file ran"
            } else {
                "not started"
            };
            out.push(Reader { fact: f.clone(), block: b.short(), file: b.file.clone(), state: state.into() });
        }
    }
    out.sort_by(|a, b| (&a.fact, &a.file, &a.block).cmp(&(&b.fact, &b.file, &b.block)));
    out.dedup_by(|a, b| a.fact == b.fact && a.block == b.block);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_key_needs_depth_three() {
        let e = |n: usize| Entry {
            path: (0..n).map(|i| journal::Step { guid: [i as u8; 16], resource: String::new(), flags: u8::from(i + 1 == n) }).collect(),
            status: "JS_Active".into(),
            unread: true,
        };
        assert_eq!(root_key(&e(2)), None);
        assert_eq!(root_key(&e(4)), Some(vec![[0; 16], [1; 16]]));
    }
}
