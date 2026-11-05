//! The quest log: `CJournalManager > JActiveEntries` plus the side tables of
//! `CWitcherJournalManager` (objective counters, tracked quest, highlighted objective).
//!
//! Layout, from the typed trees of 45 consecutive lukwraith saves and the later-game sets
//! (docs/findings/quest-state.md; every shape below is checked when reading, and anything else is
//! an error, never a guess):
//!
//! ```text
//! BS CJournalManager
//!   BS JActiveEntries
//!     VL Size:Uint32                    == number of entries (all saves seen)
//!     N x BS SJournalEntryStatus
//!           BS path                     1..5 x (VL guid:CGUID, VL resource:String, VL flags:Uint8)
//!           VL status:EJournalStatus    u16 NM index of JS_Inactive / JS_Active / JS_Success / JS_Failed
//!           VL JEntryUnread:Bool
//! BS CWitcherJournalManager
//!   BS JObjectiveCounters               VL Size:Uint32, N x BS JObjectiveCounter {VL guid:CGUID, VL count:Int32}
//!   BS JTrackedQuest                    VL guid:CGUID
//!   BS JHighlightedObjective            VL guid:CGUID
//!   BS JHuntingClues, JMonsterKnown, JEntryAdvancedInfo   (not touched)
//! ```
//!
//! An entry is identified by the GUIDs of its path (the [`Entry::key`]); the resource strings only
//! name the `.journal` file a step comes from (empty when it is the same file as the parent).
//!
//! What is PROVEN offline: the layout above (strict read on every fixture), that the game only
//! appends entries (44 of 44 consecutive pairs), and that [`apply`] of the 21 -> 22 lukwraith diff
//! reproduces the game's own `CJournalManager` and `CWitcherJournalManager` bytes
//! (`tests/journal_fixtures.rs`). What is NOT proven: that the game shows a quest as completed
//! after such an edit, and that a stale quest graph (`questSystem`) does not re-run the quest.
//! Those are the in-game gate J (docs/findings/journal-editor-design.md). `Document::apply` keeps
//! refusing journal edits until then; this module is reachable through `w3al journal-set` only.
use std::collections::{HashMap, VecDeque};

use crate::document::Document;
use crate::error::{Error, Result};
use crate::token::{Block, Kind, Node, Value, Var, VarTag};

pub type Guid = [u8; 16];

/// `EJournalStatus` value names seen in saves. `JS_Failed` exists only in later-game name tables.
pub const STATUSES: [&str; 4] = ["JS_Inactive", "JS_Active", "JS_Success", "JS_Failed"];

/// Longest entry path seen: 4 steps in early-game saves, 5 in late-game ones (hosamnasr, imz).
pub const MAX_PATH_STEPS: usize = 5;

/// Lower-case hex of the 16 bytes in file order (the form used in docs/findings).
pub fn guid_hex(g: &Guid) -> String {
    g.iter().map(|b| format!("{b:02x}")).collect()
}

/// 32 hex digits, file byte order.
pub fn parse_guid(s: &str) -> Result<Guid> {
    let s = s.trim();
    if s.len() != 32 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Payload(format!("{s:?} is not a GUID (32 hex digits)")));
    }
    let mut g = [0u8; 16];
    for (i, b) in g.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| Error::Payload(format!("bad GUID {s:?}")))?;
    }
    Ok(g)
}

/// One element of an entry path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub guid: Guid,
    /// `.journal` resource; empty when the step lives in the same file as its parent.
    pub resource: String,
    /// 0 on every step but the last, 1 on the last, on every entry seen (checked by the fixture test).
    pub flags: u8,
}

/// One `SJournalEntryStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: Vec<Step>,
    /// NM name of the status (`JS_Active`, ...).
    pub status: String,
    pub unread: bool,
}

impl Entry {
    /// The path GUIDs: what identifies an entry.
    pub fn key(&self) -> Vec<Guid> {
        self.path.iter().map(|s| s.guid).collect()
    }

    /// Short readable path: file names for steps that name a resource, else the first 8 hex digits.
    pub fn label(&self) -> String {
        self.path
            .iter()
            .map(|s| {
                if s.resource.is_empty() {
                    guid_hex(&s.guid)[..8].to_string()
                } else {
                    let f = s.resource.rsplit(['\\', '/']).next().unwrap_or(&s.resource);
                    f.strip_suffix(".journal").unwrap_or(f).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Path of full GUIDs joined by `/` (what `w3al journal-set --set` accepts).
    pub fn key_hex(&self) -> String {
        key_hex(&self.key())
    }
}

pub fn key_hex(key: &[Guid]) -> String {
    key.iter().map(guid_hex).collect::<Vec<_>>().join("/")
}

/// `JObjectiveCounter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counter {
    pub guid: Guid,
    pub count: i32,
}

/// Set a counter's count, or add the counter when the template lacks it.
///
/// The game keeps counters in hash order, not insertion order: in lukwraith 21 -> 22 the new
/// counter appeared at position 27 of 45 and the other 44 kept their relative order. `at` inserts a
/// new counter at that position (taken from a donor save, so only meaningful for a template whose
/// counters are the donor's minus the new ones); `None` appends at the end. HYPOTHESIS: the game
/// accepts any order (it accepted facts appended out of hash order, gate A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterEdit {
    pub guid: Guid,
    pub count: i32,
    pub at: Option<usize>,
}

/// The decoded quest log of a save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    /// `JActiveEntries > Size`. Equals `entries.len()` on every save seen; edits refuse otherwise.
    pub size: u32,
    pub entries: Vec<Entry>,
    /// `JObjectiveCounters > Size` (`None` when the save has no `CWitcherJournalManager`).
    pub counters_size: Option<u32>,
    pub counters: Vec<Counter>,
    pub tracked_quest: Option<Guid>,
    pub highlighted_objective: Option<Guid>,
}

impl Journal {
    /// Positions of the entries with this key (normally zero or one).
    pub fn find(&self, key: &[Guid]) -> Vec<usize> {
        self.entries.iter().enumerate().filter(|(_, e)| e.key() == key).map(|(i, _)| i).collect()
    }

    /// Entries whose key equals another entry's key (none on the saves seen).
    pub fn duplicate_keys(&self) -> usize {
        let mut seen = HashMap::new();
        for e in &self.entries {
            *seen.entry(e.key()).or_insert(0usize) += 1;
        }
        seen.values().filter(|&&n| n > 1).count()
    }
}

fn bad(why: impl Into<String>) -> Error {
    Error::Unverified(format!("journal layout: {}", why.into()))
}

fn block_name<'a>(doc: &'a Document, n: &Node) -> Option<&'a str> {
    match &n.kind {
        Kind::Block(b) => doc.name(b.name),
        _ => None,
    }
}

fn top_block<'a>(doc: &'a Document, name: &str) -> Result<Option<&'a Node>> {
    let mut found = doc.top_nodes().filter(|n| block_name(doc, n) == Some(name));
    let first = found.next();
    if found.next().is_some() {
        return Err(bad(format!("more than one top-level {name}")));
    }
    Ok(first)
}

