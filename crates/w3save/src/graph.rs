//! Name-aware view, keyed tree diff and subtree transplant for whole top-level blocks
//! (`questSystem`, `community`, `gameProp`, `universe`, ...). Research tooling for the quest graph
//! (docs/findings/quest-graph-design.md); nothing here interprets what a block means.
//!
//! * [`render`] prints a subtree as one line per node, with NM names resolved, so two saves with
//!   different name indices can be compared as text.
//! * [`diff`] aligns the children of every `BS` block by a key (block name plus its identity
//!   value: the `GUID`/`guid` child of quest threads and blocks, the `fileName` of a quest, ...;
//!   see [`identity`]) and reports removed / added / changed nodes by path.
//! * [`transplant`] replaces subtrees of a template with the donor's, the way `journal::apply`
//!   replays a journal diff, and checks the result.
//!
//! Name indices are save-specific. [`NameMode::Prefix`] copies a donor subtree only when the
//! template's NM table is a prefix of the donor's (the game's own consecutive saves: 43 of 44
//! lukwraith pairs, see journal-editor-design.md) and appends the donor's extra names in order, so
//! every index inside the copied bytes (including any inside opaque values) means the same name.
//! [`NameMode::Remap`] translates every index of the copied subtree instead ([`remap_node`]),
//! including the ones inside the opaque values of `questSystem` and `community`, and refuses values
//! whose layout is not decoded (docs/findings/name-remap.md).
use std::collections::{BTreeMap, HashMap};

use crate::document::Document;
use crate::error::{Error, Result};
use crate::journal::guid_hex;
use crate::token::{fixed_len, Block, Kind, Node, Part, Raw, Sized, SsContent, Value, Var};

/// Identity children of a block, in priority order. A block keyed by one of these is matched
/// across saves by that value; other blocks are matched by name and occurrence.
/// From the typed trees of the lukwraith saves: `quest > fileName`, `questThread`/`questBlock > GUID`,
/// `ExternalDialog > guid`, `SJournalEntryStatus > path` (not used here), community `c > ...`.
const IDENTITY_VARS: [&str; 7] = ["fileName", "tag", "GUID", "guid", "name", "id", "p"];

fn nm(doc: &Document, i: u16) -> String {
    doc.name(i).map_or_else(|| format!("#{i}"), str::to_owned)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A short text for a decoded value (names resolved).
pub fn value_text(doc: &Document, v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::U8(x) => x.to_string(),
        Value::I8(x) => x.to_string(),
        Value::U16(x) => x.to_string(),
        Value::I16(x) => x.to_string(),
        Value::U32(x) => x.to_string(),
        Value::I32(x) => x.to_string(),
        Value::U64(x) => x.to_string(),
        Value::I64(x) => x.to_string(),
        Value::Float(b) => format!("{}", f32::from_bits(*b)),
        Value::Double(b) => format!("{}", f64::from_bits(*b)),
        Value::CName(0) => "''".into(),
        Value::CName(i) | Value::Enum(i) => nm(doc, *i),
        Value::Guid(g) => guid_hex(g),
        Value::Str(s) => format!("{s:?}"),
        Value::Opaque(parts) => {
            let mut s = String::new();
            for p in parts {
                if let Part::Bytes(b) = p {
                    s.push_str(&hex(b));
                } else {
                    s.push_str("<node>");
                }
            }
            if s.len() > 96 {
                format!("{}..({} hex)", &s[..96], s.len())
            } else {
                s
            }
        }
    }
}

/// The node's own name (`BS`/`VL`/`OP` name; tag for the rest).
pub fn node_name(doc: &Document, n: &Node) -> String {
    match &n.kind {
        Kind::Block(b) => nm(doc, b.name),
        Kind::Var(v) => nm(doc, v.name),
        _ => n.tag().to_owned(),
    }
}

/// One line for a node without its children: `BS name`, `VL name:Type = value`, `SS <n bytes>`.
pub fn node_line(doc: &Document, n: &Node) -> String {
    match &n.kind {
        Kind::Block(b) => format!("BS {}", nm(doc, b.name)),
        Kind::Var(v) => format!("{} {}:{} = {}", n.tag(), nm(doc, v.name), nm(doc, v.ty), value_text(doc, &v.value)),
        Kind::Sized(s) => format!("SS <{} bytes> {}", s.body_len(), short_hash(&s.body_bytes())),
        Kind::Manu(m) => format!("MANU <{} names>", m.names.len()),
        Kind::Raw(r) => format!("RAW {} <{} bytes>", r.tag, n.encoded_len()),
    }
}

/// FNV-1a of some bytes, for reports (not a security hash).
pub fn short_hash(b: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in b {
        h ^= u64::from(*x);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("#{:08x}", h as u32)
}

/// The identity value of a block: the first child var named in [`IDENTITY_VARS`] (by priority).
pub fn identity(doc: &Document, n: &Node) -> Option<String> {
    let Kind::Block(b) = &n.kind else { return None };
    for want in IDENTITY_VARS {
        for c in &b.children {
            if let Kind::Var(v) = &c.kind {
                if doc.name(v.name) == Some(want) {
                    let t = value_text(doc, &v.value);
                    // Shorten GUIDs and paths for keys; the full value stays in the rendered line.
                    let t = match &v.value {
                        Value::Guid(g) => guid_hex(g)[..8].to_owned(),
                        Value::Str(s) => s.rsplit(['\\', '/']).next().unwrap_or(s).to_owned(),
                        _ => t,
                    };
                    return Some(t);
                }
            }
        }
    }
    None
}

/// Path step of a node: `name` or `name[identity]`.
pub fn step(doc: &Document, n: &Node) -> String {
    match identity(doc, n) {
        Some(id) => format!("{}[{id}]", node_name(doc, n)),
        None => node_name(doc, n),
    }
}

/// Name-resolved serialization of a subtree (independent of NM indices): equal for two nodes
/// exactly when they render the same, including the bytes of opaque values and `SS` bodies.
pub fn fingerprint(doc: &Document, n: &Node) -> String {
    let mut s = String::new();
    fp(doc, n, &mut s);
    s
}

fn fp(doc: &Document, n: &Node, out: &mut String) {
    match &n.kind {
        Kind::Block(b) => {
            out.push_str("{B ");
            out.push_str(&nm(doc, b.name));
            for c in &b.children {
                out.push(' ');
                fp(doc, c, out);
            }
            out.push('}');
        }
        Kind::Var(v) => {
            out.push_str(&format!("{{{} {}:{} ", n.tag(), nm(doc, v.name), nm(doc, v.ty)));
            match &v.value {
                Value::Opaque(parts) => parts_fp(doc, parts, out),
                other => out.push_str(&value_text(doc, other)),
            }
            out.push('}');
        }
        Kind::Sized(s) => {
            out.push_str("{SS ");
            match &s.content {
                SsContent::Blob(parts) => parts_fp(doc, parts, out),
                SsContent::Facts(_) => out.push_str(&hex(&s.body_bytes())),
            }
            out.push('}');
        }
        _ => out.push_str(&hex(&n.to_bytes())),
    }
}

fn parts_fp(doc: &Document, parts: &[Part], out: &mut String) {
    for p in parts {
        match p {
            Part::Bytes(b) => out.push_str(&hex(b)),
            Part::Node(n) => fp(doc, n, out),
        }
    }
}

/// One rendered line: depth, path (steps joined by `/`), text.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub depth: usize,
    pub path: String,
    pub text: String,
}

/// Every node of a subtree, outermost first, file order.
pub fn render(doc: &Document, n: &Node) -> Vec<Line> {
    let mut out = Vec::new();
    render_into(doc, n, String::new(), 0, &mut out);
    out
}

fn render_into(doc: &Document, n: &Node, parent: String, depth: usize, out: &mut Vec<Line>) {
    let path = if parent.is_empty() { step(doc, n) } else { format!("{parent}/{}", step(doc, n)) };
    out.push(Line { depth, path: path.clone(), text: node_line(doc, n) });
    for c in n.children() {
        render_into(doc, c, path.clone(), depth + 1, out);
    }
}

/// The top-level block with this NM name.
pub fn top_block<'a>(doc: &'a Document, name: &str) -> Option<&'a Node> {
    doc.top_nodes().find(|n| matches!(&n.kind, Kind::Block(b) if doc.name(b.name) == Some(name)))
}

/// Names of every top-level block, file order.
pub fn top_block_names(doc: &Document) -> Vec<String> {
    doc.top_nodes().map(|n| node_name(doc, n)).collect()
}

/// One difference between two subtrees.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Only in A (the "before" save).
    Removed { path: String, node: String },
    /// Only in B.
    Added { path: String, node: String },
    /// A leaf (or non-block node) whose rendered value differs.
    Changed { path: String, a: String, b: String },
    /// Same keyed children, different order.
    Reordered { path: String },
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Change::Removed { path, .. } | Change::Added { path, .. } | Change::Changed { path, .. } | Change::Reordered { path } => path,
        }
    }
}

