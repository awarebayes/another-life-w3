//! Write a walk into a template save: the quest frontier (`questSystem`), the scene registry,
//! the journal and the facts, without any donor. Everything copied from a played save in the
//! transplant path (`w3save::graph::transplant`) is constructed here from the graph instead.
//!
//! What a new waiting block looks like in a 5.01 save (read from the user's own saves,
//! `w3al tree`; 2,812 blocks of T_keira, the KM / Radovid / spirit donors):
//! ```text
//! BS questBlock
//!   VL GUID:CGUID = <block GUID>
//!   VL inputNamesCount:Uint32 = 1          (2,809 of 2,812 blocks)
//!   VL inputName:CName = <socket it was activated through> ('' for phase blocks entered by an unnamed link)
//!   VL activationState:Int32 = 1           (2,795 of 2,812; 17 hold 0)
//!   [BS conditions { VL numConditions:Uint32; per condition BS p { VL nP:Uint32 = 1, OP active:Bool = true
//!                    (+ OP isFulfilled:Bool = false, nP = 2, for scripted `W3QuestCond_*` conditions) } }]
//!   [BS CutControlBlock { VL wasOutputActivated:Bool = false }]      (CQuestCutControlBlock)
//!   [BS treeState { VL numSaved:Uint16 = 0 }]                        (CQuestScriptedActionsBlock, fresh)
//!   [VL ActivatedInputs:array:2,0,CName = u32 count, u16 name..]     (CQuestAndBlock: one inputName per
//!                                                                     activated input, inputNamesCount = count)
//! BS questThread                            (a running phase block: next to its questBlock)
//!   VL GUID:CGUID = <phase block GUID>
//!   BS questThread { VL numBlocksToActivate:Uint32 = 0, VL numBlocks, questBlock.., VL numThreads, questThread..,
//!                    VL suspendedScopesData:array:2,0,SQuestThreadSuspensionData = 00000000, VL pausedThread:Bool = false }
//! ```
//! HYPOTHESES (checked offline against the played donors by `w3al synth-apply --donor`, not in
//! game): the layouts above are all a fresh wait needs; a scripted-actions block saves an empty
//! tree state until it runs; the game creates spawn-set stubs itself for story phases the walk
//! switched (the phase switch, new and removed sets and removed stubs are written by
//! [`crate::community`]; stub creation is left to the game).
use std::collections::{BTreeMap, HashMap, HashSet};

use w3save::graph::{self, step};
use w3save::journal::{self, CounterEdit, Entry, JournalEdit, StatusChange, Step};
use w3save::token::{Block, Kind, Node, Part, Value, Var, VarTag};
use w3save::Document;

use crate::community::{self, CommunityReport};
use crate::frontier;
use crate::quest::{guid_hex, short, BlockKind, Cond, Error, Guid, Library, SpawnAction};
use crate::walk::{Event, Policy, Walk, Walker};

#[derive(Debug, Default)]
pub struct SynthReport {
    pub removed_blocks: Vec<String>,
    pub removed_scopes: Vec<String>,
    pub added_blocks: Vec<String>,
    pub added_scopes: Vec<String>,
    pub input_changed: Vec<String>,
    pub scenes_removed: Vec<String>,
    pub scenes_added: Vec<String>,
    pub journal: journal::JournalReport,
    pub facts_set: Vec<(String, i64)>,
    pub facts_removed: Vec<String>,
    pub names_added: usize,
    pub notes: Vec<String>,
    /// Spawn-set edits (`community`, entity data).
    pub community: CommunityReport,
    /// Objective counters appended (`count 0`), by entry GUID.
    pub counters_added: Vec<Guid>,
}

/// The arguments of one decision walk, as `w3al synth-walk` / `synth-apply` and the questbook's
/// `[effects.<id>.synth]` take them. Keys are 8 or 32 hex digits of a block GUID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkSpec {
    /// Blocks to fire, `GUID[:input]` (the input defaults to the one the template saved).
    pub starts: Vec<String>,
    /// `GUID=socket`: the output to take at a decision block.
    pub picks: Vec<String>,
    /// `GUID=section`: the choice line to take inside that block's scene (by the name of the section
    /// the line leads to).
    pub scene_picks: Vec<String>,
    /// Waits to park at even in `through` mode.
    pub stops: Vec<String>,
    /// Single-output waits count as done.
    pub through: bool,
    /// Fire the template's own fact listeners the walk made true (what the game does on load).
    pub listeners: bool,
    pub max_steps: usize,
    /// Refuse a start block that is not waiting in the template (export wants this; the CLI only notes it).
    pub require_waiting_starts: bool,
}

impl Default for WalkSpec {
    fn default() -> Self {
        WalkSpec { starts: Vec::new(), picks: Vec::new(), scene_picks: Vec::new(), stops: Vec::new(), through: false, listeners: true, max_steps: 20_000, require_waiting_starts: true }
    }
}

