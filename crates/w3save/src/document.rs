//! Layer 3: the edit surface. Parse to a typed token tree, serialize from it.
//!
//! Design rule from the plan: never patch bytes in place. A [`Document`] is the payload cut
//! into ordered regions; `to_bytes` concatenates them. Bytes that no variable-table entry
//! covers (header, `RB`, `NM` tag, pointer block, table, footer) are [`Region::Gap`]s kept as
//! raw bytes. Every top-level table entry is a [`Region::Variable`] holding a typed
//! [`Node`] tree (see `token`): `BS` blocks with children, `VL`/`OP` values, `SS` sections,
//! the `MANU` name table. Anything the tokenizer does not recognise stays a raw node and
//! still round-trips.
//!
//! `rebase` (Phase 0 step 8) walks the tree: node sizes come from [`Node::encoded_len`], so
//! after a value changes the variable table, the `RB` offsets, the pointer block and the footer
//! are rewritten from the tree (see [`Document::rebase`] for what is proven and what is not).
use crate::error::{Error, Result};
use crate::facts::FactTable;
use crate::payload::{self, PayloadMap, VarEntry};
use crate::token::{Kind, Node, SsContent, TokenStats, Tree};

#[derive(Debug, Clone, PartialEq)]
pub enum Region {
    /// Bytes not covered by any variable-table entry: header, NM tag, RB, table, footer.
    Gap(Vec<u8>),
    /// A top-level variable: table entry `index` (position in the on-disk table), how many
    /// other entries nest inside it, and its typed tree.
    Variable { index: usize, nested: usize, node: Node },
}

impl Region {
    pub fn len(&self) -> usize {
        match self {
            Region::Gap(b) => b.len(),
            Region::Variable { node, .. } => node.encoded_len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Region::Gap(b) => b.clone(),
            Region::Variable { node, .. } => node.to_bytes(),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Region::Gap(b) => out.extend_from_slice(b),
            Region::Variable { node, .. } => node.write(out),
        }
    }
}

/// Requested world-state changes, in plain terms. `w3al-core` turns quest effects
/// into this; this crate never knows what a "quest" means in story terms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edits {
    /// (fact identifier, value)
    pub facts: Vec<(String, i64)>,
    /// (quest id, phase: "started" | "completed" | "failed")
    pub quests: Vec<(String, String)>,
    /// journal entry ids to add
    pub journal: Vec<String>,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty() && self.quests.is_empty() && self.journal.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    pub base: u32,
    pub map: PayloadMap,
    /// The NM name table, `None` when it could not be read (then values stay opaque).
    pub names: Option<Vec<String>>,
    pub regions: Vec<Region>,
}