/// Selector keys `(parent block, child block, var)`: children of `parent` named `child` can be keyed
/// by the value of their own `var` instead of by position, written `child{var=value}`
/// (`community/stubs/a{idTag=01ff46161ae4726907000000001c000000}`).
///
/// `stubs > a > idTag` (the NPC stub's `IdTag`, 17 bytes): unique within every 5.x and old-gen save
/// checked, and an idTag keeps its `storyPhaseLocation` (spawn set + entries) and appearance across
/// consecutive saves of a playthrough (`tests/graph_fixtures.rs`, docs/plan/decision-chaining.md).
/// Unlike `a#k`, it does not move when the game swap-removes a stub, so a decision's NPC changes can
/// be addressed on a template whose stub list is ordered differently.
///
/// `layerStorage > entityData > idTag` (`universe > world[..]`): the saved entity state of a spawned
/// NPC carries the same idTag as its stub (119 of 148 entityData entries of T_letho are stubs); when
/// the game drops a stub it drops that entry too (T_letho -> Letho invited: 3 stubs and exactly their 3
/// entityData entries). Unique within each save checked.
pub const SELECTORS: [(&str, &str, &str); 2] = [("stubs", "a", "idTag"), ("layerStorage", "entityData", "idTag")];

/// The selector step of `n` under a parent named `parent` (`a{idTag=..}`), when [`SELECTORS`] names
/// that pair and `n` has the var. Values that would break a path (`/`, `#`, braces) are not used.
pub fn selector_step(doc: &Document, parent: &str, n: &Node) -> Option<String> {
    let Kind::Block(b) = &n.kind else { return None };
    let child = nm(doc, b.name);
    let (_, _, var) = SELECTORS.iter().find(|s| s.0 == parent && s.1 == child)?;
    let v = b.children.iter().find_map(|c| match &c.kind {
        Kind::Var(v) if doc.name(v.name) == Some(var) => Some(value_text(doc, &v.value)),
        _ => None,
    })?;
    if v.contains(['/', '#', '{', '}']) || v.contains("..(") {
        return None;
    }
    Some(format!("{child}{{{var}={v}}}"))
}

/// One NPC stub of `community > stubs` with the fields that identify it (all name-resolved text).
#[derive(Debug, Clone, PartialEq)]
pub struct StubInfo {
    /// Position in the stub list (`a#k`).
    pub index: usize,
    /// The `idTag` value as hex (the selector of [`SELECTORS`]); `None` if the stub has none.
    pub id_tag: Option<String>,
    /// `storyPhaseLocation > community` (the spawn set file), `""` when absent.
    pub community: String,
    /// `storyPhaseLocation > entry / storyEntry / entityEntry` GUIDs (full hex), `""` when absent.
    pub entries: [String; 3],
    pub appearance: String,
    pub state: Option<i32>,
    /// `position` X, Y, Z when the Vector decodes as `00 {X}{Y}{Z}{W} 0000`.
    pub position: Option<[f32; 3]>,
}

fn child_var<'a>(doc: &Document, n: &'a Node, name: &str) -> Option<&'a Var> {
    n.children().into_iter().find_map(|c| match &c.kind {
        Kind::Var(v) if doc.name(v.name) == Some(name) => Some(v),
        _ => None,
    })
}

fn child_block<'a>(doc: &Document, n: &'a Node, name: &str) -> Option<&'a Node> {
    n.children().into_iter().find(|c| matches!(&c.kind, Kind::Block(b) if doc.name(b.name) == Some(name)))
}

/// X, Y, Z of a `Vector` value (its first three 4-byte field values).
fn vector_xyz(v: &Var) -> Option<[f32; 3]> {
    let Value::Opaque(parts) = &v.value else { return None };
    let floats: Vec<f32> = parts
        .iter()
        .filter_map(|p| match p {
            Part::Node(Node { kind: Kind::Raw(r), .. }) => match r.parts.as_slice() {
                [Part::Bytes(b)] if b.len() == 4 => Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                _ => None,
            },
            _ => None,
        })
        .collect();
    (floats.len() >= 3).then(|| [floats[0], floats[1], floats[2]])
}

/// Every stub of `community > stubs`, in list order (empty when the save has no such block).
pub fn stubs(doc: &Document) -> Vec<StubInfo> {
    let Some(stubs) = top_block(doc, "community").and_then(|c| child_block(doc, c, "stubs")) else { return Vec::new() };
    let text = |n: &Node, name: &str| child_var(doc, n, name).map(|v| value_text(doc, &v.value)).unwrap_or_default();
    stubs
        .children()
        .into_iter()
        .filter(|c| matches!(&c.kind, Kind::Block(b) if doc.name(b.name) == Some("a")))
        .enumerate()
        .map(|(index, a)| {
            let loc = child_block(doc, a, "storyPhaseLocation");
            let guid = |name: &str| loc.and_then(|l| child_var(doc, l, name)).map(|v| value_text(doc, &v.value)).unwrap_or_default();
            StubInfo {
                index,
                id_tag: child_var(doc, a, "idTag").map(|v| value_text(doc, &v.value)),
                community: loc.map(|l| text(l, "community")).unwrap_or_default().trim_matches('"').replace("\\\\", "\\"),
                entries: [guid("entry"), guid("storyEntry"), guid("entityEntry")],
                appearance: text(a, "appearance"),
                state: child_var(doc, a, "state").and_then(|v| match v.value {
                    Value::I32(x) => Some(x),
                    _ => None,
                }),
                position: child_var(doc, a, "position").and_then(vector_xyz),
            }
        })
        .collect()
}

/// How [`diff_with`] keys children.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions {
    /// Key the children named in [`SELECTORS`] by their selector (`a{idTag=..}`) instead of by
    /// occurrence (`a#k`). The order of such a list is then not reported (no [`Change::Reordered`]):
    /// HYPOTHESIS: the order of `community > stubs` carries no meaning (the game swap-removes, so
    /// order is an artefact of removal history; gates G and P worked with the game's order only).
    pub stub_identity: bool,
}

/// Children of a block keyed by step text plus occurrence number (`questBlock[300f579b]#0`).
fn keyed<'a>(doc: &Document, kids: &[&'a Node]) -> Vec<(String, &'a Node)> {
    keyed_in(doc, None, kids)
}

/// [`keyed`], with selector steps for the children of `parent` when it is given.
fn keyed_in<'a>(doc: &Document, parent: Option<&Node>, kids: &[&'a Node]) -> Vec<(String, &'a Node)> {
    let pname = parent.map(|p| node_name(doc, p));
    let mut seen: HashMap<String, usize> = HashMap::new();
    kids.iter()
        .map(|c| {
            let s = pname.as_deref().and_then(|p| selector_step(doc, p, c)).unwrap_or_else(|| step(doc, c));
            let k = seen.entry(s.clone()).or_default();
            let key = format!("{s}#{k}");
            *k += 1;
            (key, *c)
        })
        .collect()
}

/// Keyed diff of two subtrees (A = before, B = after). Children of blocks and of nodes with nested
/// table entries are matched by [`step`] + occurrence; unmatched ones are reported whole (one line
/// each, not every descendant), matched ones are compared recursively.
pub fn diff(da: &Document, a: &Node, db: &Document, b: &Node) -> Vec<Change> {
    diff_with(da, a, db, b, DiffOptions::default())
}

/// [`diff`] with [`DiffOptions`].
pub fn diff_with(da: &Document, a: &Node, db: &Document, b: &Node, opts: DiffOptions) -> Vec<Change> {
    let mut out = Vec::new();
    diff_into(da, a, db, b, step(da, a), opts, &mut out);
    out
}

fn summary(doc: &Document, n: &Node) -> String {
    let d = n.descendants();
    if d == 0 {
        node_line(doc, n)
    } else {
        format!("{} (+{d} nodes, {} bytes)", node_line(doc, n), n.encoded_len())
    }
}