/// Resolve a block key (8 or 32 hex digits) to a block, preferring the template's own frontier for
/// short keys the graphs hold several times.
pub fn resolve_block(lib: &Library, tf: &frontier::Frontier, key: &str) -> Result<usize, Error> {
    if key.len() == 8 {
        if let Some(b) = tf.blocks.iter().find(|b| short(&b.guid) == key.to_ascii_lowercase()) {
            if let Some(i) = lib.get(&b.guid) {
                return Ok(i);
            }
        }
    }
    lib.resolve(key)
}

/// Build the policy and starts of a spec and run the walk (`Walker::run`, then the listeners).
/// Returns the walk and the notes (starts not waiting in the template, when allowed). The walk may
/// hold unresolved points: see [`unresolved`].
pub fn plan(lib: &Library, template: &Document, tf: &frontier::Frontier, spec: &WalkSpec) -> Result<(Walk, Vec<String>), Error> {
    let sums = frontier::fact_sums(template)?;
    let base = |id: &str| sums.get(id).copied().unwrap_or(0);
    let waiting = tf.blocks.iter().map(|b| (b.guid, b.inputs.clone())).collect();
    let mut policy = Policy { through: spec.through, max_steps: spec.max_steps, running: tf.scope_guids(), waiting, ..Default::default() };
    for p in &spec.picks {
        let (k, s) = p.split_once('=').ok_or_else(|| Error::Other(format!("pick {p:?}: want GUID=socket")))?;
        let i = resolve_block(lib, tf, k)?;
        let b = &lib.blocks[i];
        if !b.sockets().contains(&s) {
            return Err(Error::Other(format!("pick: block {} has outputs {:?}, not {s:?}", lib.describe(i), b.sockets())));
        }
        policy.picks.insert(b.guid, s.to_owned());
    }
    for p in &spec.scene_picks {
        let (k, s) = p.split_once('=').ok_or_else(|| Error::Other(format!("scene pick {p:?}: want GUID=section")))?;
        let i = resolve_block(lib, tf, k)?;
        if !matches!(lib.blocks[i].kind, BlockKind::Scene { .. } | BlockKind::Interaction { .. }) {
            return Err(Error::Other(format!("scene pick: block {} plays no scene", lib.describe(i))));
        }
        policy.scene_picks.entry(lib.blocks[i].guid).or_default().push(s.to_owned());
    }
    for s in &spec.stops {
        let i = resolve_block(lib, tf, s)?;
        policy.stop.insert(lib.blocks[i].guid);
    }
    let mut starts = Vec::new();
    let mut notes = Vec::new();
    for s in &spec.starts {
        let (k, input) = s.split_once(':').map_or((s.as_str(), None), |(k, i)| (k, Some(i)));
        let i = resolve_block(lib, tf, k)?;
        let saved = tf.find(&lib.blocks[i].guid).map(|b| b.input.clone());
        if saved.is_none() {
            if spec.require_waiting_starts {
                return Err(Error::Other(format!("start {} is not waiting in the save (wrong template, or an earlier effect moved past it)", lib.describe(i))));
            }
            notes.push(format!("start {} is not waiting in the template", lib.describe(i)));
        }
        starts.push((i, input.map(str::to_owned).or(saved).unwrap_or_else(|| "In".into())));
    }
    if starts.is_empty() {
        return Err(Error::Other("give at least one start block (a block waiting in the template)".into()));
    }
    let frontier_blocks: Vec<(usize, String)> = tf.blocks.iter().filter_map(|b| lib.get(&b.guid).map(|i| (i, b.input.clone()))).collect();
    let mut w = Walker::new(lib, &base, policy);
    w.run(&starts);
    if spec.listeners {
        w.fire_listeners(&frontier_blocks);
    }
    Ok((w.into_walk(), notes))
}

/// The unresolved points of a walk, one line each (a walk with any must not be applied).
pub fn unresolved(lib: &Library, walk: &Walk) -> Vec<String> {
    walk.events
        .iter()
        .filter_map(|e| match e {
            Event::Unresolved { block, why } => Some(format!("{}: {why}", lib.describe(*block))),
            _ => None,
        })
        .collect()
}

/// The spawn-set actions of a walk, in order.
pub fn spawn_actions(walk: &Walk) -> Vec<SpawnAction> {
    walk.events
        .iter()
        .filter_map(|e| match e {
            Event::Spawn { action, .. } => Some(action.clone()),
            _ => None,
        })
        .collect()
}

