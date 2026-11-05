//! The save side of synthesis: which graph blocks a save waits at (`questSystem`), in which
//! scopes, and how a walk's prediction compares with what a played donor really contains.
use std::collections::{BTreeMap, BTreeSet, HashSet};

use w3save::graph;
use w3save::journal;
use w3save::token::{Kind, Value};
use w3save::{Document, Node};

use crate::quest::{guid_hex, short, BlockKind, Error, Guid, Library};
use crate::walk::{Event, Walk};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierBlock {
    pub guid: Guid,
    /// The saved `inputName` (the socket the block was activated through); the first one when the
    /// block holds several.
    pub input: String,
    /// Every saved `inputName` in file order (`inputNamesCount` of them): a `CQuestAndBlock` keeps one
    /// per input it has seen (pack 51: `a73fe883` holds `In 2`, `In 1`), everything else one.
    pub inputs: Vec<String>,
    pub n_conds: usize,
    /// Path as `w3al graph-diff` prints it.
    pub path: String,
    /// Scope wrappers above it, outermost first.
    pub scope: Vec<Guid>,
}

#[derive(Debug, Default, Clone)]
pub struct Frontier {
    pub blocks: Vec<FrontierBlock>,
    /// Running scopes: `(GUID, path of the wrapper)`.
    pub scopes: Vec<(Guid, String)>,
    /// `ExternalDialog` registry: `(tag, scene block GUID)`.
    pub scenes: Vec<(String, Guid)>,
    /// Scopes suspended while the player is in another world (`suspendedScopesData` of a running thread):
    /// `(phase block GUID, path of the suspendedScopesData var)`. Their state is a nested stream this crate
    /// does not decode (see [`suspended_entries`]).
    pub suspended: Vec<(Guid, String)>,
}

/// One entry of a `suspendedScopesData` array.
#[derive(Debug, Clone)]
pub struct SuspendedEntry {
    pub guid: Guid,
    pub guid_node: w3save::token::Node,
    pub data_node: w3save::token::Node,
}

/// The parsed value of a `suspendedScopesData` var: `u32 count`, then per entry
/// `00 | u16 name, u16 type, <GUID node, 16 bytes> | u16 name, u16 type, <data node> | 00 00`
/// (read from the 5.01 saves of the pack line: pack 43 holds 35 entries under the side-quest hub
/// `117e4006` while the player is at Kaer Morhen; every GUID is a phase block of the graphs; the data
/// node is a `BLCK`/`AVAL` stream). `None` when the value does not have exactly this shape.
#[derive(Debug, Clone)]
pub struct Suspended {
    /// `u16 name, u16 type` before the GUID and before the data (the same for every entry).
    pub head_guid: [u8; 4],
    pub head_data: [u8; 4],
    pub entries: Vec<SuspendedEntry>,
}

/// Parse a `suspendedScopesData` value (see [`Suspended`]). HYPOTHESIS: the layout above (checked to re-encode
/// byte for byte before any edit).
pub fn suspended_entries(parts: &[w3save::token::Part]) -> Option<Suspended> {
    use w3save::token::Part;
    // Flatten into a byte/node sequence.
    let mut bytes: Vec<u8> = Vec::new();
    let mut nodes: Vec<(usize, w3save::token::Node)> = Vec::new(); // (position in the byte stream, node)
    for p in parts {
        match p {
            Part::Bytes(b) => bytes.extend_from_slice(b),
            Part::Node(n) => nodes.push((bytes.len(), n.clone())),
        }
    }
    let count = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?) as usize;
    if nodes.len() != count * 2 {
        return None;
    }
    let mut out = Suspended { head_guid: [0; 4], head_data: [0; 4], entries: Vec::new() };
    let mut at = 4;
    for k in 0..count {
        let (gpos, gnode) = &nodes[2 * k];
        let (dpos, dnode) = &nodes[2 * k + 1];
        // 00 + head_guid before the GUID node.
        if *gpos != at + 5 || bytes.get(at) != Some(&0) {
            return None;
        }
        let hg: [u8; 4] = bytes.get(at + 1..at + 5)?.try_into().ok()?;
        if *dpos != gpos + 4 {
            return None;
        }
        let hd: [u8; 4] = bytes.get(*gpos..gpos + 4)?.try_into().ok()?;
        if k == 0 {
            out.head_guid = hg;
            out.head_data = hd;
        } else if hg != out.head_guid || hd != out.head_data {
            return None;
        }
        if bytes.get(*dpos..dpos + 2) != Some(&[0, 0][..]) {
            return None;
        }
        let w3save::token::Kind::Raw(r) = &gnode.kind else { return None };
        let [Part::Bytes(g)] = &r.parts[..] else { return None };
        let guid: Guid = g.as_slice().try_into().ok()?;
        out.entries.push(SuspendedEntry { guid, guid_node: gnode.clone(), data_node: dnode.clone() });
        at = dpos + 2;
    }
    (at == bytes.len()).then_some(out)
}