fn children(n: &Node) -> &[Node] {
    match &n.kind {
        Kind::Block(b) => &b.children,
        _ => &[],
    }
}

fn child_block<'a>(doc: &Document, n: &'a Node, name: &str) -> Option<&'a Node> {
    children(n).iter().find(|c| block_name(doc, c) == Some(name))
}

/// The value of a `VL name:ty` node, or an error naming what was expected.
fn var<'a>(doc: &Document, n: &'a Node, name: &str, ty: &str) -> Result<&'a Value> {
    match &n.kind {
        Kind::Var(Var { tag: VarTag::Vl, name: nm, ty: t, value })
            if doc.name(*nm) == Some(name) && doc.name(*t) == Some(ty) =>
        {
            Ok(value)
        }
        _ => Err(bad(format!("expected VL {name}:{ty}"))),
    }
}

fn parse_entry(doc: &Document, n: &Node) -> Result<Entry> {
    if block_name(doc, n) != Some("SJournalEntryStatus") {
        return Err(bad("JActiveEntries child is not SJournalEntryStatus"));
    }
    let [path, status, unread] = children(n) else {
        return Err(bad("SJournalEntryStatus does not have exactly path, status, JEntryUnread"));
    };
    if block_name(doc, path) != Some("path") {
        return Err(bad("first SJournalEntryStatus child is not path"));
    }
    let steps = children(path);
    if steps.is_empty() || steps.len() % 3 != 0 {
        return Err(bad(format!("path has {} children, not 3 per step", steps.len())));
    }
    let mut out = Vec::with_capacity(steps.len() / 3);
    for s in steps.chunks(3) {
        let Value::Guid(guid) = var(doc, &s[0], "guid", "CGUID")? else { return Err(bad("guid value not decoded")) };
        let Value::Str(resource) = var(doc, &s[1], "resource", "String")? else {
            return Err(bad("resource value not decoded"));
        };
        let Value::U8(flags) = var(doc, &s[2], "flags", "Uint8")? else { return Err(bad("flags value not decoded")) };
        out.push(Step { guid: *guid, resource: resource.clone(), flags: *flags });
    }
    let Value::Enum(st) = var(doc, status, "status", "EJournalStatus")? else {
        return Err(bad("status value not decoded"));
    };
    let status = doc.name(*st).ok_or_else(|| bad("status name index outside the NM table"))?.to_string();
    let Value::Bool(unread) = var(doc, unread, "JEntryUnread", "Bool")? else {
        return Err(bad("JEntryUnread value not decoded"));
    };
    Ok(Entry { path: out, status, unread: *unread })
}

/// `VL Size:Uint32` followed by blocks: (size, blocks).
fn sized_list<'a>(doc: &Document, n: &'a Node, what: &str) -> Result<(u32, &'a [Node])> {
    let kids = children(n);
    let Some(first) = kids.first() else { return Err(bad(format!("{what} is empty"))) };
    let Value::U32(size) = var(doc, first, "Size", "Uint32")? else { return Err(bad(format!("{what} Size not decoded"))) };
    Ok((*size, &kids[1..]))
}

fn guid_child(doc: &Document, n: &Node) -> Result<Option<Guid>> {
    match children(n) {
        [] => Ok(None),
        [g] => match var(doc, g, "guid", "CGUID")? {
            Value::Guid(g) => Ok(Some(*g)),
            _ => Err(bad("guid value not decoded")),
        },
        _ => Err(bad("tracking block with more than one child")),
    }
}

/// Read the quest log. Strict: any shape other than the documented one is an error.
pub fn read(doc: &Document) -> Result<Journal> {
    let jm = top_block(doc, "CJournalManager")?.ok_or_else(|| bad("no CJournalManager"))?;
    let [active] = children(jm) else { return Err(bad("CJournalManager does not hold exactly JActiveEntries")) };
    if block_name(doc, active) != Some("JActiveEntries") {
        return Err(bad("CJournalManager child is not JActiveEntries"));
    }
    let (size, list) = sized_list(doc, active, "JActiveEntries")?;
    let entries = list.iter().map(|n| parse_entry(doc, n)).collect::<Result<Vec<_>>>()?;

    let mut j = Journal { size, entries, counters_size: None, counters: Vec::new(), tracked_quest: None, highlighted_objective: None };
    let Some(wm) = top_block(doc, "CWitcherJournalManager")? else { return Ok(j) };
    if let Some(oc) = child_block(doc, wm, "JObjectiveCounters") {
        let (size, list) = sized_list(doc, oc, "JObjectiveCounters")?;
        j.counters_size = Some(size);
        for c in list {
            if block_name(doc, c) != Some("JObjectiveCounter") {
                return Err(bad("JObjectiveCounters child is not JObjectiveCounter"));
            }
            let [g, n] = children(c) else { return Err(bad("JObjectiveCounter is not guid + count")) };
            let (Value::Guid(guid), Value::I32(count)) = (var(doc, g, "guid", "CGUID")?, var(doc, n, "count", "Int32")?) else {
                return Err(bad("JObjectiveCounter values not decoded"));
            };
            j.counters.push(Counter { guid: *guid, count: *count });
        }
    }
    if let Some(t) = child_block(doc, wm, "JTrackedQuest") {
        j.tracked_quest = guid_child(doc, t)?;
    }
    if let Some(h) = child_block(doc, wm, "JHighlightedObjective") {
        j.highlighted_objective = guid_child(doc, h)?;
    }
    Ok(j)
}

// ---------------------------------------------------------------------------------------------
// Diff

/// An entry present in both saves whose status, unread flag or step details differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryChange {
    /// Position in A and in B.
    pub a: usize,
    pub b: usize,
    pub before: Entry,
    pub after: Entry,
}