fn diff_into(da: &Document, a: &Node, db: &Document, b: &Node, path: String, opts: DiffOptions, out: &mut Vec<Change>) {
    if fingerprint(da, a) == fingerprint(db, b) {
        return;
    }
    let (ka, kb) = (a.children(), b.children());
    let structural = matches!(a.kind, Kind::Block(_)) && matches!(b.kind, Kind::Block(_));
    if !structural || (ka.is_empty() && kb.is_empty()) {
        out.push(Change::Changed { path, a: node_line(da, a), b: node_line(db, b) });
        return;
    }
    let (ka, kb) = if opts.stub_identity { (keyed_in(da, Some(a), &ka), keyed_in(db, Some(b), &kb)) } else { (keyed(da, &ka), keyed(db, &kb)) };
    let mb: BTreeMap<&str, &Node> = kb.iter().map(|(k, n)| (k.as_str(), *n)).collect();
    let ma: BTreeMap<&str, &Node> = ka.iter().map(|(k, n)| (k.as_str(), *n)).collect();
    for (k, n) in &ka {
        let child = format!("{path}/{}", k.trim_end_matches("#0"));
        match mb.get(k.as_str()) {
            Some(m) => diff_into(da, n, db, m, child, opts, out),
            None => out.push(Change::Removed { path: child, node: summary(da, n) }),
        }
    }
    for (k, n) in &kb {
        if !ma.contains_key(k.as_str()) {
            out.push(Change::Added { path: format!("{path}/{}", k.trim_end_matches("#0")), node: summary(db, n) });
        }
    }
    let selector_list = opts.stub_identity && ka.iter().chain(&kb).any(|(k, _)| k.contains('{'));
    let common_a: Vec<&str> = ka.iter().map(|x| x.0.as_str()).filter(|k| mb.contains_key(k)).collect();
    let common_b: Vec<&str> = kb.iter().map(|x| x.0.as_str()).filter(|k| ma.contains_key(k)).collect();
    if common_a != common_b && !selector_list {
        out.push(Change::Reordered { path });
    }
}

/// Diff of a named top-level block between two saves. `Ok(None)` when neither has it.
pub fn diff_top(da: &Document, db: &Document, name: &str) -> Result<Option<Vec<Change>>> {
    diff_top_with(da, db, name, DiffOptions::default())
}

/// [`diff_top`] with [`DiffOptions`].
pub fn diff_top_with(da: &Document, db: &Document, name: &str, opts: DiffOptions) -> Result<Option<Vec<Change>>> {
    match (top_block(da, name), top_block(db, name)) {
        (Some(a), Some(b)) => Ok(Some(diff_with(da, a, db, b, opts))),
        (None, None) => Ok(None),
        _ => Err(Error::Unverified(format!("top-level block {name} exists in only one save"))),
    }
}

/// True when `a`'s NM names are a prefix of `b`'s (what the game's own consecutive saves show).
pub fn names_prefix(a: &Document, b: &Document) -> bool {
    match (&a.names, &b.names) {
        (Some(x), Some(y)) => x.len() <= y.len() && x[..] == y[..x.len()],
        _ => false,
    }
}

/// One step of a graph transplant. Paths are the ones [`diff`] prints (`#k` only for a repeated
/// key past the first), starting with the top-level block name.
#[derive(Debug, Clone, PartialEq)]
pub enum GraftOp {
    /// Make the template's node at `path` equal to the donor's (whole subtree). Both must have it.
    Replace { path: String },
    /// Remove the template's node at `path` (its parent must be a `BS` block).
    Remove { path: String },
    /// Remove the node at `path` and move the LAST sibling block of the same name into its place: what the
    /// game does to `community > stubs > a` (21 -> 22 and the user's own mq0005 completion both show it:
    /// the arsonist's stub slot is taken by the former last stub, then the list is one shorter).
    SwapRemove { path: String },
    /// Copy the donor's node at `path` into the template's parent block, right after the sibling
    /// that precedes it in the donor (first position if no earlier donor sibling is in the template).
    Insert { path: String },
    /// Set the `Uint32`/`Int32` var at `path` to the number of its sibling blocks named `of`
    /// (`numBlocks`/`questBlock`, `numThreads`/`questThread`, `tagsCount`/`ExternalDialog`,
    /// `numCommunities`/`c`, `numStubs`/`a`; [`COUNT_RULES`], checked on every fixture by
    /// `tests/graph_fixtures.rs`).
    Recount { path: String, of: String },
}

/// `(parent block, count var, counted child block)`: the count var equals the number of child blocks
/// of that name. PROVEN on every 5.x fixture (`tests/graph_fixtures.rs`), so a transplant that adds or
/// removes such children can recompute the count.
pub const COUNT_RULES: [(&str, &str, &str); 5] = [
    ("questThread", "numBlocks", "questBlock"),
    ("questThread", "numThreads", "questThread"),
    ("CQuestExternalScenePlayer", "tagsCount", "ExternalDialog"),
    ("communities", "numCommunities", "c"),
    ("stubs", "numStubs", "a"),
];

impl GraftOp {
    pub fn path(&self) -> &str {
        match self {
            GraftOp::Replace { path }
            | GraftOp::Remove { path }
            | GraftOp::SwapRemove { path }
            | GraftOp::Insert { path }
            | GraftOp::Recount { path, .. } => path,
        }
    }
}

/// What [`transplant`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraftReport {
    /// Names appended to the template's NM table (prefix: the donor's extra names in the donor's
    /// order; remap: the names the copied subtrees use that the template lacked, in first use).
    pub names_added: usize,
    /// Remap only: donor name indices that stand at a different index in the result.
    pub names_remapped: usize,
    /// Variable-table entries before and after.
    pub nodes: (usize, usize),
    pub replaced: usize,
    pub removed: usize,
    pub inserted: usize,
    /// `(path, old, new)`.
    pub recounted: Vec<(String, i64, i64)>,
}

fn split_path(path: &str) -> Vec<String> {
    path.split('/').filter(|s| !s.is_empty()).map(|s| if s.contains('#') { s.to_owned() } else { format!("{s}#0") }).collect()
}

/// Location of a node: index of its top-level node, then positions in [`Node::children`] order.
pub type Loc = (usize, Vec<usize>);

/// Where the node at `path` is in this document, `Ok(None)` when it is not there.
pub fn locate(doc: &Document, path: &str) -> Result<Option<Loc>> {
    let steps = split_path(path);
    let Some((first, rest)) = steps.split_first() else {
        return Err(Error::Payload("empty graph path".into()));
    };
    let tops: Vec<&Node> = doc.top_nodes().collect();
    let Some(top) = tops.iter().position(|n| format!("{}#0", step(doc, n)) == *first) else {
        return Ok(None);
    };
    let mut at = tops[top];
    let mut idx = Vec::new();
    for s in rest {
        let kids = at.children();
        // A selector step (`a{idTag=..}`) is matched among selector keys; any other by step + occurrence.
        let keys = if s.contains('{') { keyed_in(doc, Some(at), &kids) } else { keyed(doc, &kids) };
        let Some(i) = keys.iter().position(|(k, _)| k == s) else { return Ok(None) };
        idx.push(i);
        at = kids[i];
    }
    Ok(Some((top, idx)))
}

/// The child steps of the node at `path` (as paths print them: `#0` dropped, selector steps for the
/// children named in [`SELECTORS`]), `Ok(None)` when the node is not there.
pub fn child_steps(doc: &Document, path: &str) -> Result<Option<Vec<String>>> {
    let Some(loc) = locate(doc, path)? else { return Ok(None) };
    let n = node_at(doc, &loc);
    Ok(Some(keyed_in(doc, Some(n), &n.children()).into_iter().map(|(k, _)| k.trim_end_matches("#0").to_owned()).collect()))
}

/// The node at a location from [`locate`].
pub fn node_at<'a>(doc: &'a Document, loc: &Loc) -> &'a Node {
    let mut n = doc.top_nodes().nth(loc.0).expect("located top node");
    for &i in &loc.1 {
        n = n.children()[i];
    }
    n
}

fn node_at_mut<'a>(doc: &'a mut Document, loc: &Loc) -> &'a mut Node {
    let mut n = doc.top_nodes_mut().nth(loc.0).expect("located top node");
    for &i in &loc.1 {
        n = n.children_mut().into_iter().nth(i).expect("located child");
    }
    n
}

/// A `BS` node's children (for a block, the table-entry children ARE `Block::children`).
fn block_kids_mut(n: &mut Node) -> Result<&mut Vec<Node>> {
    match &mut n.kind {
        Kind::Block(b) => Ok(&mut b.children),
        _ => Err(Error::Payload("graph edit: the parent is not a BS block".into())),
    }
}

/// Apply `ops` to `template`, copying subtrees from `donor`. All-or-nothing: works on a clone,
/// renumbers the table in post-order, rebases, re-parses the written bytes, then checks every op on
/// the result (replaced and inserted subtrees render identically to the donor's, removed paths are
/// gone) and that the table matches the tree.
///
/// [`NameMode::Prefix`] (this function) requires the template's NM names to be a prefix of the
/// donor's; the donor's extra names are appended in order first, so every name index inside copied
/// bytes (opaque values included) means the same name. HYPOTHESIS (gate G passed for one quest):
/// the game runs a quest frontier assembled this way.
pub fn transplant(template: &mut Document, donor: &Document, ops: &[GraftOp]) -> Result<GraftReport> {
    transplant_with(template, donor, ops, NameMode::Prefix)
}

/// How [`transplant_with`] makes the donor's NM indices mean the same names in the template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameMode {
    /// The template's NM names must be a prefix of the donor's; the donor's extra names are appended
    /// and copied bytes are used as they are (what the game's own consecutive saves show).
    Prefix,
    /// Every NM index inside a copied subtree is translated to the template's index for the same
    /// name ([`remap_node`]); names the template lacks are appended in order of first use. Refuses a
    /// subtree holding a value whose name-carrying layout is not decoded. When the template's names
    /// happen to be a prefix of the donor's, the donor's extra names are appended first as in
    /// `Prefix`, so the translation is the identity and the result is byte-identical to `Prefix`
    /// (the walk still runs and still refuses what it cannot decode).
    Remap,
}