impl Suspended {
    /// The value's parts again (the exact inverse of [`suspended_entries`]).
    pub fn to_parts(&self) -> Vec<w3save::token::Part> {
        use w3save::token::Part;
        let mut parts = Vec::new();
        let mut pending: Vec<u8> = (self.entries.len() as u32).to_le_bytes().to_vec();
        for e in &self.entries {
            pending.push(0);
            pending.extend_from_slice(&self.head_guid);
            parts.push(Part::Bytes(std::mem::take(&mut pending)));
            parts.push(Part::Node(e.guid_node.clone()));
            parts.push(Part::Bytes(self.head_data.to_vec()));
            parts.push(Part::Node(e.data_node.clone()));
            pending.extend_from_slice(&[0, 0]);
        }
        parts.push(Part::Bytes(pending));
        parts
    }
}

impl Frontier {
    pub fn guids(&self) -> BTreeSet<Guid> {
        self.blocks.iter().map(|b| b.guid).collect()
    }

    pub fn scope_guids(&self) -> HashSet<Guid> {
        self.scopes.iter().map(|s| s.0).collect()
    }

    pub fn find(&self, g: &Guid) -> Option<&FrontierBlock> {
        self.blocks.iter().find(|b| &b.guid == g)
    }
}

fn var_value<'a>(doc: &Document, n: &'a Node, name: &str) -> Option<&'a Value> {
    n.children().into_iter().find_map(|c| match &c.kind {
        Kind::Var(v) if doc.name(v.name) == Some(name) => Some(&v.value),
        _ => None,
    })
}

fn child_block<'a>(doc: &Document, n: &'a Node, name: &str) -> Option<&'a Node> {
    n.children().into_iter().find(|c| matches!(&c.kind, Kind::Block(b) if doc.name(b.name) == Some(name)))
}

/// Read the frontier of a save.
pub fn read(doc: &Document) -> Result<Frontier, Error> {
    let top = graph::top_block(doc, "questSystem").ok_or_else(|| Error::Other("the save has no questSystem block".into()))?;
    let mut f = Frontier::default();
    if let Some(players) = child_block(doc, top, "questExternalScenePlayers") {
        for p in players.children() {
            for d in p.children() {
                if matches!(&d.kind, Kind::Block(b) if doc.name(b.name) == Some("ExternalDialog")) {
                    let tag = var_value(doc, d, "tag").map(|v| graph::value_text(doc, v)).unwrap_or_default();
                    if let Some(Value::Guid(g)) = var_value(doc, d, "guid") {
                        f.scenes.push((tag, *g));
                    }
                }
            }
        }
    }
    let mut path = "questSystem".to_owned();
    visit(doc, top, &mut path, &mut Vec::new(), &mut f);
    Ok(f)
}

