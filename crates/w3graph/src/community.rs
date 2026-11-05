//! Spawn-set edits of the synthesis: what a `CQuestStoryPhaseSetterBlock` does to the save's
//! `community` block (and to the entity state that goes with a dropped stub).
//!
//! Layout (user's own 5.01 saves, `w3al tree --block community`):
//! ```text
//! BS community
//!   BS communities { VL numCommunities:Uint32, N x BS c { VL p:String = "<file>.w2comm",
//!       BS communityData { VL isActive:Bool, VL activePhase:CName, VL numCommunityTables:Uint32,
//!           N x BS tableEntry { VL GUID, BS tableEntryData { VL numPhases:Uint32,
//!               N x BS phaseEntry { VL GUID, BS phaseRtData { VL killedNPCsNum:Int32, VL spawnDelay:EngineTime } } } } } } }
//!   BS stubs { VL numStubs:Int32, N x BS a { BS storyPhaseLocation { community:String, storyEntry:CGUID, .. }, VL idTag:IdTag, .. } }
//! ```
//! and `universe > world[..] > layerStorage { VL numEntities:Uint32, N x BS entityData { VL idTag:IdTag, .. } }`.
//!
//! What the played donors show (session V1/V2, `graph-diff --stub-id`):
//! * `CActivateStoryPhase` on a set the save holds: `activePhase` changes (`keira_metz.w2comm`:
//!   `q109_island_laboratory` -> `q109_island_attacking`); the set's stub keeps its idTag and gets the
//!   new phase's `storyEntry`, appearance and position (the game re-spawned the NPC);
//! * `CActivateStoryPhase` on a set the save does not hold: a `c` block appears with every table
//!   entry and story phase of the `.w2comm` in file order, `killedNPCsNum 0`, zero `spawnDelay`
//!   (spirit released: `q105_boy1`, `q105_girl1..3`, `q105_boy2`, `q105_ghost_in_horse`,
//!   `e3_burrows_npcs`); its stubs appear with new idTags;
//! * `CDeactivateSpawnset`: the `c` block is gone (not `isActive = false`), its stubs are gone
//!   (`q105_evil_heart`: 1; Letho: `letho`, `sq102_letho_horse`, `sq102_midcots_boy`: 1 each) and so
//!   are the `entityData` entries carrying their idTags (Letho: the 3 of 3 that had one).
//!
//! What is written here: the phase switch and the new `c` block (from the `.w2comm`), the removal of
//! the deactivated set with its stubs and entity data, and the removal of the stubs of a phase that
//! was switched away from (so the game spawns the new phase's NPCs). HYPOTHESIS (gate K3): the game
//! creates the stubs of an active set / phase on load; stub creation is left to it.
use w3save::graph::{self, value_text};
use w3save::token::{Kind, Node, Value};
use w3save::Document;

use crate::quest::{normalize, parse_guid, Error, Library, SpawnAction};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommunityReport {
    /// `(set, old phase, new phase)`.
    pub phases_set: Vec<(String, String, String)>,
    pub sets_added: Vec<String>,
    pub sets_removed: Vec<String>,
    /// `(idTag, set)`.
    pub stubs_removed: Vec<(String, String)>,
    /// idTags whose `entityData` left `layerStorage`.
    pub entities_removed: Vec<String>,
    pub notes: Vec<String>,
}

fn kids_mut(n: &mut Node) -> Result<&mut Vec<Node>, Error> {
    match &mut n.kind {
        Kind::Block(b) => Ok(&mut b.children),
        _ => Err(Error::Other("not a BS block".into())),
    }
}

fn block_name<'a>(doc: &'a Document, n: &Node) -> Option<&'a str> {
    match &n.kind {
        Kind::Block(b) => doc.name(b.name),
        _ => None,
    }
}

fn var_value<'a>(doc: &Document, n: &'a Node, name: &str) -> Option<&'a Value> {
    n.children().into_iter().find_map(|c| match &c.kind {
        Kind::Var(v) if doc.name(v.name) == Some(name) => Some(&v.value),
        _ => None,
    })
}

/// Set a var by name index; `None` block index = in `n` itself, else inside that child block.
fn set_var_in(n: &mut Node, block: Option<u16>, var: u16, value: Value) -> bool {
    let target = match block {
        Some(b) => match n.children_mut().into_iter().find(|c| matches!(&c.kind, Kind::Block(x) if x.name == b)) {
            Some(t) => t,
            None => return false,
        },
        None => n,
    };
    for c in target.children_mut() {
        if let Kind::Var(v) = &mut c.kind {
            if v.name == var {
                v.value = value;
                return true;
            }
        }
    }
    false
}