/// Donor index -> template index through the name, appending the name when the template lacks it.
fn map_name(work: &mut Document, from: &[String], cache: &mut HashMap<u16, u16>, i: u16) -> Result<u16> {
    if let Some(&j) = cache.get(&i) {
        return Ok(j);
    }
    let name = from
        .get((i as usize).wrapping_sub(1))
        .ok_or_else(|| Error::Unverified(format!("name remap: index {i} is not in the donor's NM table")))?;
    let j = work.intern_name(name)?;
    cache.insert(i, j);
    Ok(j)
}

/// [`transplant`] with a choice of [`NameMode`]. In `Remap` mode the template's names need not be a
/// prefix of the donor's (a second donor on the same template, another playthrough), and the check
/// after writing compares subtrees by [`resolved_fingerprint`] (names, not indices). HYPOTHESIS: the
/// game accepts names we appended, and a remapped subtree means in the template what it meant in the
/// donor (untested in game; offline equality is proven, docs/findings/name-remap.md).
pub fn transplant_with(template: &mut Document, donor: &Document, ops: &[GraftOp], mode: NameMode) -> Result<GraftReport> {
    let unverified = |why: String| Error::Unverified(format!("graph transplant: {why}"));
    let prefix = names_prefix(template, donor);
    if mode == NameMode::Prefix && !prefix {
        return Err(unverified(
            "the template's NM names are not a prefix of the donor's (another playthrough, or an older donor); copied name indices would change meaning".into(),
        ));
    }
    let donor_names = donor.names.as_deref().ok_or_else(|| unverified("the donor's NM table could not be read".into()))?;
    let mut work = template.clone();
    work.rebase()?;
    if !work.indices_are_post_order() {
        return Err(unverified("the template's table indices are not in post-order".into()));
    }
    let mut report = GraftReport { nodes: (work.map.variables.len(), 0), ..Default::default() };
    let had = work.names.as_ref().map_or(0, Vec::len);
    if prefix && ops.iter().any(|o| matches!(o, GraftOp::Replace { .. } | GraftOp::Insert { .. })) {
        for name in &donor_names[had..] {
            work.intern_name(name)?;
        }
    }
    let mut cache: HashMap<u16, u16> = HashMap::new();
    let mut copy = |work: &mut Document, n: &Node| -> Result<Node> {
        match mode {
            NameMode::Prefix => Ok(n.clone()),
            NameMode::Remap => remap_node(donor_names, n, &mut |i| map_name(work, donor_names, &mut cache, i)),
        }
    };

    for op in ops {
        let path = op.path();
        let here = locate(&work, path)?;
        match op {
            GraftOp::Replace { .. } => {
                let there = locate(donor, path)?.ok_or_else(|| unverified(format!("{path} is not in the donor")))?;
                let here = here.ok_or_else(|| unverified(format!("{path} is not in the template")))?;
                let new = copy(&mut work, node_at(donor, &there))?;
                *node_at_mut(&mut work, &here) = new;
                report.replaced += 1;
            }
            GraftOp::Remove { .. } => {
                let (top, mut idx) = here.ok_or_else(|| unverified(format!("{path} is not in the template")))?;
                let last = idx.pop().ok_or_else(|| unverified("cannot remove a top-level block".into()))?;
                block_kids_mut(node_at_mut(&mut work, &(top, idx)))?.remove(last);
                report.removed += 1;
            }
            GraftOp::SwapRemove { .. } => {
                let (top, mut idx) = here.ok_or_else(|| unverified(format!("{path} is not in the template")))?;
                let at = idx.pop().ok_or_else(|| unverified("cannot remove a top-level block".into()))?;
                let parent = (top, idx);
                let name = node_name(&work, node_at(&work, &parent).children()[at]);
                let last = node_at(&work, &parent)
                    .children()
                    .iter()
                    .rposition(|c| matches!(c.kind, Kind::Block(_)) && node_name(&work, c) == name)
                    .expect("the removed node itself matches");
                let kids = block_kids_mut(node_at_mut(&mut work, &parent))?;
                kids.swap(at, last);
                kids.remove(last);
                report.removed += 1;
            }
            GraftOp::Insert { .. } => {
                if here.is_some() {
                    return Err(unverified(format!("{path} is already in the template")));
                }
                let (dtop, mut didx) = locate(donor, path)?.ok_or_else(|| unverified(format!("{path} is not in the donor")))?;
                let dlast = didx.pop().ok_or_else(|| unverified("cannot insert a top-level block".into()))?;
                let parent_path = path.rsplit_once('/').map(|x| x.0).unwrap_or_default();
                let tparent = locate(&work, parent_path)?.ok_or_else(|| unverified(format!("{parent_path} is not in the template")))?;
                let dparent = node_at(donor, &(dtop, didx));
                // Position by the same kind of keys the path uses (selector keys for `a{idTag=..}`).
                let sel = path.rsplit('/').next().is_some_and(|s| s.contains('{'));
                let keys = |d: &Document, p: &Node| -> Vec<String> {
                    let kids = p.children();
                    if sel { keyed_in(d, Some(p), &kids) } else { keyed(d, &kids) }.into_iter().map(|x| x.0).collect()
                };
                let dkeys = keys(donor, dparent);
                let new = copy(&mut work, dparent.children()[dlast])?;
                let tkeys = keys(&work, node_at(&work, &tparent));
                // Block children and table-entry children coincide for a BS parent.
                let at = dkeys[..dlast].iter().rev().find_map(|k| tkeys.iter().position(|t| t == k)).map_or(0, |p| p + 1);
                block_kids_mut(node_at_mut(&mut work, &tparent))?.insert(at, new);
                report.inserted += 1;
            }
            GraftOp::Recount { of, .. } => {
                let (top, mut idx) = here.ok_or_else(|| unverified(format!("{path} is not in the template")))?;
                let last = idx.pop().ok_or_else(|| unverified("a count is never a top-level block".into()))?;
                let parent = (top, idx);
                let count = {
                    let p = node_at(&work, &parent);
                    p.children().iter().filter(|c| matches!(c.kind, Kind::Block(_)) && node_name(&work, c) == *of).count()
                };
                let var = node_at_mut(&mut work, &parent).children_mut().into_iter().nth(last).expect("located count");
                let Kind::Var(v) = &mut var.kind else { return Err(unverified(format!("{path} is not a value"))) };
                let too_many = || unverified(format!("{path}: count {count} does not fit"));
                let old = match v.value {
                    Value::U32(x) => {
                        v.value = Value::U32(u32::try_from(count).map_err(|_| too_many())?);
                        i64::from(x)
                    }
                    Value::I32(x) => {
                        v.value = Value::I32(i32::try_from(count).map_err(|_| too_many())?);
                        i64::from(x)
                    }
                    _ => return Err(unverified(format!("{path} is not a Uint32/Int32 count"))),
                };
                report.recounted.push((path.to_owned(), old, count as i64));
            }
        }
    }

    report.names_added = work.names.as_ref().map_or(0, Vec::len) - had;
    report.names_remapped = cache.iter().filter(|(i, j)| i != j).count();
    work.renumber_post_order();
    work.rebase()?;
    report.nodes.1 = work.map.variables.len();

    // Check the result as it will be written: re-parse the bytes, then every op.
    let reread = Document::parse(&work.to_bytes(), work.base)?;
    let fail = |why: String| Err(Error::Payload(format!("graph transplant did not verify: {why}")));
    if reread.table() != reread.map.variables || !reread.indices_are_post_order() {
        return fail("variable table does not match the tree".into());
    }
    for op in ops {
        let path = op.path();
        let here = locate(&reread, path)?;
        match op {
            GraftOp::Replace { .. } | GraftOp::Insert { .. } => {
                let there = locate(donor, path)?.ok_or_else(|| unverified(format!("{path} is not in the donor")))?;
                let same = match (here, mode) {
                    (None, _) => false,
                    (Some(h), NameMode::Prefix) => fingerprint(&reread, node_at(&reread, &h)) == fingerprint(donor, node_at(donor, &there)),
                    (Some(h), NameMode::Remap) => {
                        resolved_fingerprint(&reread, node_at(&reread, &h))? == resolved_fingerprint(donor, node_at(donor, &there))?
                    }
                };
                if !same {
                    return fail(format!("{path} does not equal the donor's"));
                }
            }
            GraftOp::Remove { .. } | GraftOp::SwapRemove { .. } => {
                // A later sibling with the same key moves up into a removed `name#k` slot; a keyed path must be gone.
                if here.is_some() && !path.contains('#') && identity_of_last(path) {
                    return fail(format!("{path} is still there"));
                }
            }
            GraftOp::Recount { .. } => {}
        }
    }
    *template = reread;
    Ok(report)
}