impl Document {
    pub fn parse(payload: &[u8], base: u32) -> Result<Self> {
        let map = payload::read_map(payload, base)?;
        let names = payload::read_names(payload, &map).ok();

        let mut spans: Vec<(usize, usize, usize)> = Vec::with_capacity(map.variables.len());
        for (index, v) in map.variables.iter().enumerate() {
            let start = payload::rel(v.offset, base, payload.len())?;
            let end = start
                .checked_add(v.size as usize)
                .filter(|&e| e <= payload.len())
                .ok_or_else(|| Error::Payload(format!("variable {index} runs past the end of the payload")))?;
            spans.push((start, end, index));
        }
        // FINDING (Phase 0, 5.x save): the table is NOT in physical order, and entries
        // NEST: entry 1 (rel 16, size 32) fully contains entry 0 (rel 20, size 28).
        // `Tree` sorts by start (outermost first) and derives the nesting; a partial
        // overlap (crossing a parent's end) is an error.
        let tree = Tree::new(payload, names.as_deref(), spans)?;

        let mut regions: Vec<Region> = Vec::with_capacity(tree.top.len() * 2 + 1);
        let mut cursor = 0usize;
        for &k in &tree.top {
            let (start, end, index) = tree.span(k);
            if start > cursor {
                regions.push(Region::Gap(payload[cursor..start].to_vec()));
            }
            let node = tree.build(k, 0)?;
            debug_assert_eq!(node.encoded_len(), end - start);
            regions.push(Region::Variable { index, nested: node.descendants(), node });
            cursor = end;
        }
        if cursor < payload.len() {
            regions.push(Region::Gap(payload[cursor..].to_vec()));
        }
        Ok(Self { base, map, names, regions })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.regions.iter().map(Region::len).sum());
        for r in &self.regions {
            r.write(&mut out);
        }
        out
    }

    /// Top-level nodes in file order.
    pub fn top_nodes(&self) -> impl Iterator<Item = &Node> {
        self.regions.iter().filter_map(|r| match r {
            Region::Variable { node, .. } => Some(node),
            Region::Gap(_) => None,
        })
    }

    /// Visit every node of the document (outermost first, file order).
    pub fn visit<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        for n in self.top_nodes() {
            n.visit(f);
        }
    }

    /// The NM name for a 1-based index, if the table was read.
    pub fn name(&self, idx: u16) -> Option<&str> {
        self.names.as_ref()?.get((idx as usize).checked_sub(1)?).map(String::as_str)
    }

    /// The `SS` section that holds the fact table (`SXAP`, `SBDF`, facts, `EBDF`), if the
    /// tokenizer recognised one. On 5.01 saves it is the only child of the top-level `BS`
    /// named `facts`. Read it through the node's [`Kind::Sized`]: `body_bytes()` for the raw
    /// body, `facts()` for the parsed list.
    pub fn facts_node(&self) -> Option<&Node> {
        let mut found = None;
        self.visit(&mut |n| {
            if found.is_none() && is_facts_ss(n) {
                found = Some(n);
            }
        });
        found
    }

    /// Mutable access to the facts `SS` node. Changing its content changes its encoded size;
    /// the variable table must then be recomputed (`rebase`, step 8).
    pub fn facts_node_mut(&mut self) -> Option<&mut Node> {
        fn find(n: &mut Node) -> Option<&mut Node> {
            if is_facts_ss(n) {
                return Some(n);
            }
            match &mut n.kind {
                Kind::Block(b) => b.children.iter_mut().find_map(find),
                _ => None,
            }
        }
        self.regions.iter_mut().find_map(|r| match r {
            Region::Variable { node, .. } => find(node),
            Region::Gap(_) => None,
        })
    }

    /// Recompute the variable table from the tree: entry `i` is the span of the node whose
    /// `index` is `i`, positioned by the sizes of everything before it. On an unedited
    /// document this equals `map.variables` exactly (checked on every fixture).
    pub fn table(&self) -> Vec<VarEntry> {
        let flat = self.layout();
        let mut out = vec![VarEntry { offset: 0, size: 0 }; flat.len()];
        for (index, start, size) in flat {
            if let Some(e) = out.get_mut(index) {
                *e = VarEntry { offset: self.base + start as u32, size: size as u32 };
            }
        }
        out
    }

    /// True when every node's table index is its post-order rank (children before their parent,
    /// file order), counted over the whole document. PROVEN on 122 of 122 fixture files
    /// (docs/research.md, 2026-11-04 "journal editor"); edits that add nodes check it first and
    /// refuse a document where it does not hold.
    pub fn indices_are_post_order(&self) -> bool {
        let mut next = 0usize;
        self.top_nodes().all(|n| n.is_post_order(&mut next)) && next == self.map.variables.len()
    }

    /// Re-number every node in post-order (see [`Document::indices_are_post_order`]) and return the
    /// new number of table entries. Needed after nodes were added or removed; follow it with
    /// [`Document::rebase`], which then writes a table of the new length.
    pub fn renumber_post_order(&mut self) -> usize {
        let mut next = 0usize;
        for r in &mut self.regions {
            if let Region::Variable { index, nested, node } = r {
                node.renumber_post_order(&mut next);
                *index = node.index;
                *nested = node.descendants();
            }
        }
        next
    }

    /// Mutable top-level nodes in file order.
    pub fn top_nodes_mut(&mut self) -> impl Iterator<Item = &mut Node> {
        self.regions.iter_mut().filter_map(|r| match r {
            Region::Variable { node, .. } => Some(node),
            Region::Gap(_) => None,
        })
    }

    /// Index of a name in the NM table (1-based, what `BS`/`VL` headers store), if present.
    pub fn name_index(&self, name: &str) -> Option<u16> {
        let i = self.names.as_ref()?.iter().position(|n| n == name)?;
        u16::try_from(i + 1).ok()
    }

    /// Index of `name`, appending it to the end of the NM table (the `MANU` node) when missing.
    /// Appending never changes an existing index, so nothing else needs rewriting; `MANU` grows and
    /// `rebase` moves the pointer block, table and footer behind it. Call `rebase` afterwards.
    ///
    /// Evidence: the game's own table grows the same way between consecutive saves (B's names = A's
    /// names + new ones at the end in 43 of 44 lukwraith pairs; `JS_Success` first appears like that in
    /// 01 -> 02). HYPOTHESIS: the game also accepts a name that WE appended (untested in game).
    pub fn intern_name(&mut self, name: &str) -> Result<u16> {
        if let Some(i) = self.name_index(name) {
            return Ok(i);
        }
        if name.is_empty() || name.len() > 255 || !name.chars().all(|c| (c as u32) < 0x100 && !c.is_control()) {
            return Err(Error::Payload(format!("{name:?} cannot be an NM name (1..255 Latin-1 characters)")));
        }
        let names = self.names.as_mut().ok_or_else(|| Error::Unverified("the NM table could not be read".into()))?;
        let index = u16::try_from(names.len() + 1).map_err(|_| Error::Unverified("the NM table is full".into()))?;
        let mut manus = self.regions.iter_mut().filter_map(|r| match r {
            Region::Variable { node: Node { kind: Kind::Manu(m), .. }, .. } => Some(m),
            _ => None,
        });
        let (Some(manu), None) = (manus.next(), manus.next()) else {
            return Err(Error::Unverified("expected exactly one MANU node".into()));
        };
        if manu.names.len() != names.len() {
            return Err(Error::Unverified("MANU and the decoded name table disagree".into()));
        }
        manu.names.push(name.chars().map(|c| c as u32 as u8).collect());
        names.push(name.to_owned());
        Ok(index)
    }

    /// `(table index, start, size)` of every node, `start` relative to the payload.
    fn layout(&self) -> Vec<(usize, usize, usize)> {
        let mut flat = Vec::new();
        let mut at = 0usize;
        for r in &self.regions {
            if let Region::Variable { node, .. } = r {
                node.layout(at, &mut flat);
            }
            at += r.len();
        }
        flat
    }

    /// Typed-vs-raw coverage of the whole tree, per tag.
    pub fn token_stats(&self) -> TokenStats {
        let mut st = TokenStats::default();
        for n in self.top_nodes() {
            *st.top_level.entry(n.tag().to_string()).or_default() += 1;
            st.add(n, self.names.as_deref(), 0);
        }
        st
    }

    /// The fact table of the document, parsed from the facts `SS` node. Errors when the
    /// document has no recognised facts section, or more than one.
    pub fn facts_table(&self) -> Result<FactTable> {
        let mut found = 0usize;
        self.visit(&mut |n| found += is_facts_ss(n) as usize);
        if found != 1 {
            return Err(Error::Payload(format!("expected exactly one recognised facts section, found {found}")));
        }
        match self.facts_node().map(|n| &n.kind) {
            Some(Kind::Sized(s)) => FactTable::parse(&s.body_bytes()),
            _ => Err(Error::Payload("facts section not found".into())),
        }
    }

    /// Edit the fact table through `f`, write it back into the tree and `rebase`.
    ///
    /// All-or-nothing for failures inside `f` or while writing the table back: the document is
    /// only touched after `f` succeeded. The layout is checked (`rebase`) before anything
    /// changes, so a document that cannot be rebased is refused untouched. After a failure of
    /// the final `rebase` the document must be discarded (its offsets are stale).
    pub fn edit_facts<R>(&mut self, f: impl FnOnce(&mut FactTable) -> Result<R>) -> Result<R> {
        self.rebase()?;
        let mut table = self.facts_table()?;
        let out = f(&mut table)?;
        let node = self.facts_node_mut().ok_or_else(|| Error::Payload("facts section not found".into()))?;
        let Kind::Sized(s) = &mut node.kind else {
            return Err(Error::Payload("facts section is not an SS node".into()));
        };
        s.set_facts(&table)?;
        if FactTable::parse(&s.body_bytes())? != table {
            return Err(Error::Payload("fact table does not re-parse to what was written".into()));
        }
        self.rebase()?;
        Ok(out)
    }

    /// Apply world-state edits. Empty edits are a no-op so the "export with no
    /// effects equals the template" check (Phase 2 exit) can run before Phase 1.
    ///
    /// Facts (`edits.facts`): each `(name, value)` is a script-style `FactsSet`: the fact ends
    /// up with exactly ONE permanent record `{value, time, expiry -1}` and `c1 = 0`, created at
    /// the end of the table when missing ([`FactTable::set_value`]). Pairs apply in order, so a
    /// repeated name keeps the last value. That "set replaces the record list" is now proven by
    /// the game's script (`facts.ws`: `FactsSet` = `FactsRemove` + `FactsAdd(.., validFor -1)`,
    /// see docs/research.md, 2026-11-04 "script semantics"). HYPOTHESES left (native code, not
    /// proven in game): that the stamp `time` may be the latest record time of the table
    /// ([`FactTable::latest_time`]); that the record order and the table position of a new fact
    /// do not matter; that a 16-bit signed value field is right (values outside
    /// `-32768..=65535` are refused).
    ///
    /// Quest and journal edits are refused with [`Error::Unverified`] (nothing is guessed
    /// about `questSystem` / `CJournalManager`, plan unknown 3), before anything is changed.
    pub fn apply(&mut self, edits: &Edits) -> Result<()> {
        if edits.is_empty() {
            return Ok(());
        }
        if !edits.quests.is_empty() || !edits.journal.is_empty() {
            return Err(Error::Unverified(
                "quest and journal edits are not implemented: whether questSystem / CJournalManager must change for a \
                 quest to show as done is still open (docs/research.md, plan unknown 3)"
                    .into(),
            ));
        }
        self.edit_facts(|t| {
            // HYPOTHESIS: FactsSet semantics (see the doc comment above); unproven in game.
            let time = t.latest_time();
            for (name, value) in &edits.facts {
                let v = i32::try_from(*value)
                    .map_err(|_| Error::Payload(format!("fact {name:?}: value {value} does not fit a 16-bit record field")))?;
                t.set_value(name, v, time)?;
            }
            Ok(())
        })
    }

    /// Recompute everything that holds an absolute offset, after a node changed size. Call it
    /// after editing the tree (`apply` and `edit_facts` do). On an unedited document it changes
    /// nothing.
    ///
    /// What depends on the layout (PROVEN on 118 of 118 fixture files, docs/research.md,
    /// 2026-11-04 "RB decoded"; the tail layout is checked here before it is rewritten, and an
    /// unfamiliar layout is an error, never a guess):
    ///
    /// * the variable table: (offset, size) of every node, computed from the tree. The entry
    ///   count is the number of nodes: unchanged by fact edits; journal appends add nodes and
    ///   renumber the tree first ([`Document::renumber_post_order`]), so the table grows;
    /// * `RB`: `"RB" u32 n` then `n x (u16 NM name index, u32 absolute offset)`, entry `i`
    ///   describing the `i`-th top-level `BS` node in file order. Names are checked against the
    ///   tree, offsets are rewritten;
    /// * the pointer block before the table: `u32 NM offset`, `u32 RB offset`, `"SC"` (the two
    ///   bytes are kept as read);
    /// * the footer: `u32` variable-table offset + `"SE"`.
    ///
    /// Everything else just tiles after the changed node and moves with it. Known gaps, kept
    /// out of this claim: absolute offsets *inside* values (none found: a scan of the 118
    /// fixture files for the absolute/relative offsets of the top-level blocks turned up
    /// only chance hits), and anything stored in the container header or the `.json` sidecar.
    pub fn rebase(&mut self) -> Result<()> {
        let unverified = |why: &str| Error::Unverified(format!("rebase: {why}"));
        // HYPOTHESIS: nothing else in the payload stores an absolute offset (none found in a scan of
        // 118 files); only the in-game load of an edited save can confirm it.
        let n = self.regions.len();
        let mut manus = self
            .regions
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, Region::Variable { node, .. } if matches!(node.kind, Kind::Manu(_))))
            .map(|(i, _)| i);
        let (Some(manu), None) = (manus.next(), manus.next()) else {
            return Err(unverified("expected exactly one MANU node"));
        };
        if manu == 0 || manu + 2 != n {
            return Err(unverified("MANU is not followed by exactly one trailing gap (pointer block, table, footer)"));
        }
        let (Region::Gap(old_rb), Region::Gap(old_tail)) = (&self.regions[manu - 1], &self.regions[manu + 1]) else {
            return Err(unverified("the regions around MANU are not the RB and tail gaps"));
        };

        // Where every region starts now.
        let mut starts = Vec::with_capacity(n);
        let mut at = 0usize;
        for r in &self.regions {
            starts.push(at);
            at += r.len();
        }
        let base = self.base as usize;
        let abs = |pos: usize| {
            u32::try_from(base + pos).map_err(|_| Error::Payload("payload outside the 32-bit offset range".into()))
        };

        // RB gap = "RB" u32 n, n x (u16, u32), "NM".
        if old_rb.len() < 8 || &old_rb[..2] != b"RB" || &old_rb[old_rb.len() - 2..] != b"NM" {
            return Err(unverified("the gap before MANU is not RB .. NM"));
        }
        let count = u32::from_le_bytes([old_rb[2], old_rb[3], old_rb[4], old_rb[5]]) as usize;
        if old_rb.len() != 6 + 6 * count + 2 {
            return Err(unverified("RB entry count does not match the gap length"));
        }
        let blocks: Vec<(u16, usize)> = self.regions[..manu - 1]
            .iter()
            .zip(&starts)
            .filter_map(|(r, &s)| match r {
                Region::Variable { node: Node { kind: Kind::Block(b), .. }, .. } => Some((b.name, s)),
                _ => None,
            })
            .collect();
        let old_names: Vec<u16> = (0..count)
            .map(|i| u16::from_le_bytes([old_rb[6 + 6 * i], old_rb[7 + 6 * i]]))
            .collect();
        if old_names != blocks.iter().map(|b| b.0).collect::<Vec<_>>() {
            return Err(unverified("RB entries are not the top-level BS nodes in file order"));
        }
        let mut new_rb = Vec::with_capacity(old_rb.len());
        new_rb.extend_from_slice(b"RB");
        new_rb.extend_from_slice(&(count as u32).to_le_bytes());
        for (name, start) in &blocks {
            new_rb.extend_from_slice(&name.to_le_bytes());
            new_rb.extend_from_slice(&abs(*start)?.to_le_bytes());
        }
        new_rb.extend_from_slice(b"NM");
        let rb_offset = abs(starts[manu - 1])?;
        let nm_offset = abs(starts[manu - 1] + old_rb.len() - 2)?;

        // Tail = pointer block (10) + variable table + footer (6).
        let old_vars = self.map.variables.len();
        let declared = old_tail.get(10..14).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        if old_tail.len() != 10 + 4 + 8 * old_vars + 6 || declared != Some(old_vars) {
            return Err(unverified("the tail is not pointer block + table + footer"));
        }
        // The entry count follows the tree. It only differs from the old one after nodes were added
        // (journal appends), and then the indices must already be renumbered 0..n
        // (`renumber_post_order`); a gap or a duplicate is refused below.
        let flat = self.layout();
        let vars = flat.len();
        let mut table: Vec<Option<VarEntry>> = vec![None; vars];
        for (index, start, size) in flat {
            match table.get_mut(index) {
                Some(slot @ None) => *slot = Some(VarEntry { offset: abs(start)?, size: size as u32 }),
                _ => return Err(unverified("a node's table index is out of range or used twice")),
            }
        }
        let vt_offset = abs(starts[manu + 1] + 10)?;
        let mut new_tail = Vec::with_capacity(old_tail.len());
        new_tail.extend_from_slice(&nm_offset.to_le_bytes());
        new_tail.extend_from_slice(&rb_offset.to_le_bytes());
        new_tail.extend_from_slice(&old_tail[8..10]);
        new_tail.extend_from_slice(&(vars as u32).to_le_bytes());
        let mut variables = Vec::with_capacity(vars);
        for e in table {
            let e = e.ok_or_else(|| unverified("a variable-table entry has no node"))?;
            new_tail.extend_from_slice(&e.offset.to_le_bytes());
            new_tail.extend_from_slice(&e.size.to_le_bytes());
            variables.push(e);
        }
        new_tail.extend_from_slice(&vt_offset.to_le_bytes());
        new_tail.extend_from_slice(b"SE");
        debug_assert_eq!(new_tail.len(), 10 + 4 + 8 * vars + 6);

        self.regions[manu - 1] = Region::Gap(new_rb);
        self.regions[manu + 1] = Region::Gap(new_tail);
        self.map.rb_offset = rb_offset;
        self.map.nm_offset = nm_offset;
        self.map.variable_table_offset = vt_offset;
        self.map.variables = variables;
        Ok(())
    }
}