fn child_block_mut(n: &mut Node, idx: u16) -> Option<&mut Node> {
    n.children_mut().into_iter().find(|c| matches!(&c.kind, Kind::Block(b) if b.name == idx))
}

fn child_block(n: &Node, idx: u16) -> Option<&Node> {
    n.children().into_iter().find(|c| matches!(&c.kind, Kind::Block(b) if b.name == idx))
}

/// The save's spelling of a spawn-set path (`quests\...\x.w2comm`, lower case, backslashes).
fn save_path(file: &str) -> String {
    normalize(file).replace('/', "\\")
}

fn c_path(doc: &Document, c: &Node) -> String {
    match var_value(doc, c, "p") {
        Some(Value::Str(s)) => normalize(s),
        _ => String::new(),
    }
}

/// `community/communities`, read-only.
fn communities(doc: &Document) -> Result<&Node, Error> {
    graph::top_block(doc, "community")
        .and_then(|c| c.children().into_iter().find(|k| block_name(doc, k) == Some("communities")))
        .ok_or_else(|| Error::Other("the save has no community/communities block".into()))
}

/// The children of `community/communities`, mutable (no name lookups while this borrow lives).
fn communities_kids_mut(doc: &mut Document) -> Result<&mut Vec<Node>, Error> {
    let loc = graph::locate(doc, "community/communities")?.ok_or_else(|| Error::Other("the save has no community/communities block".into()))?;
    kids_mut(crate::synth::node_at_mut(doc, &loc))
}

/// NM indices of the names a `c` block uses.
struct Nm {
    p: u16,
    data: u16,
    is_active: u16,
    active_phase: u16,
    num_tables: u16,
    table_entry: u16,
    guid: u16,
    table_data: u16,
    num_phases: u16,
    phase_entry: u16,
    rt: u16,
    killed: u16,
}

impl Nm {
    fn intern(doc: &mut Document) -> Result<Nm, Error> {
        let mut n = |s: &str| doc.intern_name(s);
        Ok(Nm {
            p: n("p")?,
            data: n("communityData")?,
            is_active: n("isActive")?,
            active_phase: n("activePhase")?,
            num_tables: n("numCommunityTables")?,
            table_entry: n("tableEntry")?,
            guid: n("GUID")?,
            table_data: n("tableEntryData")?,
            num_phases: n("numPhases")?,
            phase_entry: n("phaseEntry")?,
            rt: n("phaseRtData")?,
            killed: n("killedNPCsNum")?,
        })
    }
}

/// Build a fresh `c` block for `file` with the layout of `proto` (an existing `c` of the save) and
/// the tables / phases of the `.w2comm`.
fn new_c(nm: &Nm, proto: &Node, file: &str, phase: u16, comm: &crate::resources::Community) -> Result<Node, Error> {
    let mut c = proto.clone();
    if !set_var_in(&mut c, None, nm.p, Value::Str(save_path(file))) {
        return Err(Error::Other("community c block without p".into()));
    }
    let data = child_block_mut(&mut c, nm.data).ok_or_else(|| Error::Other("community c block without communityData".into()))?;
    set_var_in(data, None, nm.is_active, Value::Bool(true));
    set_var_in(data, None, nm.active_phase, Value::CName(phase));
    set_var_in(data, None, nm.num_tables, Value::U32(comm.tables.len() as u32));
    let kids = kids_mut(data)?;
    let is_table = |k: &Node| matches!(&k.kind, Kind::Block(b) if b.name == nm.table_entry);
    let proto_table = kids.iter().find(|k| is_table(k)).cloned().ok_or_else(|| Error::Other("community c block without tableEntry".into()))?;
    kids.retain(|k| !is_table(k));
    let proto_phase = child_block(&proto_table, nm.table_data)
        .and_then(|td| child_block(td, nm.phase_entry))
        .cloned()
        .ok_or_else(|| Error::Other("tableEntry without tableEntryData/phaseEntry".into()))?;
    for t in &comm.tables {
        let mut te = proto_table.clone();
        set_var_in(&mut te, None, nm.guid, Value::Guid(t.guid));
        let td = child_block_mut(&mut te, nm.table_data).expect("checked above");
        set_var_in(td, None, nm.num_phases, Value::U32(t.phases.len() as u32));
        let tk = kids_mut(td)?;
        tk.retain(|k| !matches!(&k.kind, Kind::Block(b) if b.name == nm.phase_entry));
        for ph in &t.phases {
            let mut pe = proto_phase.clone();
            set_var_in(&mut pe, None, nm.guid, Value::Guid(ph.guid));
            set_var_in(&mut pe, Some(nm.rt), nm.killed, Value::I32(0));
            tk.push(pe);
        }
        kids.push(te);
    }
    Ok(c)
}