/// The ops that replay `changes` (from [`diff`] A -> B) on A, restricted to paths under one of
/// `scopes` (all when empty): a changed leaf or reordered block is replaced by the donor's, a removed
/// node removed, an added node inserted, and a count var named in [`COUNT_RULES`] recounted instead of
/// copied (so a partial selection keeps counts right). Removals of unkeyed repeats (`a#3`) run from
/// the highest `#k` down so the earlier ones keep their numbers.
pub fn ops_from_diff(changes: &[Change], scopes: &[String]) -> Vec<GraftOp> {
    let inside = |p: &str| scopes.is_empty() || scopes.iter().any(|s| p == s || p.starts_with(&format!("{s}/")));
    let count_of = |p: &str| -> Option<String> {
        let mut it = p.rsplit('/');
        let (var, parent) = (it.next()?, it.next()?);
        let parent = parent.split(['[', '#']).next()?;
        COUNT_RULES.iter().find(|r| r.0 == parent && r.1 == var).map(|r| r.2.to_owned())
    };
    let (mut ops, mut removes, mut counts) = (Vec::new(), Vec::new(), Vec::new());
    for c in changes.iter().filter(|c| inside(c.path())) {
        let path = c.path().to_owned();
        match c {
            Change::Changed { .. } => match count_of(&path) {
                Some(of) => counts.push(GraftOp::Recount { path, of }),
                None => ops.push(GraftOp::Replace { path }),
            },
            Change::Reordered { .. } => ops.push(GraftOp::Replace { path }),
            Change::Removed { .. } => removes.push(path),
            Change::Added { .. } => ops.push(GraftOp::Insert { path }),
        }
    }
    let occurrence = |p: &String| p.rsplit('#').next().and_then(|k| k.parse::<usize>().ok()).unwrap_or(0);
    removes.sort_by_key(|p| std::cmp::Reverse(occurrence(p)));
    // Removals first (paths are the template's), then inserts and replacements, counts last. A stub
    // addressed by selector is swap-removed, as the game removes stubs (gate G evidence).
    let swap = |p: &str| p.rsplit('/').next().is_some_and(|s| s.starts_with("a{") && s.ends_with('}'));
    let mut out: Vec<GraftOp> =
        removes.into_iter().map(|path| if swap(&path) { GraftOp::SwapRemove { path } } else { GraftOp::Remove { path } }).collect();
    // A reordered or replaced parent already carries its children's changes; drop ops beneath it.
    let replaced: Vec<String> = ops.iter().filter(|&o| matches!(o, GraftOp::Replace { .. })).map(|o| o.path().to_owned()).collect();
    let under = |p: &str| replaced.iter().any(|r| p.starts_with(&format!("{r}/")));
    out.retain(|o| !under(o.path()));
    out.extend(ops.into_iter().filter(|o| !under(o.path())));
    for c in counts {
        // A count also needs recounting when its parent was not replaced as a whole.
        if !under(c.path()) {
            out.push(c);
        }
    }
    out
}

/// True when the last step of a path carries an identity (`name[id]`) or a selector
/// (`name{var=value}`), i.e. is unique by key.
fn identity_of_last(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|s| s.ends_with(']') || s.ends_with('}'))
}

// Name remapping. Every layout below was checked on every value of `questSystem` and `community`
// in 126 5.x saves (101 under C:\w3-extract, 25 under C:\w3-fixtures; tests/graph_fixtures.rs): each
// decoded name/type position holds a valid NM index, every value ends exactly where its layout says,
// and translating a value into another save's table gives the bytes the game wrote there
// (docs/findings/name-remap.md). Anything else is refused, never copied blind.

/// Struct types whose serialized layout is decoded. In the tree: a `00` byte, then per field
/// `u16 name, u16 type` and the value as a nested untagged table entry, then `u16 0` (`Vector` =
/// `00 {X:Float}{Y:Float}{Z:Float}{W:Float} 0000`, 35 bytes; `EngineTime` = `00 0000`, no fields).
/// Inside a record stream (below) a field is `u16 name, u16 type, u32 size, value[size - 4]`.
pub const STRUCT_TYPES: [&str; 5] = ["Vector", "SActionPointId", "GameTime", "EngineTime", "SQuestThreadSuspensionData"];

/// Types whose value is fixed-width data with no name index in it ([`fixed_len`] widths).
const PLAIN_TYPES: [&str; 12] = ["Bool", "Uint8", "Int8", "Uint16", "Int16", "Uint32", "Int32", "Uint64", "Int64", "Float", "Double", "CGUID"];

fn plain_width(ty: &str) -> Option<usize> {
    PLAIN_TYPES.contains(&ty).then(|| fixed_len(ty)).flatten()
}

/// Element type of `array:<n>,<m>,<element>` (`array:2,0,CName`, `array:154,0,Uint8`).
fn array_of(ty: &str) -> Option<&str> {
    let mut it = ty.strip_prefix("array:")?.splitn(3, ',');
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match (it.next(), it.next(), it.next()) {
        (Some(a), Some(b), Some(e)) if numeric(a) && numeric(b) => Some(e),
        _ => None,
    }
}

fn refuse<T>(why: impl std::fmt::Display) -> Result<T> {
    Err(Error::Unverified(format!("name remap: {why}")))
}

fn name_of(names: &[String], i: u16) -> Result<&str> {
    match names.get((i as usize).wrapping_sub(1)) {
        Some(n) => Ok(n),
        None => refuse(format!("index {i} is not in the NM table")),
    }
}

/// Maps one NM index of the source document to the index to write.
pub type NameMap<'a> = dyn FnMut(u16) -> Result<u16> + 'a;

/// A copy of `n` (a subtree of a document whose NM table is `names`) with every NM index passed
/// through `f`: `BS`/`VL`/`OP` names and types, `CName` values (0 = empty stays 0), and the indices
/// inside the opaque values decoded here: `IdTag` (none), `array:..,CName`, the [`STRUCT_TYPES`] and
/// arrays of them, and the `BLCK`/`AVAL`/`PORP` record streams in `SQuestThreadSuspensionData >
/// scopeData` and in `SXAP` sections (`treeState`). Sizes never change (u16 for u16).
///
/// `Error::Unverified` for anything else: an `Enum` value (EJournalStatus & co. are not proven to be
/// name indices), an opaque value of another type, a field or record of an undecoded type, a raw node
/// outside a decoded value, a value that does not tile exactly.
pub fn remap_node(names: &[String], n: &Node, f: &mut NameMap) -> Result<Node> {
    let kind = match &n.kind {
        Kind::Block(b) => {
            let name = f(b.name)?;
            let children = b.children.iter().map(|c| remap_node(names, c, f)).collect::<Result<_>>()?;
            Kind::Block(Block { name, children })
        }
        Kind::Var(v) => {
            let ty = name_of(names, v.ty)?;
            let (name, ty_index) = (f(v.name)?, f(v.ty)?);
            let value = match &v.value {
                Value::CName(0) => Value::CName(0),
                Value::CName(i) => Value::CName(f(*i)?),
                Value::Enum(_) => return refuse(format!("{ty} values are not proven to be name indices")),
                Value::Opaque(parts) => Value::Opaque(remap_opaque(names, ty, parts, f)?),
                plain => plain.clone(),
            };
            Kind::Var(Var { tag: v.tag, name, ty: ty_index, value })
        }
        Kind::Sized(s) => {
            let parts = match (&s.sxap, &s.content) {
                (Some(_), SsContent::Blob(parts)) => parts,
                _ => return refuse("an SS section that is not a SXAP record stream"),
            };
            let content = match parts.as_slice() {
                [] => Vec::new(),
                [Part::Bytes(b)] => vec![Part::Bytes(remap_records(names, b, f)?)],
                _ => return refuse("an SS body with nested table entries"),
            };
            Kind::Sized(Sized { sxap: s.sxap, content: SsContent::Blob(content) })
        }
        Kind::Manu(_) | Kind::Raw(_) => return refuse(format!("a {} node outside a decoded value", n.tag())),
    };
    Ok(Node { index: n.index, kind })
}

/// Reads the parts of a value in order and builds the remapped parts (same split, same lengths).
struct Cursor<'p> {
    parts: &'p [Part],
    at: usize,
    off: usize,
    out: Vec<Part>,
}

impl<'p> Cursor<'p> {
    fn new(parts: &'p [Part]) -> Self {
        Self { parts, at: 0, off: 0, out: Vec::with_capacity(parts.len()) }
    }