fn visit(doc: &Document, n: &Node, path: &mut String, scope: &mut Vec<Guid>, f: &mut Frontier) {
    for c in n.children() {
        if let Kind::Var(v) = &c.kind {
            if doc.name(v.name) == Some("suspendedScopesData") {
                if let Value::Opaque(parts) = &v.value {
                    if let Some(s) = suspended_entries(parts) {
                        let p = format!("{path}/suspendedScopesData");
                        f.suspended.extend(s.entries.iter().map(|e| (e.guid, p.clone())));
                    }
                }
            }
            continue;
        }
        let Kind::Block(b) = &c.kind else { continue };
        let name = doc.name(b.name).unwrap_or("");
        let step = graph::step(doc, c);
        let len = path.len();
        path.push('/');
        path.push_str(&step);
        match name {
            "questBlock" => {
                if let Some(Value::Guid(g)) = var_value(doc, c, "GUID") {
                    let inputs: Vec<String> = c
                        .children()
                        .into_iter()
                        .filter_map(|k| match &k.kind {
                            Kind::Var(v) if doc.name(v.name) == Some("inputName") => Some(graph::value_text(doc, &v.value)),
                            _ => None,
                        })
                        .map(|s| if s == "''" { String::new() } else { s })
                        .collect();
                    let input = inputs.first().cloned().unwrap_or_default();
                    let n_conds = child_block(doc, c, "conditions")
                        .and_then(|k| var_value(doc, k, "numConditions"))
                        .and_then(|v| if let Value::U32(x) = v { Some(*x as usize) } else { None })
                        .unwrap_or(0);
                    f.blocks.push(FrontierBlock { guid: *g, input, inputs, n_conds, path: path.clone(), scope: scope.clone() });
                }
            }
            "questThread" => {
                if let Some(Value::Guid(g)) = var_value(doc, c, "GUID") {
                    f.scopes.push((*g, path.clone()));
                    scope.push(*g);
                    visit(doc, c, path, scope, f);
                    scope.pop();
                } else {
                    visit(doc, c, path, scope, f);
                }
            }
            "quest" | "questSystem" => visit(doc, c, path, scope, f),
            _ => {}
        }
        path.truncate(len);
    }
}

/// The template's facts as a sum function for the walker.
pub fn fact_sums(doc: &Document) -> Result<BTreeMap<String, i64>, Error> {
    let t = doc.facts_table()?;
    Ok(t.facts().iter().map(|f| (f.name.clone(), f.sum())).collect())
}