impl EntryChange {
    /// Same GUIDs but other resource strings or flags: not something [`apply`] reproduces.
    pub fn path_changed(&self) -> bool {
        self.before.path != self.after.path
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalDiff {
    pub size: (u32, u32),
    pub changed: Vec<EntryChange>,
    /// Entries only in B, with their position in B.
    pub added: Vec<(usize, Entry)>,
    /// Entries only in A, with their position in A (the game never removed one in 44 pairs).
    pub removed: Vec<(usize, Entry)>,
    /// A's entries are B's first entries in the same order, so appending `added` to A gives B's order.
    pub append_only: bool,
    pub counters_changed: Vec<(Guid, i32, i32)>,
    /// Counters only in B, with their position in B.
    pub counters_added: Vec<(usize, Counter)>,
    pub counters_removed: Vec<Counter>,
    /// A's counters appear in B in the same relative order (new ones may sit anywhere: hash order).
    pub counters_order_kept: bool,
    pub tracked_quest: Option<(Option<Guid>, Option<Guid>)>,
    pub highlighted_objective: Option<(Option<Guid>, Option<Guid>)>,
}

impl JournalDiff {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.counters_changed.is_empty()
            && self.counters_added.is_empty()
            && self.counters_removed.is_empty()
            && self.tracked_quest.is_none()
            && self.highlighted_objective.is_none()
    }

    /// The edit that turns A into B, when [`apply`] can express it: no removals, no path changes.
    pub fn to_edit(&self) -> Result<JournalEdit> {
        if !self.removed.is_empty() || !self.counters_removed.is_empty() {
            return Err(Error::Unverified("journal diff removes entries or counters; removal is not supported".into()));
        }
        if self.changed.iter().any(EntryChange::path_changed) {
            return Err(Error::Unverified("journal diff changes path resources or flags; not supported".into()));
        }
        let status = self
            .changed
            .iter()
            .map(|c| StatusChange {
                key: c.after.key(),
                from: Some(c.before.status.clone()),
                to: c.after.status.clone(),
                unread: (c.before.unread != c.after.unread).then_some(c.after.unread),
            })
            .collect();
        let mut counters: Vec<CounterEdit> =
            self.counters_changed.iter().map(|&(guid, _, count)| CounterEdit { guid, count, at: None }).collect();
        let keep_pos = self.counters_order_kept;
        counters.extend(self.counters_added.iter().map(|&(j, c)| CounterEdit { guid: c.guid, count: c.count, at: keep_pos.then_some(j) }));
        Ok(JournalEdit {
            status,
            append: self.added.iter().map(|(_, e)| e.clone()).collect(),
            counters,
            tracked_quest: self.tracked_quest.and_then(|t| t.1),
            highlighted_objective: self.highlighted_objective.and_then(|t| t.1),
            allow_new_names: false,
        })
    }
}

/// Compare two quest logs. Entries are matched by key, in order of occurrence.
pub fn diff(a: &Journal, b: &Journal) -> JournalDiff {
    let mut d = JournalDiff { size: (a.size, b.size), ..Default::default() };
    let mut pos_a: HashMap<Vec<Guid>, VecDeque<usize>> = HashMap::new();
    for (i, e) in a.entries.iter().enumerate() {
        pos_a.entry(e.key()).or_default().push_back(i);
    }
    let mut matched = Vec::new();
    for (j, e) in b.entries.iter().enumerate() {
        match pos_a.get_mut(&e.key()).and_then(VecDeque::pop_front) {
            Some(i) => {
                matched.push((i, j));
                if a.entries[i] != *e {
                    d.changed.push(EntryChange { a: i, b: j, before: a.entries[i].clone(), after: e.clone() });
                }
            }
            None => d.added.push((j, e.clone())),
        }
    }
    let mut left: Vec<usize> = pos_a.into_values().flatten().collect();
    left.sort_unstable();
    d.removed = left.into_iter().map(|i| (i, a.entries[i].clone())).collect();
    d.append_only = d.removed.is_empty() && matched.iter().enumerate().all(|(k, &(i, j))| i == k && j == k);

    let ca: HashMap<Guid, (usize, i32)> = a.counters.iter().enumerate().map(|(i, c)| (c.guid, (i, c.count))).collect();
    let mut kept = 0usize;
    let mut order_ok = true;
    for (j, c) in b.counters.iter().enumerate() {
        match ca.get(&c.guid) {
            Some(&(i, old)) => {
                order_ok &= i == kept;
                kept += 1;
                if old != c.count {
                    d.counters_changed.push((c.guid, old, c.count));
                }
            }
            None => d.counters_added.push((j, *c)),
        }
    }
    d.counters_removed = a.counters.iter().filter(|c| !b.counters.iter().any(|x| x.guid == c.guid)).copied().collect();
    d.counters_order_kept = order_ok && kept == a.counters.len() && d.counters_removed.is_empty();
    if a.tracked_quest != b.tracked_quest {
        d.tracked_quest = Some((a.tracked_quest, b.tracked_quest));
    }
    if a.highlighted_objective != b.highlighted_objective {
        d.highlighted_objective = Some((a.highlighted_objective, b.highlighted_objective));
    }
    d
}

// ---------------------------------------------------------------------------------------------
// Edit

/// Set the status of an existing entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusChange {
    pub key: Vec<Guid>,
    /// Expected current status. A template whose entry is neither `from` nor already `to` is refused.
    pub from: Option<String>,
    pub to: String,
    /// New `JEntryUnread`, or keep the template's.
    pub unread: Option<bool>,
}

/// A set of quest-log changes, applied all-or-nothing by [`apply`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalEdit {
    pub status: Vec<StatusChange>,
    /// "Ensure" semantics: a missing entry is appended at the end of `JActiveEntries` (and `Size` grows);
    /// an entry the template already has gets the given status and unread flag instead.
    pub append: Vec<Entry>,
    /// Set the count of an existing counter, or add a new counter (`Size` grows; see [`CounterEdit`]).
    pub counters: Vec<CounterEdit>,
    pub tracked_quest: Option<Guid>,
    pub highlighted_objective: Option<Guid>,
    /// Append a status name (`JS_Success`, `JS_Failed`) to the NM table when the template lacks it,
    /// as the game itself does between saves. Off by default: unproven in game.
    pub allow_new_names: bool,
}

impl JournalEdit {
    pub fn is_empty(&self) -> bool {
        self.status.is_empty()
            && self.append.is_empty()
            && self.counters.is_empty()
            && self.tracked_quest.is_none()
            && self.highlighted_objective.is_none()
    }
}

/// What [`apply`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalReport {
    pub status_set: usize,
    pub status_unchanged: usize,
    pub appended: usize,
    /// Appends whose entry the template already had (status/unread set instead).
    pub already_present: usize,
    pub counters_set: usize,
    pub counters_appended: usize,
    pub tracking_set: usize,
    pub size: (u32, u32),
    pub nodes: (usize, usize),
    /// NM names appended to the template's table (only with `allow_new_names`).
    pub names_added: Vec<String>,
}

/// NM indices of the schema names a new entry or counter needs.
struct Names {
    entry: u16,
    path: u16,
    guid: u16,
    cguid: u16,
    resource: u16,
    string: u16,
    flags: u16,
    uint8: u16,
    status: u16,
    ejs: u16,
    unread: u16,
    bool_: u16,
}

fn need(doc: &Document, name: &str) -> Result<u16> {
    doc.name_index(name).ok_or_else(|| {
        Error::Unverified(format!(
            "name {name:?} is not in the template's NM table; adding NM names is not implemented \
             (docs/findings/journal-editor-design.md, section d)"
        ))
    })
}

fn vl(name: u16, ty: u16, value: Value) -> Node {
    Node { index: 0, kind: Kind::Var(Var { tag: VarTag::Vl, name, ty, value }) }
}

fn bs(name: u16, children: Vec<Node>) -> Node {
    Node { index: 0, kind: Kind::Block(Block { name, children }) }
}