fn is_facts_ss(n: &Node) -> bool {
    matches!(&n.kind, Kind::Sized(s) if matches!(s.content, SsContent::Facts(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_serialize_is_identical() {
        let p = payload::testutil::synthetic(3084);
        let doc = Document::parse(&p, 3084).unwrap();
        assert_eq!(doc.to_bytes(), p);
        assert!(doc.regions.iter().any(|r| matches!(r, Region::Variable { .. })));
        // The synthetic variables are all described by the table, and the tree reproduces it.
        assert_eq!(doc.table(), doc.map.variables);
    }

    #[test]
    fn overlapping_variables_are_rejected() {
        let mut p = payload::testutil::synthetic(3084);
        // Rewrite the second table entry's offset to overlap the first.
        let map = payload::read_map(&p, 3084).unwrap();
        let vt = payload::rel(map.variable_table_offset, 3084, p.len()).unwrap();
        let entry1 = vt + 4 + 8;
        let first_off = map.variables[0].offset;
        p[entry1..entry1 + 4].copy_from_slice(&(first_off + 1).to_le_bytes());
        assert!(Document::parse(&p, 3084).is_err());
    }

    #[test]
    fn nested_variables_are_counted_not_rejected() {
        // Phase 0 finding: real saves nest table entries (a child inside its parent).
        let mut p = payload::testutil::synthetic(3084);
        let map = payload::read_map(&p, 3084).unwrap();
        let vt = payload::rel(map.variable_table_offset, 3084, p.len()).unwrap();
        let entry1 = vt + 4 + 8;
        p[entry1..entry1 + 4].copy_from_slice(&(map.variables[0].offset + 2).to_le_bytes());
        p[entry1 + 4..entry1 + 8].copy_from_slice(&2u32.to_le_bytes()); // 2+2 = 4 = the parent's size
        let doc = Document::parse(&p, 3084).unwrap();
        assert_eq!(doc.to_bytes(), p);
        assert_eq!(doc.table(), doc.map.variables);
        assert!(doc
            .regions
            .iter()
            .any(|r| matches!(r, Region::Variable { index: 0, nested: 1, .. })));
    }

    #[test]
    fn empty_edits_are_a_noop_and_real_edits_need_facts() {
        let p = payload::testutil::synthetic(3084);
        let mut doc = Document::parse(&p, 3084).unwrap();
        doc.apply(&Edits::default()).unwrap();
        assert_eq!(doc.to_bytes(), p);
        // The plain synthetic payload has no facts section: refused, document untouched.
        let e = Edits { facts: vec![("x".into(), 1)], ..Default::default() };
        assert!(doc.apply(&e).is_err());
        assert_eq!(doc.to_bytes(), p);
    }

    #[test]
    fn synthetic_tokens_are_typed() {
        let p = payload::testutil::synthetic(3084);
        let doc = Document::parse(&p, 3084).unwrap();
        let st = doc.token_stats();
        // MANU is typed; the empty BS and the VL (whose type name is not a known type, so its
        // value stays opaque) are typed too.
        assert_eq!(st.by_tag["MANU"].typed, 1);
        assert_eq!(st.by_tag["VL"].typed, 1);
        assert_eq!(st.by_tag["BS"].typed, 1);
        assert_eq!(st.total, 3);
    }

    fn facts_doc() -> (Vec<u8>, Document) {
        let p = payload::testutil::synthetic_with_facts(
            3084,
            &[
                ("bugfix_132404", 0, &[(1, 1130.5, -1.0)]),
                ("ach_attack", 1, &[(1, 1200.0, 1204.0)]),
                ("empty_one", 0, &[]),
            ],
        );
        let doc = Document::parse(&p, 3084).unwrap();
        (p, doc)
    }

    #[test]
    fn rebase_without_changes_is_byte_identical() {
        let (p, mut doc) = facts_doc();
        doc.rebase().unwrap();
        assert_eq!(doc.to_bytes(), p);
        assert_eq!(doc.map.variables, payload::read_map(&p, 3084).unwrap().variables);
        let p = payload::testutil::synthetic(3084);
        let mut doc = Document::parse(&p, 3084).unwrap();
        doc.rebase().unwrap();
        assert_eq!(doc.to_bytes(), p);
    }

    /// The facts BS grows by `delta` bytes: every pointer must land on the same thing.
    #[test]
    fn apply_grows_facts_and_rebase_shifts_everything() {
        let (p, mut doc) = facts_doc();
        let old = payload::read_map(&p, 3084).unwrap();
        let old_rb = payload::read_rb(&p, &old).unwrap();
        doc.apply(&Edits { facts: vec![("w3al_new".into(), 1), ("empty_one".into(), 3)], ..Default::default() })
            .unwrap();
        let q = doc.to_bytes();
        let delta = (q.len() - p.len()) as u32;
        // new fact: 1 (name header) + 8 + 4 + 10; "empty_one" gains one record: + 10
        assert_eq!(delta, 1 + 8 + 4 + 10 + 10);

        // Everything re-parses and the read-back map equals what rebase wrote.
        let new = payload::read_map(&q, 3084).unwrap();
        assert_eq!(new.variables, doc.map.variables);
        assert!(new.notes.is_empty());
        let again = Document::parse(&q, 3084).unwrap();
        assert_eq!(again.to_bytes(), q);
        assert_eq!(again.table(), new.variables);

        // Table: entries 0 (facts BS) and 1 (SS) grow; entries 2 and 3 (second block) and
        // MANU shift; nothing else moves.
        let (v, w) = (&old.variables, &new.variables);
        assert_eq!((w[0].offset, w[0].size), (v[0].offset, v[0].size + delta));
        assert_eq!((w[1].offset, w[1].size), (v[1].offset, v[1].size + delta));
        for i in 2..5 {
            assert_eq!((w[i].offset, w[i].size), (v[i].offset + delta, v[i].size), "entry {i}");
        }
        // RB: same names, second offset shifted, first one did not move.
        let rb = payload::read_rb(&q, &new).unwrap();
        assert_eq!(rb.iter().map(|e| e.name).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(rb[0].offset, old_rb[0].offset);
        assert_eq!(rb[1].offset, old_rb[1].offset + delta);
        // Pointer block and footer.
        assert_eq!(new.rb_offset, old.rb_offset + delta);
        assert_eq!(new.nm_offset, old.nm_offset + delta);
        assert_eq!(new.variable_table_offset, old.variable_table_offset + delta);
        assert_eq!(new.pre_table_bytes, old.pre_table_bytes);
        assert_eq!(payload::read_names(&q, &new).unwrap(), payload::read_names(&p, &old).unwrap());

        // Facts: the new one is appended, the zero-record one was replaced by one record.
        let t = again.facts_table().unwrap();
        assert_eq!(t.len(), 4);
        assert_eq!(t.facts()[3].name, "w3al_new");
        assert_eq!(t.get("empty_one").unwrap().sum(), 3);
        assert_eq!(t.get("bugfix_132404").unwrap().records.len(), 1, "untouched facts stay");
        // `latest_time` stamp: the newest record time of the template (1200.0).
        assert_eq!(t.get("w3al_new").unwrap().records[0].time, 1200.0);
    }

    #[test]
    fn exact_state_edits_shrink_and_restore() {
        let (p, mut doc) = facts_doc();
        let before = doc.facts_table().unwrap();
        doc.edit_facts(|t| {
            t.remove("ach_attack");
            t.set_records("empty_one", vec![crate::facts::FactRecord { value: 2, time: 3.0, expiry: 9.0 }])?;
            Ok(())
        })
        .unwrap();
        let q = doc.to_bytes();
        let new = payload::read_map(&q, 3084).unwrap();
        assert_eq!(Document::parse(&q, 3084).unwrap().table(), new.variables);
        assert!(q.len() != p.len());
        // Putting the old facts back restores the same state (the removed fact returns at the end).
        doc.edit_facts(|t| {
            for n in before.differing(t) {
                t.match_fact(&n, &before)?;
            }
            Ok(())
        })
        .unwrap();
        let after = doc.facts_table().unwrap();
        assert!(before.differing(&after).is_empty());
        assert_eq!(after.len(), before.len());
        assert_eq!(doc.to_bytes().len(), p.len());
    }

    #[test]
    fn quest_and_journal_edits_are_refused_untouched() {
        let (p, mut doc) = facts_doc();
        let e = Edits { quests: vec![("q".into(), "completed".into())], ..Default::default() };
        assert!(matches!(doc.apply(&e), Err(Error::Unverified(_))));
        let e = Edits { journal: vec!["j".into()], facts: vec![("x".into(), 1)], ..Default::default() };
        assert!(matches!(doc.apply(&e), Err(Error::Unverified(_))));
        assert_eq!(doc.to_bytes(), p, "nothing changed");
    }

    #[test]
    fn bad_fact_edits_leave_the_document_untouched() {
        let (p, mut doc) = facts_doc();
        for bad in [("x", 70_000), ("x", i64::MAX), ("", 1), ("\u{141}", 1)] {
            let e = Edits { facts: vec![("fine".into(), 1), (bad.0.into(), bad.1)], ..Default::default() };
            assert!(doc.apply(&e).is_err(), "{bad:?}");
            assert_eq!(doc.to_bytes(), p, "{bad:?}");
        }
    }

    #[test]
    fn rebase_refuses_a_layout_it_does_not_know() {
        let (_, mut doc) = facts_doc();
        // RB names that no longer match the top-level blocks.
        let manu = doc
            .regions
            .iter()
            .position(|r| matches!(r, Region::Variable { node, .. } if matches!(node.kind, Kind::Manu(_))))
            .unwrap();
        let Region::Gap(rb) = &mut doc.regions[manu - 1] else { panic!("no RB gap") };
        rb[6] = 9;
        assert!(matches!(doc.rebase(), Err(Error::Unverified(_))));
        // The tail was altered (an extra byte before the footer).
        let (_, mut doc) = facts_doc();
        let Region::Gap(tail) = doc.regions.last_mut().unwrap() else { panic!("no tail") };
        tail.insert(10, 0);
        assert!(matches!(doc.rebase(), Err(Error::Unverified(_))));
        // A document without a facts section cannot be edited.
        let p = payload::testutil::synthetic(3084);
        assert!(Document::parse(&p, 3084).unwrap().facts_table().is_err());
    }
}