    /// The next `n` bytes; they must lie in one bytes part (a field never straddles a nested entry).
    fn take(&mut self, n: usize) -> Result<&'p [u8]> {
        if n == 0 {
            return Ok(&[]);
        }
        match self.parts.get(self.at) {
            Some(Part::Bytes(b)) if self.off + n <= b.len() => {
                let s = &b[self.off..self.off + n];
                self.off += n;
                if self.off == b.len() {
                    (self.at, self.off) = (self.at + 1, 0);
                }
                Ok(s)
            }
            _ => refuse("a value ends early, or a nested entry sits where bytes were expected"),
        }
    }

    fn emit(&mut self, b: &[u8]) {
        match self.out.last_mut() {
            Some(Part::Bytes(last)) => last.extend_from_slice(b),
            _ => self.out.push(Part::Bytes(b.to_vec())),
        }
    }

    fn copy(&mut self, n: usize) -> Result<&'p [u8]> {
        let b = self.take(n)?;
        self.emit(b);
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn copy_u32(&mut self) -> Result<u32> {
        let b = self.copy(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A `CName`: 0 (empty) stays 0.
    fn cname(&mut self, f: &mut NameMap) -> Result<()> {
        let i = self.u16()?;
        let j = if i == 0 { 0 } else { f(i)? };
        self.emit(&j.to_le_bytes());
        Ok(())
    }

    fn node(&mut self) -> Result<&'p Node> {
        match self.parts.get(self.at) {
            Some(Part::Node(n)) if self.off == 0 => {
                self.at += 1;
                Ok(n)
            }
            _ => refuse("expected a nested entry"),
        }
    }

    fn finish(self) -> Result<Vec<Part>> {
        if self.at == self.parts.len() {
            Ok(self.out)
        } else {
            refuse("data after the end of a decoded value")
        }
    }
}

fn remap_opaque(names: &[String], ty: &str, parts: &[Part], f: &mut NameMap) -> Result<Vec<Part>> {
    let mut c = Cursor::new(parts);
    match ty {
        // u8 flag (always 01 in the fixtures) + 16 bytes; the only small integer in it (last u32)
        // exceeds the NM table length in 18 values, so it is no name index.
        "IdTag" => {
            if c.copy(17)?[0] > 1 {
                return refuse("an IdTag whose first byte is not 0/1");
            }
        }
        t if STRUCT_TYPES.contains(&t) => tree_struct(names, t, &mut c, f)?,
        t => match array_of(t) {
            Some("CName") => {
                for _ in 0..c.copy_u32()? {
                    c.cname(f)?;
                }
            }
            Some(e) if STRUCT_TYPES.contains(&e) => {
                for _ in 0..c.copy_u32()? {
                    tree_struct(names, e, &mut c, f)?;
                }
            }
            _ => return refuse(format!("values of type {t} are not decoded")),
        },
    }
    c.finish()
}

/// One struct in tree form (see [`STRUCT_TYPES`]).
fn tree_struct(names: &[String], ty: &str, c: &mut Cursor, f: &mut NameMap) -> Result<()> {
    if c.copy(1)? != [0] {
        return refuse(format!("{ty} does not start with 00"));
    }
    loop {
        let name = c.u16()?;
        if name == 0 {
            c.emit(&[0, 0]);
            return Ok(());
        }
        let fty = c.u16()?;
        let (field, fty_name) = (name_of(names, name)?, name_of(names, fty)?);
        let (a, b) = (f(name)?, f(fty)?);
        c.emit(&a.to_le_bytes());
        c.emit(&b.to_le_bytes());
        let node = c.node()?;
        let Kind::Raw(r) = &node.kind else { return refuse(format!("{ty}.{field} is a tagged node")) };
        let parts = field_value(names, ty, field, fty_name, &r.parts, f)?;
        c.out.push(Part::Node(Node { index: node.index, kind: Kind::Raw(Raw { tag: r.tag.clone(), why: r.why, parts }) }));
    }
}

/// The value of one struct field (a nested untagged entry).
fn field_value(names: &[String], ty: &str, field: &str, fty: &str, parts: &[Part], f: &mut NameMap) -> Result<Vec<Part>> {
    let mut c = Cursor::new(parts);
    match (plain_width(fty), array_of(fty)) {
        (Some(w), _) => {
            c.copy(w)?;
        }
        // `scopeData`: u32 length + a record stream (the suspended scope's thread, saved like the tree).
        (None, Some("Uint8")) if ty == "SQuestThreadSuspensionData" && field == "scopeData" => {
            let n = c.copy_u32()? as usize;
            let body = c.take(n)?;
            let out = remap_records(names, body, f)?;
            c.emit(&out);
        }
        _ => return refuse(format!("{ty}.{field} of type {fty} is not decoded")),
    }
    c.finish()
}

fn le16(b: &[u8], at: usize) -> Result<u16> {
    match b.get(at..at + 2) {
        Some(x) => Ok(u16::from_le_bytes([x[0], x[1]])),
        None => refuse("a record stream ends inside a record header"),
    }
}

fn le32(b: &[u8], at: usize) -> Result<usize> {
    match b.get(at..at + 4) {
        Some(x) => Ok(u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize),
        None => refuse("a record stream ends inside a record header"),
    }
}

fn slice(b: &[u8], from: usize, len: usize) -> Result<&[u8]> {
    match from.checked_add(len).and_then(|end| b.get(from..end)) {
        Some(s) => Ok(s),
        None => refuse("a record runs past the end of its stream"),
    }
}

/// A record stream: `BLCK u16 name, u32 size, records[size]` (a block),
/// `AVAL|PORP u16 name, u16 type, u32 size, value[size]` (a value), back to back to the end.
/// Seen in `scopeData` (a suspended `questThread`, with the names and types of the tree) and in
/// `treeState > SS` (behaviour-tree state). Remapped copy of `b`.
pub fn remap_records(names: &[String], b: &[u8], f: &mut NameMap) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(b.len());
    records(names, b, f, &mut out)?;
    Ok(out)
}

fn records(names: &[String], b: &[u8], f: &mut NameMap, out: &mut Vec<u8>) -> Result<()> {
    let mut i = 0;
    while i < b.len() {
        let tag = slice(b, i, 4)?;
        out.extend_from_slice(tag);
        out.extend_from_slice(&f(le16(b, i + 4)?)?.to_le_bytes());
        match tag {
            b"BLCK" => {
                let size = le32(b, i + 6)?;
                out.extend_from_slice(&b[i + 6..i + 10]);
                records(names, slice(b, i + 10, size)?, f, out)?;
                i += 10 + size;
            }
            b"AVAL" | b"PORP" => {
                let ty = le16(b, i + 6)?;
                let size = le32(b, i + 8)?;
                out.extend_from_slice(&f(ty)?.to_le_bytes());
                out.extend_from_slice(&b[i + 8..i + 12]);
                record_value(names, name_of(names, ty)?, slice(b, i + 12, size)?, f, out)?;
                i += 12 + size;
            }
            _ => return refuse(format!("unknown record tag {tag:02x?}")),
        }
    }
    Ok(())
}

fn record_value(names: &[String], ty: &str, data: &[u8], f: &mut NameMap, out: &mut Vec<u8>) -> Result<()> {
    let cname = |i: u16, f: &mut NameMap| -> Result<u16> { if i == 0 { Ok(0) } else { f(i) } };
    match (plain_width(ty), array_of(ty)) {
        (Some(w), _) if data.len() == w => out.extend_from_slice(data),
        (None, None) if ty == "CName" && data.len() == 2 => out.extend_from_slice(&cname(le16(data, 0)?, f)?.to_le_bytes()),
        (None, Some("CName")) if data.len() >= 4 && data.len() == 4 + 2 * le32(data, 0)? => {
            out.extend_from_slice(&data[..4]);
            for k in (4..data.len()).step_by(2) {
                out.extend_from_slice(&cname(le16(data, k)?, f)?.to_le_bytes());
            }
        }
        // An empty array of structs (`suspendedScopesData` of a suspended thread); never seen non-empty.
        (None, Some(e)) if STRUCT_TYPES.contains(&e) && data == [0, 0, 0, 0] => out.extend_from_slice(data),
        (None, None) if STRUCT_TYPES.contains(&ty) => inline_struct(names, ty, data, f, out)?,
        _ => return refuse(format!("a record value of type {ty} ({} bytes) is not decoded", data.len())),
    }
    Ok(())
}

/// A struct inside a record stream: `00`, then per field `u16 name, u16 type, u32 size` (counting
/// itself) and `size - 4` value bytes, then `u16 0`. Seen as `timeRemaining:GameTime` =
/// `00 {m_seconds:Int32, size 8} 0000`, 15 bytes.
fn inline_struct(names: &[String], ty: &str, data: &[u8], f: &mut NameMap, out: &mut Vec<u8>) -> Result<()> {
    if data.first() != Some(&0) {
        return refuse(format!("{ty} does not start with 00"));
    }
    out.push(0);
    let mut i = 1;
    loop {
        let name = le16(data, i)?;
        if name == 0 {
            if i + 2 != data.len() {
                return refuse(format!("data after the end of {ty}"));
            }
            out.extend_from_slice(&[0, 0]);
            return Ok(());
        }
        let fty = le16(data, i + 2)?;
        let size = le32(data, i + 4)?;
        let fty_name = name_of(names, fty)?;
        let Some(len) = size.checked_sub(4) else { return refuse(format!("{ty} field size below 4")) };
        let value = slice(data, i + 8, len)?;
        if plain_width(fty_name) != Some(value.len()) {
            return refuse(format!("{ty} field of type {fty_name} ({} bytes) is not decoded", value.len()));
        }
        name_of(names, name)?;
        out.extend_from_slice(&f(name)?.to_le_bytes());
        out.extend_from_slice(&f(fty)?.to_le_bytes());
        out.extend_from_slice(&data[i + 4..i + 8]);
        out.extend_from_slice(value);
        i += 4 + size;
    }
}