fn entry_node(n: &Names, e: &Entry, status: u16) -> Node {
    let mut steps = Vec::with_capacity(e.path.len() * 3);
    for s in &e.path {
        steps.push(vl(n.guid, n.cguid, Value::Guid(s.guid)));
        steps.push(vl(n.resource, n.string, Value::Str(s.resource.clone())));
        steps.push(vl(n.flags, n.uint8, Value::U8(s.flags)));
    }
    bs(
        n.entry,
        vec![bs(n.path, steps), vl(n.status, n.ejs, Value::Enum(status)), vl(n.unread, n.bool_, Value::Bool(e.unread))],
    )
}

fn status_index(doc: &mut Document, s: &str, allow_new: bool, added: &mut Vec<String>) -> Result<u16> {
    if !STATUSES.contains(&s) {
        return Err(Error::Payload(format!("unknown journal status {s:?} (expected one of {STATUSES:?})")));
    }
    match doc.name_index(s) {
        Some(i) => Ok(i),
        // HYPOTHESIS (see `Document::intern_name`): an appended status name is accepted by the game.
        None if allow_new => {
            added.push(s.to_owned());
            doc.intern_name(s)
        }
        None => need(doc, s),
    }
}

/// Top-level `CWitcherJournalManager` children that [`read`] does not model (hunting clues, known
/// monsters, advanced info) and whose bytes differ between two saves. A diff that changes these is
/// not fully reproduced by [`apply`].
pub fn unmodelled_changes(a: &Document, b: &Document) -> Vec<String> {
    let kids = |d: &Document| -> Vec<(String, Vec<u8>)> {
        top_block(d, "CWitcherJournalManager")
            .ok()
            .flatten()
            .map(|w| {
                children(w)
                    .iter()
                    .map(|c| (block_name(d, c).unwrap_or("?").to_string(), c.to_bytes()))
                    .filter(|(n, _)| !matches!(n.as_str(), "JObjectiveCounters" | "JTrackedQuest" | "JHighlightedObjective"))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (ka, kb) = (kids(a), kids(b));
    let mut out: Vec<String> = kb.iter().filter(|x| !ka.contains(x)).map(|x| x.0.clone()).collect();
    out.extend(ka.iter().filter(|x| !kb.iter().any(|y| y.0 == x.0)).map(|x| x.0.clone()));
    out.sort();
    out.dedup();
    out
}

fn top_block_mut(doc: &mut Document, name: u16) -> Option<&mut Vec<Node>> {
    doc.top_nodes_mut().find_map(|n| match &mut n.kind {
        Kind::Block(b) if b.name == name => Some(&mut b.children),
        _ => None,
    })
}

fn child_block_mut(kids: &mut [Node], name: u16) -> Option<&mut Vec<Node>> {
    kids.iter_mut().find_map(|n| match &mut n.kind {
        Kind::Block(b) if b.name == name => Some(&mut b.children),
        _ => None,
    })
}

fn set_var(n: &mut Node, v: Value) -> Result<()> {
    match &mut n.kind {
        Kind::Var(var) if std::mem::discriminant(&var.value) == std::mem::discriminant(&v) => {
            var.value = v;
            Ok(())
        }
        _ => Err(bad("value to change has an unexpected type")),
    }
}

fn check_entry_spec(e: &Entry) -> Result<()> {
    if e.path.is_empty() || e.path.len() > MAX_PATH_STEPS {
        return Err(Error::Payload(format!("entry {}: path must have 1..{MAX_PATH_STEPS} steps", e.key_hex())));
    }
    for (i, s) in e.path.iter().enumerate() {
        // Every entry seen has flags 0 on inner steps and 1 on the last (tests/journal_fixtures.rs).
        let want = u8::from(i + 1 == e.path.len());
        if s.flags != want {
            return Err(Error::Unverified(format!(
                "entry {}: step {i} has flags {} but every entry in real saves has {want} there",
                e.key_hex(),
                s.flags
            )));
        }
    }
    Ok(())
}

/// Apply `edit` to the document, all-or-nothing: the work happens on a copy that replaces `doc`
/// only after it was rebased and re-read and every requested change is visible.
///
/// HYPOTHESES (not proven in game, gate J): that the game shows these statuses as written, that
/// new entries may sit at the end of the list (the game appends there itself), and that the quest
/// graph does not need to agree. PROVEN offline: the bytes of a new entry/counter and of a status
/// flip equal the game's own (21 -> 22 fixture test), and the table renumbering is the game's
/// post-order rule.
pub fn apply(doc: &mut Document, edit: &JournalEdit) -> Result<JournalReport> {
    let mut work = doc.clone();
    work.rebase()?;
    let before = read(&work)?;
    if before.size as usize != before.entries.len() {
        return Err(bad(format!("JActiveEntries Size {} != {} entries", before.size, before.entries.len())));
    }
    if let Some(s) = before.counters_size {
        if s as usize != before.counters.len() {
            return Err(bad(format!("JObjectiveCounters Size {s} != {} counters", before.counters.len())));
        }
    }
    let structural = !edit.append.is_empty() || !edit.counters.is_empty();
    if structural && !work.indices_are_post_order() {
        return Err(Error::Unverified(
            "the template's table indices are not in post-order; adding nodes would need a numbering rule we have not seen".into(),
        ));
    }
    let mut report = JournalReport { nodes: (work.map.variables.len(), 0), ..Default::default() };

    let unique = |key: &[Guid]| -> Result<Option<usize>> {
        match before.find(key)[..] {
            [] => Ok(None),
            [i] => Ok(Some(i)),
            _ => Err(Error::Payload(format!("entry {} occurs more than once in the template", key_hex(key)))),
        }
    };

    // Resolve everything first (names, positions), then mutate.
    // (position, new status index, new unread)
    let mut sets: Vec<(usize, u16, Option<bool>)> = Vec::new();
    for c in &edit.status {
        let i = unique(&c.key)?.ok_or_else(|| {
            Error::Payload(format!(
                "the template has no journal entry {}; the quest was never started there (add it with an append)",
                key_hex(&c.key)
            ))
        })?;
        let cur = &before.entries[i].status;
        if let Some(from) = &c.from {
            if cur != from && *cur != c.to {
                return Err(Error::Payload(format!(
                    "entry {} is {cur} in the template, expected {from} (or already {})",
                    before.entries[i].label(),
                    c.to
                )));
            }
        }
        if *cur == c.to && c.unread.is_none_or(|u| u == before.entries[i].unread) {
            report.status_unchanged += 1;
        } else {
            report.status_set += 1;
        }
        sets.push((i, status_index(&mut work, &c.to, edit.allow_new_names, &mut report.names_added)?, c.unread));
    }
    let mut new_entries = Vec::new();
    let mut seen_new: Vec<Vec<Guid>> = Vec::new();
    for e in &edit.append {
        check_entry_spec(e)?;
        let st = status_index(&mut work, &e.status, edit.allow_new_names, &mut report.names_added)?;
        match unique(&e.key())? {
            Some(i) => {
                report.already_present += 1;
                sets.push((i, st, Some(e.unread)));
            }
            None => {
                if seen_new.contains(&e.key()) {
                    return Err(Error::Payload(format!("entry {} is appended twice", e.key_hex())));
                }
                seen_new.push(e.key());
                new_entries.push((e, st));
            }
        }
    }
    let names = if new_entries.is_empty() {
        None
    } else {
        Some(Names {
            entry: need(&work, "SJournalEntryStatus")?,
            path: need(&work, "path")?,
            guid: need(&work, "guid")?,
            cguid: need(&work, "CGUID")?,
            resource: need(&work, "resource")?,
            string: need(&work, "String")?,
            flags: need(&work, "flags")?,
            uint8: need(&work, "Uint8")?,
            status: need(&work, "status")?,
            ejs: need(&work, "EJournalStatus")?,
            unread: need(&work, "JEntryUnread")?,
            bool_: need(&work, "Bool")?,
        })
    };
    let mut counter_sets: Vec<(usize, i32)> = Vec::new();
    let mut counter_new: Vec<CounterEdit> = Vec::new();
    for c in &edit.counters {
        match before.counters.iter().position(|x| x.guid == c.guid) {
            Some(i) => counter_sets.push((i, c.count)),
            None if counter_new.iter().any(|x| x.guid == c.guid) => {
                return Err(Error::Payload(format!("counter {} is appended twice", guid_hex(&c.guid))))
            }
            None => counter_new.push(*c),
        }
    }
    // Positioned inserts go in ascending order so each lands at its final index; the rest go last.
    counter_new.sort_by_key(|c| c.at.unwrap_or(usize::MAX));
    if !edit.counters.is_empty() && before.counters_size.is_none() {
        return Err(bad("no JObjectiveCounters in the template"));
    }
    let counter_names = if counter_new.is_empty() {
        None
    } else {
        Some((need(&work, "JObjectiveCounter")?, need(&work, "guid")?, need(&work, "CGUID")?, need(&work, "count")?, need(&work, "Int32")?))
    };
    let ix = |doc: &Document, s: &str| need(doc, s);
    let (jm, jae, wjm, joc, jtq, jho) = (
        ix(&work, "CJournalManager")?,
        ix(&work, "JActiveEntries")?,
        work.name_index("CWitcherJournalManager"),
        work.name_index("JObjectiveCounters"),
        work.name_index("JTrackedQuest"),
        work.name_index("JHighlightedObjective"),
    );

    // Mutate: JActiveEntries.
    {
        let top = top_block_mut(&mut work, jm).ok_or_else(|| bad("no CJournalManager"))?;
        let list = child_block_mut(top, jae).ok_or_else(|| bad("no JActiveEntries"))?;
        for &(i, st, unread) in &sets {
            let Kind::Block(b) = &mut list[1 + i].kind else { return Err(bad("entry is not a block")) };
            set_var(&mut b.children[1], Value::Enum(st))?;
            if let Some(u) = unread {
                set_var(&mut b.children[2], Value::Bool(u))?;
            }
        }
        if let Some(n) = &names {
            for (e, st) in &new_entries {
                list.push(entry_node(n, e, *st));
            }
            report.appended = new_entries.len();
            let size = u32::try_from(list.len() - 1).map_err(|_| bad("too many entries"))?;
            set_var(&mut list[0], Value::U32(size))?;
        }
    }
    // Mutate: CWitcherJournalManager.
    if !edit.counters.is_empty() || edit.tracked_quest.is_some() || edit.highlighted_objective.is_some() {
        let wjm = wjm.ok_or_else(|| bad("no CWitcherJournalManager"))?;
        let top = top_block_mut(&mut work, wjm).ok_or_else(|| bad("no CWitcherJournalManager"))?;
        if !edit.counters.is_empty() {
            let list = joc.and_then(|n| child_block_mut(top, n)).ok_or_else(|| bad("no JObjectiveCounters"))?;
            for &(i, count) in &counter_sets {
                let Kind::Block(b) = &mut list[1 + i].kind else { return Err(bad("counter is not a block")) };
                set_var(&mut b.children[1], Value::I32(count))?;
            }
            report.counters_set = counter_sets.len();
            if let Some((cn, g, cg, cnt, i32t)) = counter_names {
                for c in &counter_new {
                    let node = bs(cn, vec![vl(g, cg, Value::Guid(c.guid)), vl(cnt, i32t, Value::I32(c.count))]);
                    // list[0] is Size, so counter position k is list index k + 1.
                    let at = c.at.map_or(list.len(), |k| (k + 1).min(list.len()));
                    list.insert(at, node);
                }
                report.counters_appended = counter_new.len();
                let size = u32::try_from(list.len() - 1).map_err(|_| bad("too many counters"))?;
                set_var(&mut list[0], Value::U32(size))?;
            }
        }
        for (want, block) in [(edit.tracked_quest, jtq), (edit.highlighted_objective, jho)] {
            let Some(g) = want else { continue };
            let kids = block.and_then(|n| child_block_mut(top, n)).ok_or_else(|| bad("no tracking block"))?;
            // HYPOTHESIS-free: only an existing guid is replaced; an empty tracking block is refused.
            let [node] = &mut kids[..] else { return Err(bad("tracking block does not hold exactly one guid")) };
            set_var(node, Value::Guid(g))?;
            report.tracking_set += 1;
        }
    }

    if structural {
        work.renumber_post_order();
    }
    work.rebase()?;
    report.nodes.1 = work.map.variables.len();

    // Re-read and check that every requested change is there and nothing else moved.
    let after = read(&work)?;
    let fail = |why: String| Err(Error::Payload(format!("journal edit did not verify: {why}")));
    if after.size as usize != after.entries.len() || after.entries.len() != before.entries.len() + report.appended {
        return fail(format!("Size {} for {} entries", after.size, after.entries.len()));
    }
    for &(i, st, unread) in &sets {
        let e = &after.entries[i];
        if work.name(st) != Some(e.status.as_str()) || unread.is_some_and(|u| u != e.unread) || e.key() != before.entries[i].key() {
            return fail(format!("entry {} not as requested", e.label()));
        }
    }
    for (k, (e, _)) in new_entries.iter().enumerate() {
        if after.entries[before.entries.len() + k] != **e {
            return fail(format!("appended entry {} reads back differently", e.label()));
        }
    }
    for (i, e) in before.entries.iter().enumerate() {
        if !sets.iter().any(|s| s.0 == i) && after.entries[i] != *e {
            return fail(format!("entry {} changed without being asked", e.label()));
        }
    }
    if after.counters_size.is_some_and(|s| s as usize != after.counters.len()) {
        return fail("counter Size".into());
    }
    if edit.tracked_quest.is_some_and(|g| after.tracked_quest != Some(g))
        || edit.highlighted_objective.is_some_and(|g| after.highlighted_objective != Some(g))
    {
        return fail("tracking guid".into());
    }
    for c in &edit.counters {
        if !after.counters.iter().any(|x| x.guid == c.guid && x.count == c.count) {
            return fail(format!("counter {}", guid_hex(&c.guid)));
        }
    }
    let old_order: Vec<Guid> = before.counters.iter().map(|c| c.guid).collect();
    let kept: Vec<Guid> = after.counters.iter().map(|c| c.guid).filter(|g| old_order.contains(g)).collect();
    if kept != old_order {
        return fail("existing counters changed order".into());
    }
    if work.table() != work.map.variables {
        return fail("variable table does not match the tree".into());
    }
    report.size = (before.size, after.size);
    *doc = work;
    Ok(report)
}

/// What [`remove`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoveReport {
    /// Entries removed (keys asked for that the save did not hold are skipped: already gone).
    pub entries_removed: usize,
    pub counters_removed: usize,
    pub size: (u32, u32),
    pub counters_size: (Option<u32>, Option<u32>),
}

/// Remove quest-log entries (by key) and objective counters (by GUID), all-or-nothing, the way the
/// game itself drops them: the `SJournalEntryStatus` / `JObjectiveCounter` node leaves its list, the
/// list's `Size` goes down by one per node and every other node keeps its relative order.
///
/// Evidence (docs/research.md, gate Y result and docs/plan/story-chain.md): the game's own re-saves of
/// gate Y2 dropped 1 and then 4 entries (`cginnkeepers/833d33e1/532a739f`, `sq202yen/ef808288/9a3dcf80`,
/// `maincharacters/yennefer/2486878c`, ...) and one counter that way and kept loading. HYPOTHESIS: an
/// entry removed by us is treated the same (the game never sees who removed it). A key or counter the
/// save lacks is skipped; a key the save holds twice is refused. The tracked quest and the highlighted
/// objective are not touched (a caller that removes the tracked entry should retarget them).
pub fn remove(doc: &mut Document, keys: &[Vec<Guid>], counters: &[Guid]) -> Result<RemoveReport> {
    let mut work = doc.clone();
    work.rebase()?;
    let before = read(&work)?;
    if before.size as usize != before.entries.len() {
        return Err(bad(format!("JActiveEntries Size {} != {} entries", before.size, before.entries.len())));
    }
    if before.counters_size.is_some_and(|s| s as usize != before.counters.len()) {
        return Err(bad("JObjectiveCounters Size does not match its counters"));
    }
    if !work.indices_are_post_order() {
        return Err(Error::Unverified(
            "the template's table indices are not in post-order; removing nodes would need a numbering rule we have not seen".into(),
        ));
    }
    let mut drop_entries: Vec<usize> = Vec::new();
    for k in keys {
        match before.find(k)[..] {
            [] => {}
            [i] => {
                if !drop_entries.contains(&i) {
                    drop_entries.push(i);
                }
            }
            _ => return Err(Error::Payload(format!("entry {} occurs more than once; not removed", key_hex(k)))),
        }
    }
    let mut drop_counters: Vec<usize> =
        counters.iter().filter_map(|g| before.counters.iter().position(|c| c.guid == *g)).collect();
    drop_counters.sort_unstable();
    drop_counters.dedup();
    drop_entries.sort_unstable();
    let mut report = RemoveReport { size: (before.size, before.size), counters_size: (before.counters_size, before.counters_size), ..Default::default() };
    if drop_entries.is_empty() && drop_counters.is_empty() {
        return Ok(report);
    }
    let jm = need(&work, "CJournalManager")?;
    let jae = need(&work, "JActiveEntries")?;
    {
        let top = top_block_mut(&mut work, jm).ok_or_else(|| bad("no CJournalManager"))?;
        let list = child_block_mut(top, jae).ok_or_else(|| bad("no JActiveEntries"))?;
        for &i in drop_entries.iter().rev() {
            list.remove(1 + i);
        }
        let size = u32::try_from(list.len() - 1).map_err(|_| bad("too many entries"))?;
        set_var(&mut list[0], Value::U32(size))?;
    }
    if !drop_counters.is_empty() {
        let wjm = need(&work, "CWitcherJournalManager")?;
        let joc = need(&work, "JObjectiveCounters")?;
        let top = top_block_mut(&mut work, wjm).ok_or_else(|| bad("no CWitcherJournalManager"))?;
        let list = child_block_mut(top, joc).ok_or_else(|| bad("no JObjectiveCounters"))?;
        for &i in drop_counters.iter().rev() {
            list.remove(1 + i);
        }
        let size = u32::try_from(list.len() - 1).map_err(|_| bad("too many counters"))?;
        set_var(&mut list[0], Value::U32(size))?;
    }
    work.renumber_post_order();
    work.rebase()?;
    let after = read(&work)?;
    let fail = |why: String| Err(Error::Payload(format!("journal removal did not verify: {why}")));
    let kept: Vec<&Entry> = before.entries.iter().enumerate().filter(|(i, _)| !drop_entries.contains(i)).map(|(_, e)| e).collect();
    if after.entries.iter().collect::<Vec<_>>() != kept || after.size as usize != after.entries.len() {
        return fail("entries other than the removed ones changed".into());
    }
    let kept_c: Vec<&Counter> = before.counters.iter().enumerate().filter(|(i, _)| !drop_counters.contains(i)).map(|(_, c)| c).collect();
    if after.counters.iter().collect::<Vec<_>>() != kept_c || after.counters_size.is_some_and(|s| s as usize != after.counters.len()) {
        return fail("counters other than the removed ones changed".into());
    }
    if after.tracked_quest != before.tracked_quest || after.highlighted_objective != before.highlighted_objective {
        return fail("tracking changed".into());
    }
    if work.table() != work.map.variables {
        return fail("variable table does not match the tree".into());
    }
    report.entries_removed = drop_entries.len();
    report.counters_removed = drop_counters.len();
    report.size.1 = after.size;
    report.counters_size.1 = after.counters_size;
    *doc = work;
    Ok(report)
}

/// Bytes of the top-level block with this NM name (for byte-exact comparisons between saves).
pub fn top_block_bytes(doc: &Document, name: &str) -> Option<Vec<u8>> {
    top_block(doc, name).ok().flatten().map(Node::to_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload;

    /// A synthetic payload with real-shaped journal blocks, numbered in post-order like real saves.
    fn journal_payload() -> Vec<u8> {
        let names = [
            "CJournalManager", "JActiveEntries", "Size", "Uint32", "SJournalEntryStatus", "path", "guid", "CGUID",
            "resource", "String", "flags", "Uint8", "status", "EJournalStatus", "JS_Active", "JS_Success",
            "JEntryUnread", "Bool", "CWitcherJournalManager", "JObjectiveCounters", "JObjectiveCounter", "count",
            "Int32", "JTrackedQuest", "JHighlightedObjective",
        ];
        let ix = |s: &str| names.iter().position(|n| *n == s).unwrap() as u16 + 1;
        let mut spans = Vec::<(u32, u32)>::new();
        let base = 3084u32;
        let mut p = b"SAV3".to_vec();
        for v in [67i32, 29, 164] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        // Build each node as bytes with a post-order list of spans.
        fn vlb(name: u16, ty: u16, val: &[u8]) -> Vec<u8> {
            let mut v = b"VL".to_vec();
            v.extend_from_slice(&name.to_le_bytes());
            v.extend_from_slice(&ty.to_le_bytes());
            v.extend_from_slice(val);
            v
        }
        // tree: (bytes of header, children) -> write recursively, recording spans post-order.
        enum T {
            B(u16, Vec<T>),
            V(Vec<u8>),
        }
        fn write(t: &T, p: &mut Vec<u8>, base: u32, spans: &mut Vec<(u32, u32)>) {
            let start = p.len();
            match t {
                T::B(n, kids) => {
                    p.extend_from_slice(b"BS");
                    p.extend_from_slice(&n.to_le_bytes());
                    for k in kids {
                        write(k, p, base, spans);
                    }
                }
                T::V(b) => p.extend_from_slice(b),
            }
            spans.push((base + start as u32, (p.len() - start) as u32));
        }
        let guid = |b: u8| [b; 16];
        let entry = |g: &[u8], st: &str, unread: bool| {
            let mut steps = Vec::new();
            for (k, &x) in g.iter().enumerate() {
                steps.push(T::V(vlb(ix("guid"), ix("CGUID"), &guid(x))));
                let res: &[u8] = if k == 0 { b"\x84a.jn" } else { b"\x80" };
                steps.push(T::V(vlb(ix("resource"), ix("String"), res)));
                steps.push(T::V(vlb(ix("flags"), ix("Uint8"), &[u8::from(k + 1 == g.len())])));
            }
            T::B(
                ix("SJournalEntryStatus"),
                vec![
                    T::B(ix("path"), steps),
                    T::V(vlb(ix("status"), ix("EJournalStatus"), &ix(st).to_le_bytes())),
                    T::V(vlb(ix("JEntryUnread"), ix("Bool"), &[unread as u8])),
                ],
            )
        };
        let jm = T::B(
            ix("CJournalManager"),
            vec![T::B(
                ix("JActiveEntries"),
                vec![
                    T::V(vlb(ix("Size"), ix("Uint32"), &2u32.to_le_bytes())),
                    entry(&[1], "JS_Active", false),
                    entry(&[1, 2], "JS_Active", true),
                ],
            )],
        );
        let wjm = T::B(
            ix("CWitcherJournalManager"),
            vec![
                T::B(
                    ix("JObjectiveCounters"),
                    vec![
                        T::V(vlb(ix("Size"), ix("Uint32"), &1u32.to_le_bytes())),
                        T::B(
                            ix("JObjectiveCounter"),
                            vec![T::V(vlb(ix("guid"), ix("CGUID"), &guid(2))), T::V(vlb(ix("count"), ix("Int32"), &0i32.to_le_bytes()))],
                        ),
                    ],
                ),
                T::B(ix("JTrackedQuest"), vec![T::V(vlb(ix("guid"), ix("CGUID"), &guid(1)))]),
                T::B(ix("JHighlightedObjective"), vec![T::V(vlb(ix("guid"), ix("CGUID"), &guid(2)))]),
            ],
        );
        let tops = [(ix("CJournalManager"), &jm), (ix("CWitcherJournalManager"), &wjm)];
        let mut rb = Vec::new();
        for (n, t) in tops {
            rb.push((n, base + p.len() as u32));
            write(t, &mut p, base, &mut spans);
        }
        // RB, NM, MANU, pointers, table, footer (as payload::testutil does).
        let rb_at = base + p.len() as u32;
        p.extend_from_slice(b"RB");
        p.extend_from_slice(&(rb.len() as u32).to_le_bytes());
        for (n, o) in &rb {
            p.extend_from_slice(&n.to_le_bytes());
            p.extend_from_slice(&o.to_le_bytes());
        }
        let nm_at = base + p.len() as u32;
        p.extend_from_slice(b"NM");
        let manu = base + p.len() as u32;
        p.extend_from_slice(b"MANU");
        p.extend_from_slice(&(names.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        for n in names {
            p.push(n.len() as u8);
            p.extend_from_slice(n.as_bytes());
        }
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(b"ENOD");
        spans.push((manu, base + p.len() as u32 - manu));
        p.extend_from_slice(&nm_at.to_le_bytes());
        p.extend_from_slice(&rb_at.to_le_bytes());
        p.extend_from_slice(b"SC");
        let vt = base + p.len() as u32;
        p.extend_from_slice(&(spans.len() as u32).to_le_bytes());
        for (o, s) in &spans {
            p.extend_from_slice(&o.to_le_bytes());
            p.extend_from_slice(&s.to_le_bytes());
        }
        p.extend_from_slice(&vt.to_le_bytes());
        p.extend_from_slice(b"SE");
        p
    }

    fn doc() -> (Vec<u8>, Document) {
        let p = journal_payload();
        let d = Document::parse(&p, 3084).unwrap();
        assert_eq!(d.to_bytes(), p);
        assert!(d.indices_are_post_order());
        (p, d)
    }

    #[test]
    fn reads_the_synthetic_journal() {
        let (_, d) = doc();
        let j = read(&d).unwrap();
        assert_eq!(j.size, 2);
        assert_eq!(j.entries.len(), 2);
        assert_eq!(j.entries[1].key(), vec![[1; 16], [2; 16]]);
        assert_eq!(j.entries[1].status, "JS_Active");
        assert!(j.entries[1].unread);
        assert_eq!(j.entries[1].label(), "a.jn/02020202");
        assert_eq!(j.counters, [Counter { guid: [2; 16], count: 0 }]);
        assert_eq!(j.tracked_quest, Some([1; 16]));
    }

    #[test]
    fn flip_is_two_bytes_and_append_grows_everything_consistently() {
        let (p, mut d) = doc();
        let mut e = JournalEdit::default();
        e.status.push(StatusChange { key: vec![[1; 16]], from: Some("JS_Active".into()), to: "JS_Success".into(), unread: None });
        apply(&mut d, &e).unwrap();
        let q = d.to_bytes();
        assert_eq!(q.len(), p.len());
        assert_eq!(q.iter().zip(&p).filter(|(a, b)| a != b).count(), 1, "one byte of the u16 index differs");
        assert_eq!(read(&d).unwrap().entries[0].status, "JS_Success");

        // Append an entry and a counter, retrack: table grows by the new nodes, everything re-parses.
        let new = Entry {
            path: vec![
                Step { guid: [1; 16], resource: "a.jn".into(), flags: 0 },
                Step { guid: [3; 16], resource: String::new(), flags: 1 },
            ],
            status: "JS_Success".into(),
            unread: true,
        };
        let vars = d.map.variables.len();
        let e = JournalEdit {
            append: vec![new.clone()],
            counters: vec![CounterEdit { guid: [3; 16], count: 0, at: Some(0) }],
            tracked_quest: Some([9; 16]),
            ..Default::default()
        };
        let r = apply(&mut d, &e).unwrap();
        assert_eq!((r.appended, r.counters_appended, r.tracking_set), (1, 1, 1));
        // entry = BS + path BS + 2x3 VL + 2 VL = 10 nodes; counter = BS + 2 VL = 3 nodes
        assert_eq!(d.map.variables.len(), vars + 13);
        let q = d.to_bytes();
        let again = Document::parse(&q, 3084).unwrap();
        assert_eq!(again.to_bytes(), q);
        assert_eq!(again.table(), payload::read_map(&q, 3084).unwrap().variables);
        assert!(again.indices_are_post_order());
        let j = read(&again).unwrap();
        assert_eq!((j.size, j.entries.len()), (3, 3));
        assert_eq!(j.entries[2], new);
        assert_eq!(j.counters_size, Some(2));
        assert_eq!(j.counters[0].guid, [3; 16], "inserted at the requested position");
        assert_eq!(j.tracked_quest, Some([9; 16]));

        // The same append again is "already present": no new nodes.
        let r = apply(&mut d, &JournalEdit { append: vec![new], ..Default::default() }).unwrap();
        assert_eq!((r.appended, r.already_present), (0, 1));
        assert_eq!(d.map.variables.len(), vars + 13);
    }

    #[test]
    fn refusals_leave_the_document_untouched() {
        let (p, mut d) = doc();
        let cases = [
            // unknown entry
            JournalEdit { status: vec![StatusChange { key: vec![[7; 16]], from: None, to: "JS_Success".into(), unread: None }], ..Default::default() },
            // status name not in the NM table
            JournalEdit { status: vec![StatusChange { key: vec![[1; 16]], from: None, to: "JS_Failed".into(), unread: None }], ..Default::default() },
            // unexpected current status
            JournalEdit { status: vec![StatusChange { key: vec![[1; 16]], from: Some("JS_Inactive".into()), to: "JS_Success".into(), unread: None }], ..Default::default() },
            // bad flags
            JournalEdit {
                append: vec![Entry { path: vec![Step { guid: [5; 16], resource: String::new(), flags: 0 }], status: "JS_Active".into(), unread: false }],
                ..Default::default()
            },
        ];
        for e in cases {
            assert!(apply(&mut d, &e).is_err(), "{e:?}");
            assert_eq!(d.to_bytes(), p);
        }
    }

    #[test]
    fn a_missing_status_name_is_appended_only_when_allowed() {
        let (p, mut d) = doc();
        let names_before = d.names.clone().unwrap();
        let e = JournalEdit {
            status: vec![StatusChange { key: vec![[1; 16]], from: None, to: "JS_Failed".into(), unread: None }],
            allow_new_names: true,
            ..Default::default()
        };
        let r = apply(&mut d, &e).unwrap();
        assert_eq!(r.names_added, ["JS_Failed"]);
        let q = d.to_bytes();
        assert_eq!(q.len(), p.len() + 1 + "JS_Failed".len());
        let again = Document::parse(&q, 3084).unwrap();
        let names = again.names.clone().unwrap();
        assert_eq!(&names[..names_before.len()], &names_before[..], "existing indices unchanged");
        assert_eq!(names.last().unwrap(), "JS_Failed");
        assert_eq!(again.table(), payload::read_map(&q, 3084).unwrap().variables);
        assert_eq!(read(&again).unwrap().entries[0].status, "JS_Failed");
    }

    #[test]
    fn diff_round_trips_through_apply() {
        let (_, a) = doc();
        let mut b = a.clone();
        let e = JournalEdit {
            status: vec![StatusChange { key: vec![[1; 16], [2; 16]], from: None, to: "JS_Success".into(), unread: Some(false) }],
            append: vec![Entry { path: vec![Step { guid: [4; 16], resource: "x".into(), flags: 1 }], status: "JS_Active".into(), unread: true }],
            counters: vec![CounterEdit { guid: [2; 16], count: 5, at: None }, CounterEdit { guid: [6; 16], count: 1, at: Some(0) }],
            highlighted_objective: Some([4; 16]),
            ..Default::default()
        };
        apply(&mut b, &e).unwrap();
        let (ja, jb) = (read(&a).unwrap(), read(&b).unwrap());
        let d = diff(&ja, &jb);
        assert!(d.append_only && d.counters_order_kept);
        assert_eq!((d.changed.len(), d.added.len(), d.counters_changed.len(), d.counters_added.len()), (1, 1, 1, 1));
        let mut c = a.clone();
        apply(&mut c, &d.to_edit().unwrap()).unwrap();
        assert_eq!(c.to_bytes(), b.to_bytes());
        assert!(diff(&jb, &read(&c).unwrap()).is_empty());
    }

    #[test]
    fn remove_drops_entries_and_counters_and_undoes_an_append() {
        let (p, a) = doc();
        // Append an entry and a counter, then remove them again: back to the template's bytes.
        let mut b = a.clone();
        let new = Entry { path: vec![Step { guid: [1; 16], resource: "x".into(), flags: 0 }, Step { guid: [7; 16], resource: String::new(), flags: 1 }], status: "JS_Active".into(), unread: true };
        apply(&mut b, &JournalEdit { append: vec![new.clone()], counters: vec![CounterEdit { guid: [7; 16], count: 0, at: None }], ..Default::default() }).unwrap();
        let r = remove(&mut b, &[new.key(), vec![[9; 16]]], &[[7; 16], [8; 16]]).unwrap();
        assert_eq!((r.entries_removed, r.counters_removed, r.size), (1, 1, (3, 2)));
        assert_eq!(b.to_bytes(), p, "removal is the exact inverse of the append");
        // Remove a template entry from the middle: the others keep their order, Size goes down.
        let mut c = a.clone();
        remove(&mut c, &[vec![[1; 16]]], &[[2; 16]]).unwrap();
        let j = read(&c).unwrap();
        assert_eq!((j.size, j.entries.len(), j.counters_size, j.counters.len()), (1, 1, Some(0), 0));
        assert_eq!(j.entries[0].key(), vec![[1; 16], [2; 16]]);
        assert_eq!(j.tracked_quest, Some([1; 16]), "tracking untouched");
        assert!(c.indices_are_post_order() && c.table() == c.map.variables);
        // Nothing to remove: unchanged.
        let mut d = a.clone();
        assert_eq!(remove(&mut d, &[vec![[5; 16]]], &[]).unwrap().entries_removed, 0);
        assert_eq!(d.to_bytes(), p);
    }

    #[test]
    fn guid_hex_round_trips() {
        let g = parse_guid("0f3305c02a250d4aa75c6fba97048763").unwrap();
        assert_eq!(g[0], 0x0f);
        assert_eq!(guid_hex(&g), "0f3305c02a250d4aa75c6fba97048763");
        assert!(parse_guid("0f33").is_err());
        assert!(parse_guid("zz3305c02a250d4aa75c6fba97048763").is_err());
    }
}