/// A `c` whose first `phaseRtData` is pristine (nothing killed, no spawn delay): the layout source.
fn proto_c(doc: &Document, communities: &Node) -> Option<Node> {
    let pristine = |c: &Node| -> Option<bool> {
        let data = c.children().into_iter().find(|k| block_name(doc, k) == Some("communityData"))?;
        let te = data.children().into_iter().find(|k| block_name(doc, k) == Some("tableEntry"))?;
        let td = te.children().into_iter().find(|k| block_name(doc, k) == Some("tableEntryData"))?;
        let pe = td.children().into_iter().find(|k| block_name(doc, k) == Some("phaseEntry"))?;
        let rt = pe.children().into_iter().find(|k| block_name(doc, k) == Some("phaseRtData"))?;
        let killed = matches!(var_value(doc, rt, "killedNPCsNum"), Some(Value::I32(0)));
        let delay = var_value(doc, rt, "spawnDelay").map(|v| value_text(doc, v)).unwrap_or_default();
        Some(killed && delay.bytes().all(|b| b == b'0'))
    };
    let cs: Vec<&Node> = communities.children().into_iter().filter(|k| block_name(doc, k) == Some("c")).collect();
    cs.iter().find(|c| pristine(c) == Some(true)).or(cs.first()).map(|c| (*c).clone())
}

/// Swap-remove every stub of `set` (normalized path) whose story entry `keep` rejects; returns the
/// idTags removed. Mirrors the game (gate G: the removed slot is taken by the former last stub).
fn remove_stubs(doc: &mut Document, set: &str, keep: &dyn Fn(&str) -> bool, rep: &mut CommunityReport) -> Result<Vec<String>, Error> {
    let victims: Vec<(usize, String)> = graph::stubs(doc)
        .into_iter()
        .filter(|s| normalize(&s.community) == set && !keep(&s.entries[1]))
        .filter_map(|s| s.id_tag.map(|t| (s.index, t)))
        .collect();
    if victims.is_empty() {
        return Ok(Vec::new());
    }
    let loc = graph::locate(doc, "community/stubs")?.ok_or_else(|| Error::Other("the save has no community/stubs block".into()))?;
    let a_idx = doc.name_index("a");
    let mut removed = Vec::new();
    // Highest index first: a swap-remove only moves the last stub, so lower indices stay valid.
    for (index, tag) in victims.into_iter().rev() {
        let stubs = crate::synth::node_at_mut(doc, &loc);
        let kids = kids_mut(stubs)?;
        let positions: Vec<usize> = kids.iter().enumerate().filter(|(_, k)| matches!(&k.kind, Kind::Block(b) if Some(b.name) == a_idx)).map(|(i, _)| i).collect();
        let (Some(&pos), Some(&last)) = (positions.get(index), positions.last()) else { continue };
        if pos != last {
            kids.swap(pos, last);
        }
        kids.remove(last);
        rep.stubs_removed.push((tag.clone(), set.to_owned()));
        removed.push(tag);
    }
    Ok(removed)
}