struct Names {
    quest_block: u16,
    quest_thread: u16,
    guid: u16,
    input_names_count: u16,
    input_name: u16,
    activation_state: u16,
    conditions: u16,
    num_conditions: u16,
    p: u16,
    n_p: u16,
    active: u16,
    is_fulfilled: u16,
    num_blocks_to_activate: u16,
    num_blocks: u16,
    num_threads: u16,
    suspended: u16,
    paused: u16,
    cut_block: u16,
    was_output_activated: u16,
    tree_state: u16,
    num_saved: u16,
    activated_inputs: u16,
    t_cname_array: u16,
    external_dialog: u16,
    tag: u16,
    dialogs_count: u16,
    guid_lower: u16,
    t_guid: u16,
    t_u32: u16,
    t_cname: u16,
    t_i32: u16,
    t_bool: u16,
    t_u16: u16,
    t_susp: u16,
}

impl Names {
    fn intern(doc: &mut Document) -> Result<Names, Error> {
        let mut n = |s: &str| doc.intern_name(s);
        Ok(Names {
            quest_block: n("questBlock")?,
            quest_thread: n("questThread")?,
            guid: n("GUID")?,
            input_names_count: n("inputNamesCount")?,
            input_name: n("inputName")?,
            activation_state: n("activationState")?,
            conditions: n("conditions")?,
            num_conditions: n("numConditions")?,
            p: n("p")?,
            n_p: n("nP")?,
            active: n("active")?,
            is_fulfilled: n("isFulfilled")?,
            num_blocks_to_activate: n("numBlocksToActivate")?,
            num_blocks: n("numBlocks")?,
            num_threads: n("numThreads")?,
            suspended: n("suspendedScopesData")?,
            paused: n("pausedThread")?,
            cut_block: n("CutControlBlock")?,
            was_output_activated: n("wasOutputActivated")?,
            tree_state: n("treeState")?,
            num_saved: n("numSaved")?,
            activated_inputs: n("ActivatedInputs")?,
            t_cname_array: n("array:2,0,CName")?,
            external_dialog: n("ExternalDialog")?,
            tag: n("tag")?,
            dialogs_count: n("dialogsCount")?,
            guid_lower: n("guid")?,
            t_guid: n("CGUID")?,
            t_u32: n("Uint32")?,
            t_cname: n("CName")?,
            t_i32: n("Int32")?,
            t_bool: n("Bool")?,
            t_u16: n("Uint16")?,
            t_susp: n("array:2,0,SQuestThreadSuspensionData")?,
        })
    }
}

fn vl(name: u16, ty: u16, value: Value) -> Node {
    Node { index: 0, kind: Kind::Var(Var { tag: VarTag::Vl, name, ty, value }) }
}

fn op(name: u16, ty: u16, value: Value) -> Node {
    Node { index: 0, kind: Kind::Var(Var { tag: VarTag::Op, name, ty, value }) }
}

fn bs(name: u16, children: Vec<Node>) -> Node {
    Node { index: 0, kind: Kind::Block(Block { name, children }) }
}

fn is_scripted(c: &Cond) -> bool {
    match c {
        Cond::Other(class) => class.starts_with("W3QuestCond_"),
        Cond::Logic { inner, .. } => inner.iter().any(is_scripted),
        _ => false,
    }
}

/// A fresh waiting block. `and_inputs` = the activated inputs of a `CQuestAndBlock` (empty otherwise).
fn wait_node(doc: &mut Document, nm: &Names, lib: &Library, block: usize, input: &str, and_inputs: &[String]) -> Result<Node, Error> {
    let b = &lib.blocks[block];
    // An And block lists every input it has seen (`inputNamesCount` of them: pack 51 `a73fe883` holds
    // `In 2`, `In 1`), every other block the one it was activated through.
    let inputs: Vec<String> = if and_inputs.is_empty() { vec![input.to_owned()] } else { and_inputs.to_vec() };
    let mut kids = vec![vl(nm.guid, nm.t_guid, Value::Guid(b.guid)), vl(nm.input_names_count, nm.t_u32, Value::U32(inputs.len() as u32))];
    for i in &inputs {
        let idx = if i.is_empty() { 0 } else { doc.intern_name(i)? };
        kids.push(vl(nm.input_name, nm.t_cname, Value::CName(idx)));
    }
    kids.push(vl(nm.activation_state, nm.t_i32, Value::I32(1)));
    match &b.kind {
        BlockKind::Pause { conds } if !conds.is_empty() => {
            let mut ps = vec![vl(nm.num_conditions, nm.t_u32, Value::U32(conds.len() as u32))];
            for c in conds {
                let mut p = vec![vl(nm.n_p, nm.t_u32, Value::U32(if is_scripted(c) { 2 } else { 1 })), op(nm.active, nm.t_bool, Value::Bool(true))];
                if is_scripted(c) {
                    p.push(op(nm.is_fulfilled, nm.t_bool, Value::Bool(false)));
                }
                ps.push(bs(nm.p, p));
            }
            kids.push(bs(nm.conditions, ps));
        }
        _ if b.class == "CQuestCutControlBlock" => kids.push(bs(nm.cut_block, vec![vl(nm.was_output_activated, nm.t_bool, Value::Bool(false))])),
        _ if b.class == "CQuestScriptedActionsBlock" => kids.push(bs(nm.tree_state, vec![vl(nm.num_saved, nm.t_u16, Value::U16(0))])),
        _ if b.class == "CQuestAndBlock" => {
            // `VL ActivatedInputs:array:2,0,CName = u32 count, u16 name index each`, the same names as
            // the `inputName` vars (Letho invited donor `e9e84a64`: `010000003b03` = one input, `In 1`;
            // pack 51 `a73fe883`: `02000000620e3b03` = `In 2`, `In 1`).
            let mut bytes = (inputs.len() as u32).to_le_bytes().to_vec();
            for n in &inputs {
                let idx = if n.is_empty() { 0 } else { doc.intern_name(n)? };
                bytes.extend_from_slice(&idx.to_le_bytes());
            }
            kids.push(vl(nm.activated_inputs, nm.t_cname_array, Value::Opaque(vec![Part::Bytes(bytes)])));
        }
        _ => {}
    }
    Ok(bs(nm.quest_block, kids))
}