/// Facts that are play noise, not quest state (from decision-chaining.md A): dialogue line ids
/// `N_N_N`, hit detectors, kill statistics, load stamps.
pub fn is_noise_fact(id: &str) -> bool {
    let parts: Vec<&str> = id.split('_').collect();
    (parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
        || id.contains("_hit")
        || id.contains("was_killed")
        || id.ends_with("_noticeboard_empty")
        || id.ends_with("_notice_board_empty")
        || id == "MeditationWaitFinished"
        || id.starts_with("ach_")
        || id.starts_with("statistic_")
        || id.starts_with("InteractiveEntity_")
        || id == "game_is_loaded"
        || id.starts_with("tut_")
        || id.starts_with("tutorial_")
        || id.starts_with("achun_")
        || id.starts_with("RemasterSkill")
        || id.starts_with("switch_")
        || id.starts_with("mh305_merchant")
        || id.ends_with("_weapon_hit")
}

/// What the walk predicts for the save, as GUID sets.
#[derive(Debug, Default)]
pub struct Prediction {
    pub removed: BTreeSet<Guid>,
    pub added: BTreeSet<Guid>,
    pub facts: BTreeSet<String>,
    pub facts_removed: BTreeSet<String>,
    /// Journal entries activated: `(key, status)`.
    pub journal: Vec<(Vec<Guid>, String)>,
    pub scopes_removed: BTreeSet<Guid>,
    pub scopes_added: BTreeSet<Guid>,
}

pub fn predict(lib: &Library, walk: &Walk, template: &Frontier) -> Prediction {
    let mut p = Prediction::default();
    let exited: HashSet<Guid> = walk.exited.iter().map(|i| lib.blocks[*i].guid).collect();
    let reentered: HashSet<Guid> = walk.reentered.iter().map(|i| lib.blocks[*i].guid).collect();
    let gone = |g: &Guid| exited.contains(g) && !reentered.contains(g);
    for b in &template.blocks {
        let consumed = lib.get(&b.guid).is_some_and(|i| walk.consumed.contains(&i)) && !reentered.contains(&b.guid);
        if consumed || gone(&b.guid) || b.scope.iter().any(|s| exited.contains(s)) {
            p.removed.insert(b.guid);
        }
    }
    for (g, _) in &template.scopes {
        if gone(g) || template.scopes.iter().any(|(s, _)| exited.contains(s) && s != g && is_inside(template, g, s)) {
            p.scopes_removed.insert(*g);
        }
    }
    for w in &walk.waits {
        p.added.insert(lib.blocks[w.block].guid);
    }
    for e in &walk.entered {
        let g = lib.blocks[*e].guid;
        p.added.insert(g);
        p.scopes_added.insert(g);
    }
    // A wait that was already in the template is not "added" (re-activation through the same input).
    let had = template.guids();
    p.added.retain(|g| !had.contains(g) || p.removed.contains(g));
    p.facts = walk.facts.iter().filter(|(_, v)| **v > 0).map(|(k, _)| k.clone()).collect();
    p.facts_removed = walk.removed_facts.iter().cloned().collect();
    for e in &walk.events {
        if let Event::Journal { path, status, .. } = e {
            p.journal.push((path.iter().map(|s| s.guid).collect(), status.clone()));
        }
    }
    p
}

fn is_inside(f: &Frontier, inner: &Guid, outer: &Guid) -> bool {
    let op = f.scopes.iter().find(|s| &s.0 == outer).map(|s| s.1.as_str()).unwrap_or("");
    let ip = f.scopes.iter().find(|s| &s.0 == inner).map(|s| s.1.as_str()).unwrap_or("");
    !op.is_empty() && ip.starts_with(op) && ip != op
}

/// Compare a prediction with a real donor played from the template. Lines for a report plus
/// the counts `(hits, misses, extras)` per layer: `hits` = predicted and real, `misses` = real
/// but not predicted, `extras` = predicted but not real.
pub struct Report {
    pub lines: Vec<String>,
    pub blocks: (usize, usize, usize),
    pub facts: (usize, usize, usize),
    pub journal: (usize, usize, usize),
}

fn label(lib: &Library, g: &Guid) -> String {
    lib.get(g).map_or_else(|| format!("{} (not in the graphs)", short(g)), |i| lib.describe(i))
}

pub fn compare(lib: &Library, pred: &Prediction, template: &Document, donor: &Document) -> Result<Report, Error> {
    let tf = read(template)?;
    let df = read(donor)?;
    let (tg, dg) = (tf.guids(), df.guids());
    let real_removed: BTreeSet<Guid> = tg.difference(&dg).copied().collect();
    let real_added: BTreeSet<Guid> = dg.difference(&tg).copied().collect();
    let mut lines = Vec::new();
    let mut blocks = (0, 0, 0);

    // Files the prediction touches: real changes in other files are listed apart (other play / noise).
    let touched: HashSet<String> = pred.removed.iter().chain(&pred.added).filter_map(|g| lib.get(g)).map(|i| lib.blocks[i].file.clone()).collect();
    let in_touched = |g: &Guid| lib.get(g).is_some_and(|i| touched.contains(&lib.blocks[i].file));

    lines.push("## questSystem blocks (frontier GUIDs)".into());
    for (what, predicted, real) in [("removed", &pred.removed, &real_removed), ("added", &pred.added, &real_added)] {
        for g in predicted {
            if real.contains(g) {
                blocks.0 += 1;
                lines.push(format!("  ok   {what}  {}", label(lib, g)));
            } else {
                blocks.2 += 1;
                lines.push(format!("  EXTRA {what}  {}  (predicted, not in the donor)", label(lib, g)));
            }
        }
        for g in real {
            if !predicted.contains(g) {
                let tag = if in_touched(g) { "MISS " } else { "other" };
                if tag == "MISS " {
                    blocks.1 += 1;
                }
                let wait = df.find(g).or_else(|| tf.find(g)).map(|b| format!(" input={:?} conds={}", b.input, b.n_conds)).unwrap_or_default();
                lines.push(format!("  {tag} {what}  {}{wait}", label(lib, g)));
            }
        }
    }

    lines.push("## facts".into());
    let tfacts = fact_sums(template)?;
    let dfacts = fact_sums(donor)?;
    let real_facts: BTreeSet<String> = dfacts.keys().filter(|k| !tfacts.contains_key(*k) && !is_noise_fact(k)).cloned().collect();
    let mut facts = (0, 0, 0);
    for f in &pred.facts {
        if real_facts.contains(f) || tfacts.contains_key(f) && dfacts.get(f) != tfacts.get(f) {
            facts.0 += 1;
            lines.push(format!("  ok    {f}"));
        } else if tfacts.contains_key(f) {
            lines.push(format!("  ok?   {f}  (already in the template; the graph adds to it)"));
            facts.0 += 1;
        } else {
            facts.2 += 1;
            lines.push(format!("  EXTRA {f}"));
        }
    }
    for f in &real_facts {
        if !pred.facts.contains(f) {
            facts.1 += 1;
            lines.push(format!("  MISS  {f}"));
        }
    }
    let real_removed: BTreeSet<String> = tfacts.keys().filter(|k| !dfacts.contains_key(*k) && !is_noise_fact(k)).cloned().collect();
    for f in &pred.facts_removed {
        if real_removed.contains(f) {
            facts.0 += 1;
            lines.push(format!("  ok    {f} removed"));
        } else {
            facts.2 += 1;
            lines.push(format!("  EXTRA {f} removed (still in the donor)"));
        }
    }
    for f in &real_removed {
        if !pred.facts_removed.contains(f) {
            facts.1 += 1;
            lines.push(format!("  MISS  {f} removed"));
        }
    }

    lines.push("## journal".into());
    let tj = journal::read(template)?;
    let dj = journal::read(donor)?;
    let d = journal::diff(&tj, &dj);
    let mut real_j: Vec<(Vec<Guid>, String, String)> = d.added.iter().map(|(_, e)| (e.key(), e.status.clone(), e.label())).collect();
    real_j.extend(d.changed.iter().filter(|c| !c.path_changed()).map(|c| (c.after.key(), c.after.status.clone(), c.after.label())));
    // HYPOTHESIS (lukwraith 21 -> 22: 5 flips; spirit donor: 4): when a quest's root entry is set
    // to Success, every Active entry under it becomes Success too.
    let mut predicted: Vec<(Vec<Guid>, String)> = pred.journal.clone();
    for (key, status) in &pred.journal {
        if status != "JS_Success" || key.len() > 2 {
            continue;
        }
        for e in &tj.entries {
            let k = e.key();
            if k.len() > key.len() && k.starts_with(key) && e.status == "JS_Active" && !predicted.iter().any(|(pk, _)| *pk == k) {
                predicted.push((k, "JS_Success".into()));
            }
        }
    }
    let mut journal_c = (0, 0, 0);
    for (key, status) in &predicted {
        let hit = real_j.iter().find(|(k, s, _)| k == key && s == status);
        let path = key.iter().map(short).collect::<Vec<_>>().join("/");
        match hit {
            Some((_, _, l)) => {
                journal_c.0 += 1;
                lines.push(format!("  ok    {status} {l}"));
            }
            None => {
                let same_key = real_j.iter().find(|(k, _, _)| k == key);
                if let Some((_, s, l)) = same_key {
                    journal_c.0 += 1;
                    lines.push(format!("  ok~   {status} {l} (donor has {s}: a later block flipped it)"));
                } else {
                    journal_c.2 += 1;
                    lines.push(format!("  EXTRA {status} {path}"));
                }
            }
        }
    }
    for (key, status, l) in &real_j {
        if !predicted.iter().any(|(k, _)| k == key) {
            journal_c.1 += 1;
            let who = key.last().and_then(|g| lib.journal_by_last.get(g)).map(|b| format!("  (activated by {})", lib.describe(*b))).unwrap_or_default();
            lines.push(format!("  MISS  {status} {l}{who}"));
        }
    }
    lines.push(format!(
        "## totals  blocks hit/miss/extra {}/{}/{}   facts {}/{}/{}   journal {}/{}/{}",
        blocks.0, blocks.1, blocks.2, facts.0, facts.1, facts.2, journal_c.0, journal_c.1, journal_c.2
    ));
    Ok(Report { lines, blocks, facts, journal: journal_c })
}

/// One line per frontier block, with the graph's description.
pub fn describe_frontier(lib: &Library, f: &Frontier, grep: Option<&str>) -> Vec<String> {
    f.blocks
        .iter()
        .filter(|b| grep.is_none_or(|g| b.path.contains(g) || label(lib, &b.guid).contains(g)))
        .map(|b| {
            let kind = lib.get(&b.guid).map(|i| match &lib.blocks[i].kind {
                BlockKind::Pause { conds } => format!(" {conds:?}"),
                _ => String::new(),
            });
            format!("{} input={:?} conds={}  {}{}  @ {}", guid_hex(&b.guid), b.input, b.n_conds, label(lib, &b.guid), kind.unwrap_or_default(), b.path)
        })
        .collect()
}