/// Drop `layerStorage > entityData` entries carrying these idTags (every world), recounting `numEntities`.
fn remove_entity_data(doc: &mut Document, tags: &[String], rep: &mut CommunityReport) -> Result<(), Error> {
    if tags.is_empty() {
        return Ok(());
    }
    let Some(top) = doc.top_nodes().position(|n| block_name(doc, n) == Some("universe")) else { return Ok(()) };
    let (world_idx, ls_idx, ed_idx, num_idx) = (doc.name_index("world"), doc.name_index("layerStorage"), doc.name_index("entityData"), doc.name_index("numEntities"));
    let is = |n: &Node, idx: Option<u16>| matches!(&n.kind, Kind::Block(b) if Some(b.name) == idx);
    // Collect `(world child index, layerStorage child index, entityData child index, tag)` first.
    let mut hits: Vec<(usize, usize, usize, String)> = Vec::new();
    {
        let universe = doc.top_nodes().nth(top).expect("universe");
        for (wi, w) in universe.children().into_iter().enumerate().filter(|(_, w)| is(w, world_idx)) {
            for (li, ls) in w.children().into_iter().enumerate().filter(|(_, l)| is(l, ls_idx)) {
                for (ei, ed) in ls.children().into_iter().enumerate().filter(|(_, e)| is(e, ed_idx)) {
                    let tag = var_value(doc, ed, "idTag").map(|v| value_text(doc, v)).unwrap_or_default();
                    if tags.contains(&tag) {
                        hits.push((wi, li, ei, tag));
                    }
                }
            }
        }
    }
    hits.sort_by(|a, b| b.cmp(a)); // remove from the back so indices stay valid
    for (wi, li, ei, tag) in hits {
        let universe = doc.top_nodes_mut().nth(top).expect("universe");
        let w = kids_mut(universe)?.get_mut(wi).ok_or_else(|| Error::Other("world vanished".into()))?;
        let ls = kids_mut(w)?.get_mut(li).ok_or_else(|| Error::Other("layerStorage vanished".into()))?;
        let kids = kids_mut(ls)?;
        kids.remove(ei);
        let count = kids.iter().filter(|k| is(k, ed_idx)).count() as u32;
        for k in kids.iter_mut() {
            if let Kind::Var(v) = &mut k.kind {
                if Some(v.name) == num_idx {
                    v.value = Value::U32(count);
                }
            }
        }
        rep.entities_removed.push(tag);
    }
    Ok(())
}

/// Apply spawn-set actions in order. Counts (`numCommunities`, `numStubs`) are recomputed here.
pub fn apply_spawn(doc: &mut Document, lib: &Library, actions: &[SpawnAction], rep: &mut CommunityReport) -> Result<(), Error> {
    if actions.is_empty() {
        return Ok(());
    }
    if !lib.has_resources(".w2comm") {
        rep.notes.push("no spawn-set files loaded (use --install or a folder with .w2comm): community not written".into());
        return Ok(());
    }
    let nm = Nm::intern(doc)?;
    let mut dropped: Vec<String> = Vec::new();
    for a in actions {
        // A setter whose spawn-set import did not resolve (Letho's End phase has one): nothing to address.
        if matches!(a, SpawnAction::Activate { spawnset, .. } | SpawnAction::Deactivate { spawnset } if spawnset.is_empty()) {
            rep.notes.push(format!("{a:?}: empty spawn set, skipped"));
            continue;
        }
        match a {
            SpawnAction::Activate { spawnset, phase } => {
                let comm = match lib.community(spawnset) {
                    Ok(c) => c,
                    Err(e) => {
                        rep.notes.push(format!("{spawnset}: not read ({e}); phase {phase:?} not written"));
                        continue;
                    }
                };
                // The unnamed default phase is CName 0 (`activePhase = ''` in the saves).
                let phase_idx = if phase.is_empty() { 0 } else { doc.intern_name(phase)? };
                let existing: Option<(usize, String)> = {
                    let cs = communities(doc)?;
                    cs.children().into_iter().enumerate().find(|(_, c)| c_path(doc, c) == *spawnset).map(|(i, c)| {
                        let old = child_block(c, nm.data).and_then(|d| var_value(doc, d, "activePhase")).map(|v| value_text(doc, v)).unwrap_or_default();
                        (i, old.trim_matches('\'').to_owned())
                    })
                };
                match existing {
                    Some((i, old)) => {
                        if old == *phase {
                            continue;
                        }
                        let c = communities_kids_mut(doc)?.get_mut(i).expect("found above");
                        if !set_var_in(c, Some(nm.data), nm.is_active, Value::Bool(true)) || !set_var_in(c, Some(nm.data), nm.active_phase, Value::CName(phase_idx)) {
                            return Err(Error::Other(format!("{spawnset}: c block without communityData/isActive/activePhase")));
                        }
                        rep.phases_set.push((spawnset.clone(), old, phase.clone()));
                        // Stubs of the phase switched away from leave; the game spawns the new phase's.
                        let keep = |entry: &str| parse_guid(entry).ok().and_then(|g| comm.phase_of(&g).map(|p| p == phase)).unwrap_or(true);
                        dropped.extend(remove_stubs(doc, spawnset, &keep, rep)?);
                    }
                    None => {
                        let node = {
                            let cs = communities(doc)?;
                            let proto = proto_c(doc, cs).ok_or_else(|| Error::Other("the save has no community c block to copy the layout from".into()))?;
                            new_c(&nm, &proto, spawnset, phase_idx, &comm)?
                        };
                        communities_kids_mut(doc)?.push(node);
                        rep.sets_added.push(spawnset.clone());
                    }
                }
            }
            SpawnAction::Deactivate { spawnset } => {
                let pos = communities(doc)?.children().into_iter().position(|c| c_path(doc, c) == *spawnset);
                if let Some(i) = pos {
                    communities_kids_mut(doc)?.remove(i);
                    rep.sets_removed.push(spawnset.clone());
                }
                dropped.extend(remove_stubs(doc, spawnset, &|_| false, rep)?);
            }
            SpawnAction::Other(what) => rep.notes.push(format!("spawn action {what} not modelled")),
        }
    }
    remove_entity_data(doc, &dropped, rep)?;
    crate::synth::recount_one(doc, "community/communities", "numCommunities", "c")?;
    crate::synth::recount_one(doc, "community/stubs", "numStubs", "a")?;
    Ok(())
}