/// A fresh scope wrapper: `questThread[guid]` with an empty inner thread.
fn scope_node(nm: &Names, guid: Guid) -> Node {
    bs(
        nm.quest_thread,
        vec![
            vl(nm.guid, nm.t_guid, Value::Guid(guid)),
            bs(
                nm.quest_thread,
                vec![
                    vl(nm.num_blocks_to_activate, nm.t_u32, Value::U32(0)),
                    vl(nm.num_blocks, nm.t_u32, Value::U32(0)),
                    vl(nm.num_threads, nm.t_u32, Value::U32(0)),
                    vl(nm.suspended, nm.t_susp, Value::Opaque(vec![Part::Bytes(vec![0, 0, 0, 0])])),
                    vl(nm.paused, nm.t_bool, Value::Bool(false)),
                ],
            ),
        ],
    )
}

pub(crate) fn node_at_mut<'a>(doc: &'a mut Document, loc: &graph::Loc) -> &'a mut Node {
    let mut n = doc.top_nodes_mut().nth(loc.0).expect("located top node");
    for &i in &loc.1 {
        n = n.children_mut().into_iter().nth(i).expect("located child");
    }
    n
}

fn kids_mut(n: &mut Node) -> Result<&mut Vec<Node>, Error> {
    match &mut n.kind {
        Kind::Block(b) => Ok(&mut b.children),
        _ => Err(Error::Other("not a BS block".into())),
    }
}

/// Insert a child into the block at `path`, after the last sibling block of the same name
/// (blocks before threads, as the game orders them). Returns the new child's path.
fn insert_child(doc: &mut Document, path: &str, child: Node, child_name: u16, before_var: Option<u16>) -> Result<String, Error> {
    let loc = graph::locate(doc, path)?.ok_or_else(|| Error::Other(format!("{path} is not in the save")))?;
    let key = step(doc, &child);
    let parent = node_at_mut(doc, &loc);
    let kids = kids_mut(parent)?;
    // Position: after the last block of the same name; with none, right before `before_var`
    // (`numThreads` for a questBlock, `suspendedScopesData` for a questThread) or at the end.
    let mut at = kids.len();
    if let Some(last) = kids.iter().rposition(|k| matches!(&k.kind, Kind::Block(b) if b.name == child_name)) {
        at = last + 1;
    } else if let Some(v) = before_var {
        at = kids.iter().position(|k| matches!(&k.kind, Kind::Var(x) if x.name == v)).unwrap_or(at);
    }
    kids.insert(at, child);
    Ok(format!("{path}/{key}"))
}

/// Remove the child keyed `key` (as `step` prints it) from the block at `path`.
fn remove_child(doc: &mut Document, path: &str, key: &str) -> Result<bool, Error> {
    let full = format!("{path}/{key}");
    let Some((top, mut idx)) = graph::locate(doc, &full)? else { return Ok(false) };
    let last = idx.pop().ok_or_else(|| Error::Other("cannot remove a top-level block".into()))?;
    kids_mut(node_at_mut(doc, &(top, idx)))?.remove(last);
    Ok(true)
}