/// Fingerprint of a subtree that does not depend on NM indices at all, opaque values included: the
/// subtree remapped onto local indices numbered in order of first use, plus those names. Two subtrees
/// (of any two saves) give the same string exactly when they hold the same bytes up to name indices
/// and the same name at every index position. Errors where [`remap_node`] refuses. Unlike
/// [`fingerprint`] (whose hashes guard the TOML deltas and stay as they are), this one is equal for a
/// remapped copy and its donor.
pub fn resolved_fingerprint(doc: &Document, n: &Node) -> Result<String> {
    let names = doc.names.as_deref().ok_or_else(|| Error::Unverified("name remap: the NM table could not be read".into()))?;
    let mut local: HashMap<u16, u16> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    let canon = remap_node(names, n, &mut |i| {
        if let Some(&j) = local.get(&i) {
            return Ok(j);
        }
        order.push(name_of(names, i)?);
        let j = u16::try_from(order.len()).map_err(|_| Error::Unverified("name remap: too many names".into()))?;
        local.insert(i, j);
        Ok(j)
    })?;
    Ok(format!("{}|{}", hex(&canon.to_bytes()), order.join("\u{1}")))
}

/// Give the keyed block at `path` (a `questBlock` or `questThread` addressed `name[xxxxxxxx]` by its
/// `GUID`/`guid` var) another GUID, keeping every other byte of the subtree. Returns the node's new
/// path. Refused when the last step is not GUID-keyed, the node is absent, or the parent already has
/// a sibling under the new key.
///
/// Research tooling for branch synthesis (docs/findings/branch-synthesis.md): it turns the donor's
/// waiting block of one decision output into the block another output waits at. HYPOTHESIS (one case
/// checked offline, q109 Keira `f5290e77` -> `c133524a`, equal to the real donor's bytes): a waiting
/// block's saved state (`inputName`, `activationState`, the condition flags) does not depend on its
/// GUID, so a block of the same class, entered through the same input socket and holding the same
/// number of conditions, saves the same bytes apart from the GUID. The caller picks such a block
/// from the quest graph; nothing here can check that.
pub fn rekey(doc: &mut Document, path: &str, guid: &[u8; 16]) -> Result<String> {
    let bad = |why: String| Error::Payload(format!("graph rekey {path}: {why}"));
    let (parent, last) = path.rsplit_once('/').ok_or_else(|| bad("not a path inside a block".into()))?;
    let (name, key) = last
        .strip_suffix(']')
        .and_then(|s| s.split_once('['))
        .filter(|(_, k)| k.len() == 8 && k.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| bad("the last step is not keyed by a GUID (`name[xxxxxxxx]`)".into()))?;
    let new_path = format!("{parent}/{name}[{}]", &guid_hex(guid)[..8]);
    if new_path != path && locate(doc, &new_path)?.is_some() {
        return Err(bad(format!("{new_path} already exists")));
    }
    let loc = locate(doc, path)?.ok_or_else(|| bad("not in this save".into()))?;
    let id_names = [doc.name_index("GUID"), doc.name_index("guid")];
    let n = node_at_mut(doc, &loc);
    let Kind::Block(b) = &mut n.kind else { return Err(bad("not a BS block".into())) };
    let id = b.children.iter_mut().find_map(|c| match &mut c.kind {
        Kind::Var(v) if id_names.contains(&Some(v.name)) && matches!(v.value, Value::Guid(g) if guid_hex(&g).starts_with(key)) => Some(v),
        _ => None,
    });
    let v = id.ok_or_else(|| bad(format!("no GUID child starting with {key}")))?;
    v.value = Value::Guid(*guid);
    doc.rebase()?;
    Ok(new_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn removed(p: &str) -> Change {
        Change::Removed { path: p.into(), node: String::new() }
    }
    fn added(p: &str) -> Change {
        Change::Added { path: p.into(), node: String::new() }
    }
    fn changed(p: &str) -> Change {
        Change::Changed { path: p.into(), a: String::new(), b: String::new() }
    }

    #[test]
    fn paths_split_into_keyed_steps() {
        assert_eq!(split_path("community/stubs/a#8"), ["community#0", "stubs#0", "a#8"]);
        assert_eq!(split_path("questSystem/quest[x.w2quest]/questThread"), ["questSystem#0", "quest[x.w2quest]#0", "questThread#0"]);
        assert!(identity_of_last("a/questBlock[300f579b]"));
        assert!(!identity_of_last("community/stubs/a#8"));
    }

    #[test]
    fn ops_remove_first_then_insert_and_recount_counts() {
        let t = "questSystem/q/questThread[38424c39]/questThread";
        let changes = vec![
            changed(&format!("{t}/numBlocks")),
            removed(&format!("{t}/questBlock[3446e000]")),
            added(&format!("{t}/questBlock[7fd9eeae]")),
            changed(&format!("{t}/questBlock[ce70ee7f]/activationState")),
            changed("questSystem/other/questThread/numBlocks"),
        ];
        let ops = ops_from_diff(&changes, &["questSystem/q".into()]);
        assert_eq!(
            ops,
            vec![
                GraftOp::Remove { path: format!("{t}/questBlock[3446e000]") },
                GraftOp::Insert { path: format!("{t}/questBlock[7fd9eeae]") },
                GraftOp::Replace { path: format!("{t}/questBlock[ce70ee7f]/activationState") },
                GraftOp::Recount { path: format!("{t}/numBlocks"), of: "questBlock".into() },
            ]
        );
    }

    #[test]
    fn repeated_unkeyed_removals_run_from_the_end() {
        let ops = ops_from_diff(&[removed("community/stubs/a#18"), removed("community/stubs/a#19"), removed("community/stubs/a")], &[]);
        let order: Vec<&str> = ops.iter().map(GraftOp::path).collect();
        assert_eq!(order, ["community/stubs/a#19", "community/stubs/a#18", "community/stubs/a"]);
    }

    #[test]
    fn a_reordered_parent_is_replaced_whole() {
        let p = "questSystem/questExternalScenePlayers/CQuestExternalScenePlayer";
        let changes = vec![
            removed(&format!("{p}/ExternalDialog[mq0005_arsonist]")),
            added(&format!("{p}/ExternalDialog[prologue_smith]")),
            changed(&format!("{p}/tagsCount")),
            Change::Reordered { path: p.into() },
        ];
        assert_eq!(ops_from_diff(&changes, &[]), vec![GraftOp::Replace { path: p.into() }]);
    }

    // Name remapping on hand-written values. Each value is built twice by one builder, once per NM
    // table; remapping the first through the names must give exactly the second.

    const NAMES: [&str; 30] = [
        "questThread", "inputName", "CName", "ActivatedInputs", "array:2,0,CName", "In", "In 1", "suspendedScopesData",
        "array:2,0,SQuestThreadSuspensionData", "timeRemaining", "GameTime", "m_seconds", "Int32", "active", "Bool", "scopeBlockGUID",
        "CGUID", "scopeData", "array:154,0,Uint8", "SQuestThreadSuspensionData", "position", "Vector", "X", "Y", "Float", "EngineTime",
        "spawnDelay", "IdTag", "idTag", "EJournalStatus",
    ];

    /// Table A (NAMES) and table B (reversed, plus two names in front, so every index moves).
    fn tables() -> (Vec<String>, Vec<String>) {
        let a: Vec<String> = NAMES.iter().map(|s| s.to_string()).collect();
        let mut b = vec!["pad0".to_string(), "pad1".to_string()];
        b.extend(a.iter().rev().cloned());
        (a, b)
    }

    fn ix(t: &[String], name: &str) -> u16 {
        t.iter().position(|x| x == name).expect("name in table") as u16 + 1
    }

    fn le(i: u16) -> [u8; 2] {
        i.to_le_bytes()
    }

    fn by_name<'a>(from: &'a [String], to: &'a [String]) -> impl FnMut(u16) -> Result<u16> + 'a {
        move |i| Ok(ix(to, &from[i as usize - 1]))
    }

    fn raw(b: Vec<u8>) -> Node {
        Node { index: 0, kind: Kind::Raw(Raw { tag: "?".into(), why: "test", parts: vec![Part::Bytes(b)] }) }
    }

    fn var(t: &[String], name: &str, ty: &str, parts: Vec<Part>) -> Node {
        Node { index: 0, kind: Kind::Var(Var { tag: crate::token::VarTag::Vl, name: ix(t, name), ty: ix(t, ty), value: Value::Opaque(parts) }) }
    }

    fn rec(tag: &[u8; 4], name: u16, ty: Option<u16>, body: &[u8]) -> Vec<u8> {
        let mut v = tag.to_vec();
        v.extend(le(name));
        if let Some(t) = ty {
            v.extend(le(t));
        }
        v.extend((body.len() as u32).to_le_bytes());
        v.extend(body);
        v
    }

    /// A suspended thread as the game saves it in `scopeData` (layout of the real records).
    fn stream(t: &[String]) -> Vec<u8> {
        let mut inner = rec(b"AVAL", ix(t, "inputName"), Some(ix(t, "CName")), &le(ix(t, "In")));
        let inputs = [&[1u8, 0, 0, 0][..], &le(ix(t, "In 1"))].concat();
        inner.extend(rec(b"AVAL", ix(t, "ActivatedInputs"), Some(ix(t, "array:2,0,CName")), &inputs));
        inner.extend(rec(b"AVAL", ix(t, "suspendedScopesData"), Some(ix(t, "array:2,0,SQuestThreadSuspensionData")), &[0; 4]));
        let gt = [&[0u8][..], &le(ix(t, "m_seconds")), &le(ix(t, "Int32")), &8u32.to_le_bytes(), &(-5i32).to_le_bytes(), &[0, 0]].concat();
        inner.extend(rec(b"AVAL", ix(t, "timeRemaining"), Some(ix(t, "GameTime")), &gt));
        inner.extend(rec(b"PORP", ix(t, "active"), Some(ix(t, "Bool")), &[1]));
        rec(b"BLCK", ix(t, "questThread"), None, &inner)
    }

    /// `suspendedScopesData` with one element: `00 {scopeBlockGUID:CGUID}{scopeData:array:154,0,Uint8} 0000`.
    fn suspension(t: &[String]) -> Node {
        let s = stream(t);
        let buf = [&(s.len() as u32).to_le_bytes()[..], &s].concat();
        let parts = vec![
            Part::Bytes([&[1u8, 0, 0, 0, 0][..], &le(ix(t, "scopeBlockGUID")), &le(ix(t, "CGUID"))].concat()),
            Part::Node(raw(vec![0xab; 16])),
            Part::Bytes([le(ix(t, "scopeData")), le(ix(t, "array:154,0,Uint8"))].concat()),
            Part::Node(raw(buf)),
            Part::Bytes(vec![0, 0]),
        ];
        var(t, "suspendedScopesData", "array:2,0,SQuestThreadSuspensionData", parts)
    }

    fn vector(t: &[String]) -> Node {
        let parts = vec![
            Part::Bytes([&[0u8][..], &le(ix(t, "X")), &le(ix(t, "Float"))].concat()),
            Part::Node(raw(1.5f32.to_le_bytes().to_vec())),
            Part::Bytes([le(ix(t, "Y")), le(ix(t, "Float"))].concat()),
            Part::Node(raw((-2.0f32).to_le_bytes().to_vec())),
            Part::Bytes(vec![0, 0]),
        ];
        var(t, "position", "Vector", parts)
    }

    #[test]
    fn opaque_values_remap_to_the_same_names() {
        let (a, b) = tables();
        let inputs = |t: &[String]| {
            let bytes = [&[3u8, 0, 0, 0][..], &le(ix(t, "In")), &[0, 0], &le(ix(t, "In 1"))].concat();
            var(t, "ActivatedInputs", "array:2,0,CName", vec![Part::Bytes(bytes)])
        };
        let empty = |t: &[String]| var(t, "spawnDelay", "EngineTime", vec![Part::Bytes(vec![0, 0, 0])]);
        let tag = |t: &[String]| var(t, "idTag", "IdTag", vec![Part::Bytes([&[1u8][..], &[0x5a; 16]].concat())]);
        type Build = fn(&[String]) -> Node;
        for (what, build) in [
            ("ActivatedInputs (an empty CName stays 0)", inputs as Build),
            ("Vector", vector as Build),
            ("EngineTime", empty as Build),
            ("IdTag", tag as Build),
            ("suspendedScopesData with a record stream", suspension as Build),
        ] {
            let (from, want) = (build(&a), build(&b));
            assert_ne!(from, want, "{what}: the tables must differ");
            assert_eq!(remap_node(&a, &from, &mut by_name(&a, &b)).unwrap(), want, "{what}");
            assert_eq!(remap_node(&b, &want, &mut by_name(&b, &a)).unwrap(), from, "{what} back");
        }
        // A treeState SS: SXAP + the same record stream.
        let ss = |t: &[String]| Node {
            index: 0,
            kind: Kind::Sized(Sized { sxap: Some(crate::token::Sxap { ints: [67, 29, 164] }), content: SsContent::Blob(vec![Part::Bytes(stream(t))]) }),
        };
        assert_eq!(remap_node(&a, &ss(&a), &mut by_name(&a, &b)).unwrap(), ss(&b));
        assert_eq!(remap_records(&a, &stream(&a), &mut by_name(&a, &b)).unwrap(), stream(&b));
    }

    #[test]
    fn undecoded_values_are_refused() {
        let (a, b) = tables();
        let refused = |n: &Node| matches!(remap_node(&a, n, &mut by_name(&a, &b)), Err(Error::Unverified(_)));
        // An opaque value of a type without a decoder (here: a CName that did not decode as 2 bytes).
        assert!(refused(&var(&a, "inputName", "CName", vec![Part::Bytes(vec![1, 2, 3])])));
        // An Enum value (not proven to be a name index).
        let e = Node { index: 0, kind: Kind::Var(Var { tag: crate::token::VarTag::Vl, name: 1, ty: ix(&a, "EJournalStatus"), value: Value::Enum(3) }) };
        assert!(refused(&e));
        // IdTag with a first byte other than 0/1; Vector with a trailing byte; a struct field of an undecoded type.
        assert!(refused(&var(&a, "idTag", "IdTag", vec![Part::Bytes(vec![2; 17])])));
        let mut v = vector(&a);
        if let Kind::Var(Var { value: Value::Opaque(p), .. }) = &mut v.kind {
            p.push(Part::Bytes(vec![9]));
        }
        assert!(refused(&v));
        let cname_field = vec![
            Part::Bytes([&[0u8][..], &le(ix(&a, "X")), &le(ix(&a, "CName"))].concat()),
            Part::Node(raw(le(ix(&a, "In")).to_vec())),
            Part::Bytes(vec![0, 0]),
        ];
        assert!(refused(&var(&a, "position", "Vector", cname_field)));
        // Record streams: an unknown tag, a value of an undecoded type, a record running past the end.
        let f = &mut by_name(&a, &b);
        assert!(remap_records(&a, b"XXXX\x01\x00", f).is_err());
        assert!(remap_records(&a, &rec(b"AVAL", 1, Some(ix(&a, "Vector")), &[1, 2, 3]), f).is_err());
        let s = stream(&a);
        assert!(remap_records(&a, &s[..s.len() - 1], f).is_err());
        // A raw node outside a decoded value.
        assert!(refused(&raw(vec![1, 2])));
    }

    #[test]
    fn array_type_names_parse() {
        assert_eq!(array_of("array:2,0,CName"), Some("CName"));
        assert_eq!(array_of("array:154,0,Uint8"), Some("Uint8"));
        assert_eq!(array_of("array:x,0,CName"), None);
        assert_eq!(array_of("CName"), None);
    }

    #[test]
    fn selector_paths_are_keyed_steps_and_stubs_swap_remove() {
        let stub = "community/stubs/a{idTag=0121659951849b055c00000000db070000}";
        assert_eq!(split_path(stub), ["community#0", "stubs#0", "a{idTag=0121659951849b055c00000000db070000}#0"]);
        assert!(identity_of_last(stub));
        let ent = "universe/world[novigrad.w2w]/layerStorage/entityData{idTag=01ab}";
        let ops = ops_from_diff(&[removed(stub), removed(ent), changed(&format!("{stub}/state")), added("community/stubs/a{idTag=01cd}")], &[]);
        assert_eq!(
            ops,
            vec![
                GraftOp::SwapRemove { path: stub.into() },
                GraftOp::Remove { path: ent.into() },
                GraftOp::Replace { path: format!("{stub}/state") },
                GraftOp::Insert { path: "community/stubs/a{idTag=01cd}".into() },
            ]
        );
    }

    #[test]
    fn scopes_match_whole_steps_only() {
        let ops = ops_from_diff(&[changed("a/questThread[1234]/x"), changed("a/questThread[12345]/x")], &["a/questThread[1234]".into()]);
        assert_eq!(ops, vec![GraftOp::Replace { path: "a/questThread[1234]/x".into() }]);
    }
}