/// The spawn-set state of a save, for comparisons: `(set, active phase)` and the stubs `(idTag, set, story entry)`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommunityState {
    pub sets: Vec<(String, String)>,
    pub stubs: Vec<(String, String, String)>,
}

pub fn state(doc: &Document) -> CommunityState {
    let mut st = CommunityState::default();
    if let Ok(cs) = communities(doc) {
        for c in cs.children().into_iter().filter(|k| block_name(doc, k) == Some("c")) {
            let data = c.children().into_iter().find(|k| block_name(doc, k) == Some("communityData"));
            let phase = data.and_then(|d| var_value(doc, d, "activePhase")).map(|v| value_text(doc, v)).unwrap_or_default();
            st.sets.push((c_path(doc, c), phase.trim_matches('\'').to_owned()));
        }
    }
    for s in graph::stubs(doc) {
        st.stubs.push((s.id_tag.unwrap_or_default(), normalize(&s.community), s.entries[1].clone()));
    }
    st
}

/// Differences between two saves' spawn-set state, restricted to `sets` (normalized paths; empty =
/// all): one line each. Timers, positions and action points are not compared.
pub fn compare(a: &Document, b: &Document, sets: &[String]) -> Vec<String> {
    let (sa, sb) = (state(a), state(b));
    let inside = |s: &str| sets.is_empty() || sets.iter().any(|x| x == s);
    let short = |e: &str| e[..8.min(e.len())].to_owned();
    let mut lines = Vec::new();
    for (set, phase) in sa.sets.iter().filter(|(s, _)| inside(s)) {
        match sb.sets.iter().find(|(s, _)| s == set) {
            None => lines.push(format!("set removed in B: {set} (phase {phase})")),
            Some((_, pb)) if pb != phase => lines.push(format!("phase differs: {set} A {phase} B {pb}")),
            _ => {}
        }
    }
    for (set, phase) in sb.sets.iter().filter(|(s, _)| inside(s)) {
        if !sa.sets.iter().any(|(s, _)| s == set) {
            lines.push(format!("set only in B: {set} (phase {phase})"));
        }
    }
    for (tag, set, entry) in sa.stubs.iter().filter(|(_, s, _)| inside(s)) {
        match sb.stubs.iter().find(|(t, _, _)| t == tag) {
            None => lines.push(format!("stub only in A: {set} {tag} entry {}", short(entry))),
            Some((_, _, eb)) if eb != entry => lines.push(format!("stub entry differs: {set} {tag} A {} B {}", short(entry), short(eb))),
            _ => {}
        }
    }
    for (tag, set, entry) in sb.stubs.iter().filter(|(_, s, _)| inside(s)) {
        if !sa.stubs.iter().any(|(t, _, _)| t == tag) {
            lines.push(format!("stub only in B: {set} {tag} entry {}", short(entry)));
        }
    }
    lines
}

/// The spawn sets a walk touches (normalized paths), for [`compare`].
pub fn touched_sets(actions: &[SpawnAction]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for a in actions {
        if let SpawnAction::Activate { spawnset, .. } | SpawnAction::Deactivate { spawnset } = a {
            if !v.contains(spawnset) {
                v.push(spawnset.clone());
            }
        }
    }
    v
}