/// Set the count var `var` of the block at `path` to the number of its child blocks named `of`
/// (`graph::COUNT_RULES`).
pub(crate) fn recount_one(doc: &mut Document, path: &str, var: &str, of: &str) -> Result<(), Error> {
    let Some(loc) = graph::locate(doc, path)? else { return Ok(()) };
    let of_idx = doc.name_index(of);
    let var_idx = doc.name_index(var);
    let n = node_at_mut(doc, &loc);
    let kids = kids_mut(n)?;
    let count = kids.iter().filter(|k| matches!(&k.kind, Kind::Block(b) if Some(b.name) == of_idx)).count();
    for k in kids.iter_mut() {
        if let Kind::Var(v) = &mut k.kind {
            if Some(v.name) == var_idx {
                // `numStubs` is an Int32, the other counts are Uint32: keep the type.
                v.value = match v.value {
                    Value::I32(_) => Value::I32(i32::try_from(count).map_err(|_| Error::Other(format!("{path}/{var}: {count} does not fit")))?),
                    _ => Value::U32(u32::try_from(count).map_err(|_| Error::Other(format!("{path}/{var}: {count} does not fit")))?),
                };
            }
        }
    }
    Ok(())
}

fn recount(doc: &mut Document, thread_path: &str) -> Result<(), Error> {
    recount_one(doc, thread_path, "numBlocks", "questBlock")?;
    recount_one(doc, thread_path, "numThreads", "questThread")
}

/// The last status the walk gave each journal path, in first-seen order.
pub fn journal_items(walk: &Walk) -> Vec<(Vec<crate::quest::PathStep>, String)> {
    let mut order: Vec<Vec<Guid>> = Vec::new();
    let mut last: HashMap<Vec<Guid>, (Vec<crate::quest::PathStep>, String)> = HashMap::new();
    for e in &walk.events {
        if let Event::Journal { path, status, .. } = e {
            if path.is_empty() {
                continue;
            }
            let key: Vec<Guid> = path.iter().map(|s| s.guid).collect();
            if !last.contains_key(&key) {
                order.push(key.clone());
            }
            last.insert(key, (path.clone(), status.clone()));
        }
    }
    order.into_iter().filter_map(|k| last.remove(&k)).collect()
}

/// [`journal_edit_with`] plus the objective counters: a new entry that is a quest objective in the
/// `.journal` resources gets `count 0` (every objective entry of a save has one: `resources.rs`).
pub fn journal_edit_with(lib: &Library, walk: &Walk, template: &Document) -> Result<JournalEdit, Error> {
    let mut edit = journal_edit(walk, template)?;
    if !lib.has_resources(".journal") {
        return Ok(edit);
    }
    let objectives = lib.objectives()?;
    let tj = journal::read(template)?;
    for e in &edit.append {
        let Some(last) = e.key().last().copied() else { continue };
        if objectives.contains(&last) && !tj.counters.iter().any(|c| c.guid == last) && !edit.counters.iter().any(|c| c.guid == last) {
            edit.counters.push(CounterEdit { guid: last, count: 0, at: None });
        }
    }
    Ok(edit)
}

/// A new `JActiveEntries` entry for this path (flags: 1 on the last step, 0 before), unread.
fn entry_for(path: &[crate::quest::PathStep], status: &str) -> Entry {
    let n = path.len();
    Entry {
        path: path.iter().enumerate().map(|(i, s)| Step { guid: s.guid, resource: s.resource.clone(), flags: u8::from(i + 1 == n) }).collect(),
        status: status.to_owned(),
        unread: true,
    }
}

/// Build the journal edit: status flips for entries the template has, appends for new ones (with
/// the quest root and phase entries of a quest the log does not know yet), and the Success
/// propagation to the Active children of a quest set to Success. No counters (they need the
/// `.journal` resources: [`journal_edit_with`]).
pub fn journal_edit(walk: &Walk, template: &Document) -> Result<JournalEdit, Error> {
    let tj = journal::read(template)?;
    let mut edit = JournalEdit::default();
    let mut seen: HashSet<Vec<Guid>> = HashSet::new();
    for (path, status) in journal_items(walk) {
        let key: Vec<Guid> = path.iter().map(|s| s.guid).collect();
        seen.insert(key.clone());
        if let Some(&i) = tj.find(&key).first() {
            if tj.entries[i].status != status {
                edit.status.push(StatusChange { key: key.clone(), from: None, to: status.clone(), unread: Some(true) });
            }
        } else {
            // HYPOTHESIS (pack 44, old-gen, after Ugly Baby: `questact2/q402ciri`, its phase `dd17fe27` and the
            // ally quests' roots and phases are Active entries although only their objectives have journal
            // blocks): activating an objective of a quest the log does not know yet also creates the quest's
            // root and phase entries (every prefix of depth >= 2) as Active. The group (depth 1) is always there.
            for depth in 2..path.len() {
                let pk: Vec<Guid> = key[..depth].to_vec();
                if tj.find(&pk).is_empty() && !edit.append.iter().any(|e| e.key() == pk) {
                    edit.append.push(entry_for(&path[..depth], "JS_Active"));
                }
            }
            // An ancestor appended above may get its own journal block later in the walk: one entry.
            match edit.append.iter_mut().find(|e| e.key() == key) {
                Some(e) => e.status = status.clone(),
                None => edit.append.push(entry_for(&path, &status)),
            }
        }
        // HYPOTHESIS (21 -> 22: 5 flips; spirit donors: 4 each): a quest root set to Success
        // flips its Active entries to Success.
        if status == "JS_Success" && key.len() <= 2 {
            for e in &tj.entries {
                let k = e.key();
                if k.len() > key.len() && k.starts_with(&key) && e.status == "JS_Active" && seen.insert(k.clone()) {
                    edit.status.push(StatusChange { key: k, from: None, to: "JS_Success".into(), unread: Some(true) });
                }
            }
            // Entries this same walk appended as Active under the quest flip too (spirit donors).
            for e in &mut edit.append {
                let k = e.key();
                if k.len() > key.len() && k.starts_with(&key) && e.status == "JS_Active" {
                    e.status = "JS_Success".into();
                }
            }
            for s in &mut edit.status {
                if s.key.len() > key.len() && s.key.starts_with(&key) && s.to == "JS_Active" {
                    s.to = "JS_Success".into();
                }
            }
        }
    }
    Ok(edit)
}

/// Apply the walk to a copy of the template. Graph first, then journal, then facts; the result is
/// re-read from its bytes and its counts checked.
pub fn apply(lib: &Library, walk: &Walk, template: &Document) -> Result<(Document, SynthReport), Error> {
    let mut doc = template.clone();
    doc.rebase()?;
    let names_before = doc.names.as_ref().map_or(0, Vec::len);
    let nm = Names::intern(&mut doc)?;
    let tf = frontier::read(&doc)?;
    let mut rep = SynthReport::default();
    let reentered: HashSet<Guid> = walk.reentered.iter().map(|i| lib.blocks[*i].guid).collect();
    let exited: HashSet<Guid> = walk.exited.iter().map(|i| lib.blocks[*i].guid).collect();
    let consumed: HashSet<Guid> = walk.consumed.iter().map(|i| lib.blocks[*i].guid).collect();
    let mut touched: HashSet<String> = HashSet::new();

    // 1. Remove fired blocks and ended scopes (outermost first so inner paths are not needed).
    let mut removals: Vec<(String, String, bool)> = Vec::new(); // (parent path, key, is scope)
    for b in &tf.blocks {
        let inside_gone = b.scope.iter().any(|s| exited.contains(s) && !reentered.contains(s));
        if inside_gone {
            continue; // goes with its scope
        }
        let gone = (consumed.contains(&b.guid) && !reentered.contains(&b.guid)) || (exited.contains(&b.guid) && !reentered.contains(&b.guid));
        let inside_reentered = b.scope.iter().any(|s| reentered.contains(s));
        if gone || inside_reentered {
            let (parent, key) = b.path.rsplit_once('/').ok_or_else(|| Error::Other("bad frontier path".into()))?;
            removals.push((parent.to_owned(), key.to_owned(), false));
        }
    }
    // Template scopes that leave the save (with their parent, as exited, or as the old content of a
    // re-entered phase): their paths must not be reused for new waits (q309 `9c93af3f` Restart loop:
    // `e1bea12d` is removed with the old content and entered again fresh).
    let mut scopes_gone: HashSet<Guid> = HashSet::new();
    for (g, path) in &tf.scopes {
        let (parent, key) = path.rsplit_once('/').ok_or_else(|| Error::Other("bad scope path".into()))?;
        let parent_gone = tf.scopes.iter().any(|(s, p)| s != g && path.starts_with(&format!("{p}/")) && exited.contains(s) && !reentered.contains(s));
        if parent_gone {
            scopes_gone.insert(*g);
            continue;
        }
        let inside_reentered = tf.scopes.iter().any(|(s, p)| s != g && path.starts_with(&format!("{p}/")) && reentered.contains(s));
        if (exited.contains(g) && !reentered.contains(g)) || inside_reentered {
            scopes_gone.insert(*g);
            removals.push((parent.to_owned(), key.to_owned(), true));
        }
    }
    // Deeper paths first within the same parent do not matter (keys are GUIDs); remove in reverse path length so that
    // a removed inner node never precedes its parent's removal.
    removals.sort_by_key(|(p, _, _)| std::cmp::Reverse(p.len()));
    for (parent, key, is_scope) in &removals {
        if remove_child(&mut doc, parent, key)? {
            touched.insert(parent.clone());
            if *is_scope {
                rep.removed_scopes.push(format!("{parent}/{key}"));
            } else {
                rep.removed_blocks.push(format!("{parent}/{key}"));
            }
        }
    }

    // 1b. Suspended scopes the walk finished (a quest of another world): their entry leaves the array.
    for g in &walk.drop_suspended {
        let Some((_, var_path)) = tf.suspended.iter().find(|(s, _)| s == g) else {
            return Err(Error::Other(format!("suspended scope {} is not in the save", short(g))));
        };
        let loc = graph::locate(&doc, var_path)?.ok_or_else(|| Error::Other(format!("{var_path} vanished")))?;
        let n = node_at_mut(&mut doc, &loc);
        let Kind::Var(v) = &mut n.kind else { return Err(Error::Other(format!("{var_path} is not a var"))) };
        let Value::Opaque(parts) = &v.value else { return Err(Error::Other(format!("{var_path} is not opaque"))) };
        let mut s = frontier::suspended_entries(parts).ok_or_else(|| Error::Other(format!("{var_path}: unexpected layout")))?;
        if s.to_parts() != *parts {
            return Err(Error::Other(format!("{var_path}: does not re-encode byte for byte; not edited")));
        }
        s.entries.retain(|e| e.guid != *g);
        v.value = Value::Opaque(s.to_parts());
        rep.removed_scopes.push(format!("{var_path}[{}]", short(g)));
    }

    // 2. Re-entered phases: their questBlock takes the new input socket.
    for (i, e) in walk.events.iter().enumerate() {
        let Event::Enter { phase, socket } = e else { continue };
        let g = lib.blocks[*phase].guid;
        if !reentered.contains(&g) {
            continue;
        }
        // Only the last Enter of this phase counts.
        if walk.events[i + 1..].iter().any(|x| matches!(x, Event::Enter { phase: p, .. } if p == phase)) {
            continue;
        }
        if let Some(b) = tf.find(&g) {
            let idx = if socket.is_empty() { 0 } else { doc.intern_name(socket)? };
            let loc = graph::locate(&doc, &b.path)?.ok_or_else(|| Error::Other(format!("{} vanished", b.path)))?;
            let n = node_at_mut(&mut doc, &loc);
            for c in kids_mut(n)? {
                if let Kind::Var(v) = &mut c.kind {
                    if v.name == nm.input_name {
                        v.value = Value::CName(idx);
                        rep.input_changed.push(format!("{} -> {socket:?}", b.path));
                    }
                }
            }
        }
    }

    // 3. New scopes and waits. Scope paths: template wrappers by GUID, new wrappers created on demand.
    let mut scope_paths: BTreeMap<Guid, String> = tf.scopes.iter().filter(|(g, _)| !scopes_gone.contains(g)).map(|(g, p)| (*g, p.clone())).collect();
    let enter_socket: HashMap<Guid, String> = walk
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Enter { phase, socket } => Some((lib.blocks[*phase].guid, socket.clone())),
            _ => None,
        })
        .collect();
    let ensure_scope = |doc: &mut Document, scope_paths: &mut BTreeMap<Guid, String>, chain: &[Guid], rep: &mut SynthReport, touched: &mut HashSet<String>| -> Result<String, Error> {
        // The inner thread path of the deepest scope of `chain`, creating missing wrappers.
        let mut inner = String::new();
        let mut known = false;
        for (depth, g) in chain.iter().enumerate() {
            if let Some(p) = scope_paths.get(g) {
                inner = format!("{p}/questThread");
                known = true;
                continue;
            }
            if !known && depth == 0 {
                return Err(Error::Other(format!("scope {} is not in the template and has no parent there", short(g))));
            }
            if inner.is_empty() {
                return Err(Error::Other(format!("no path for scope {}", short(g))));
            }
            // A running phase = questBlock[g] (input = the socket it was entered through) + questThread[g].
            let socket = enter_socket.get(g).cloned().unwrap_or_default();
            let bi = lib.get(g).ok_or_else(|| Error::Other(format!("scope {} not in the graphs", short(g))))?;
            let qb = wait_node(doc, &nm, lib, bi, &socket, &[])?;
            let p1 = insert_child(doc, &inner, qb, nm.quest_block, Some(nm.num_threads))?;
            let p2 = insert_child(doc, &inner, scope_node(&nm, *g), nm.quest_thread, Some(nm.suspended))?;
            touched.insert(inner.clone());
            rep.added_blocks.push(p1);
            rep.added_scopes.push(p2.clone());
            scope_paths.insert(*g, p2.clone());
            inner = format!("{p2}/questThread");
        }
        Ok(inner)
    };
    for w in &walk.waits {
        let g = lib.blocks[w.block].guid;
        if let Some(old) = tf.find(&g) {
            if !consumed.contains(&g) && !w.scope.iter().any(|s| reentered.contains(s)) {
                if w.and_inputs.is_empty() || w.and_inputs == old.inputs {
                    continue; // already waiting there
                }
                // An And block the template holds that saw another input: rewritten with all of them.
                let (parent, key) = old.path.rsplit_once('/').ok_or_else(|| Error::Other("bad frontier path".into()))?;
                if remove_child(&mut doc, parent, key)? {
                    rep.removed_blocks.push(old.path.clone());
                    touched.insert(parent.to_owned());
                }
            }
        }
        let inner = ensure_scope(&mut doc, &mut scope_paths, &w.scope, &mut rep, &mut touched)?;
        let node = wait_node(&mut doc, &nm, lib, w.block, &w.input, &w.and_inputs)?;
        let p = insert_child(&mut doc, &inner, node, nm.quest_block, Some(nm.num_threads))?;
        touched.insert(inner);
        rep.added_blocks.push(p);
    }
    // Entered phases without waits inside still get their wrapper.
    for e in &walk.entered {
        let g = lib.blocks[*e].guid;
        if scope_paths.contains_key(&g) {
            continue;
        }
        let running = |x: &Guid| tf.scope_guids().contains(x) || scope_paths.contains_key(x);
        let mut chain = lib.scope_chain(*e, &running);
        chain.push(g);
        ensure_scope(&mut doc, &mut scope_paths, &chain, &mut rep, &mut touched)?;
    }

    // 4. Recount every touched thread (and the parents of removed scopes).
    for t in &touched {
        recount(&mut doc, t)?;
    }

    // 5. Scene registry: drop entries of fired interaction blocks, add entries for new ones.
    let players = "questSystem/questExternalScenePlayers/CQuestExternalScenePlayer";
    // An entry whose scene block no longer waits anywhere (fired, cut, or gone with its scope) leaves
    // the registry (101 of 101 saves: every registered guid is a waiting block).
    let now_waiting = frontier::read(&doc)?.guids();
    for (tag, g) in &tf.scenes {
        if !now_waiting.contains(g) && remove_child(&mut doc, players, &format!("ExternalDialog[{tag}]"))? {
            rep.scenes_removed.push(format!("{tag} {}", short(g)));
        }
    }
    for w in &walk.waits {
        let b = &lib.blocks[w.block];
        let BlockKind::Interaction { tags, .. } = &b.kind else { continue };
        if tf.scenes.iter().any(|(_, g)| *g == b.guid) {
            continue;
        }
        let Some(tag) = tags.first() else {
            rep.notes.push(format!("interaction {} has no actor tag; not registered", lib.describe(w.block)));
            continue;
        };
        let tag_idx = doc.intern_name(tag)?;
        let node = bs(
            nm.external_dialog,
            vec![vl(nm.tag, nm.t_cname, Value::CName(tag_idx)), vl(nm.dialogs_count, nm.t_u32, Value::U32(1)), vl(nm.guid_lower, nm.t_guid, Value::Guid(b.guid))],
        );
        insert_child(&mut doc, players, node, nm.external_dialog, None)?;
        rep.scenes_added.push(format!("{tag} {}", short(&b.guid)));
    }
    recount_one(&mut doc, players, "tagsCount", "ExternalDialog")?;

    // 6. Spawn sets: phase switches, new sets, deactivated sets with their stubs and entity data.
    community::apply_spawn(&mut doc, lib, &spawn_actions(walk), &mut rep.community)?;
    // New nodes carry index 0: give the table its post-order numbering before the journal and
    // fact editors (which rebase on entry) see it.
    doc.renumber_post_order();
    doc.rebase()?;

    // 7. Journal (with the objective counters of new objective entries).
    let edit = journal_edit_with(lib, walk, template)?;
    rep.counters_added = edit.counters.iter().map(|c| c.guid).collect();
    rep.journal = journal::apply(&mut doc, &edit)?;

    // 8. Facts.
    let base = frontier::fact_sums(template)?;
    let time = doc.facts_table()?.latest_time();
    doc.edit_facts(|t| {
        for (id, delta) in &walk.facts {
            if walk.removed_facts.contains(id) || *delta <= 0 {
                continue;
            }
            let v = base.get(id).copied().unwrap_or(0) + delta;
            t.set_value(id, i32::try_from(v).unwrap_or(i32::MAX), time)?;
            rep.facts_set.push((id.clone(), v));
        }
        for id in &walk.removed_facts {
            if t.remove(id).is_some() {
                rep.facts_removed.push(id.clone());
            }
        }
        Ok(())
    })?;

    doc.renumber_post_order();
    doc.rebase()?;
    let reread = Document::parse(&doc.to_bytes(), doc.base)?;
    if reread.table() != reread.map.variables || !reread.indices_are_post_order() {
        return Err(Error::Other("synthesized save does not re-read: variable table does not match the tree".into()));
    }
    // Every count rule must hold, and every new wait must be there.
    let rf = frontier::read(&reread)?;
    for w in &walk.waits {
        let g = lib.blocks[w.block].guid;
        if rf.find(&g).is_none() {
            return Err(Error::Other(format!("synthesized save lacks the new wait {}", guid_hex(&g))));
        }
    }
    rep.names_added = reread.names.as_ref().map_or(0, Vec::len) - names_before;
    Ok((reread, rep))
}
