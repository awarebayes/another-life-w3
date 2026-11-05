//! Layer 3a: the typed token tree (Phase 0, step 7).
//!
//! The variable table (see `payload`) lists every serialized value of the save as an
//! `(offset, size)` span. Spans NEST: a `BS` block contains its children, a few `VL`/`OP`
//! values contain untagged sub-values (strings, GUIDs inside arrays of structs). This
//! module turns those spans into a tree of [`Node`]s and back into bytes.
//!
//! What the tags mean, established from real 5.01 saves (docs/research.md, 2026-11-04,
//! "typed tokenizer"); every shape below is checked at parse time and anything that does
//! not match falls back to [`Kind::Raw`], which round-trips as bytes. A rare or unfamiliar
//! token never fails the whole save; it is counted in [`TokenStats`] with the reason.
//!
//! ```text
//! BS  : "BS" u16 name                       then children, tiling the rest of the span exactly
//! VL  : "VL" u16 name u16 type value        value width depends on the type
//! OP  : "OP" u16 name u16 type value        same layout as VL
//! SS  : "SS" u32 len body[len]              len == span size - 6
//! MANU: "MANU" u32 count u32 0 names.. u32 0 "ENOD"   the NM name table (payload::read_names)
//! ```
//! `name` and `type` are 1-based indices into the NM name table.
//!
//! Nesting and sizes. Every node knows its own encoded size ([`Node::encoded_len`]), so the
//! variable table can be recomputed from the tree ([`crate::Document::table`]) once a value
//! grows. The only length field that depends on a child is the `SS` length: it is NOT stored,
//! it is derived from the body when writing, so growing an `SS` body needs no fix-up of that
//! field (the table entries of everything after it still need `rebase`).
//!
//! The structure follows Atvaark's W3SavegameEditor (MIT licence,
//! <https://github.com/Atvaark/W3SavegameEditor>, `VariableParsers/`): its `VlVariableParser`
//! reads a 1-based name index and type index and then a value chosen by type name, its
//! `SsVariableParser` an `Int32` inner length followed by nested variables, and its
//! `VariableParserBase.ReadValue` lists the fixed widths used in [`fixed_len`]. Differences
//! found on real 5.01 bytes: `BS` carries its children inline (they are separate table
//! entries here), and `SS` bodies are not nested variables but a `SXAP` block stream.
use std::collections::BTreeMap;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::facts::FactTable;
use crate::packed;
use crate::payload;
use crate::reader::Reader;

/// Real saves nest 26 levels deep. Anything past this is a damaged or hostile table.
pub const MAX_DEPTH: usize = 256;

/// Bytes that no table entry covers, or a nested table entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Bytes(Vec<u8>),
    Node(Node),
}

/// One variable-table entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Position of the entry in the on-disk variable table (the table is unordered).
    pub index: usize,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Block(Block),
    Var(Var),
    Sized(Sized),
    Manu(Manu),
    Raw(Raw),
}

/// `BS`: a named group. Children tile the span after the 4-byte header with no gap.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub name: u16,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum VarTag {
    Vl,
    Op,
}

impl VarTag {
    pub fn as_bytes(self) -> &'static [u8; 2] {
        match self {
            VarTag::Vl => b"VL",
            VarTag::Op => b"OP",
        }
    }
}

/// `VL` / `OP`: a typed value.
#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    pub tag: VarTag,
    pub name: u16,
    pub ty: u16,
    pub value: Value,
}

/// A decoded value. Anything not listed is [`Value::Opaque`]: bytes (plus any untagged
/// sub-values nested inside them) that round-trip but are not interpreted.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    /// IEEE-754 bit pattern, so NaN payloads survive.
    Float(u32),
    Double(u64),
    /// `CName`: 1-based index into the NM name table, 0 = empty.
    CName(u16),
    /// `EJournalStatus`, `EZoneName`, `EDifficultyMode`, `eGwintFaction`: a u16 (the
    /// `EZoneName` samples are NM indices). HYPOTHESIS: all four are name indices.
    Enum(u16),
    Guid([u8; 16]),
    /// Packed string; only produced when re-encoding gives the identical bytes.
    Str(String),
    Opaque(Vec<Part>),
}

/// `SS`: size-prefixed section. The length is derived from the body on write, so a body that
/// grows (a new fact) needs no fix-up of the length field.
///
/// Body layout seen on every `SS` with data: `SXAP` + three u32 (67, 29, 164 on 5.01, the same
/// three ints as the `SAV3` header) + content. The content of the one `SS` under the `facts`
/// block is the fact table ([`FactsBlock`]); the other `SS` bodies (`BLCK`/`AVAL`/`PORP`
/// streams, entity data) are kept as [`SsContent::Blob`] until someone needs them.
#[derive(Debug, Clone, PartialEq)]
pub struct Sized {
    /// `None` when the body does not start with `SXAP` + 3 ints (an empty `SS`, say); then
    /// `content` is a blob holding the whole body.
    pub sxap: Option<Sxap>,
    pub content: SsContent,
}

/// The three u32 after the `SXAP` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Sxap {
    pub ints: [u32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub enum SsContent {
    Facts(FactsBlock),
    /// Everything after the `SXAP` header, uninterpreted (plus any nested table entries).
    Blob(Vec<Part>),
}

impl Sized {
    /// Serialized body: what the `SS` length field counts.
    pub fn body_len(&self) -> usize {
        self.sxap.map_or(0, |_| 16) + self.content.encoded_len()
    }

    /// The body bytes (without the `SS` tag and length), for tools that want to read them.
    pub fn body_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.body_len());
        self.write_body(&mut v);
        v
    }

    pub(crate) fn write_body(&self, out: &mut Vec<u8>) {
        if let Some(x) = &self.sxap {
            out.extend_from_slice(b"SXAP");
            for i in x.ints {
                out.extend_from_slice(&i.to_le_bytes());
            }
        }
        self.content.write(out);
    }

    pub fn facts(&self) -> Option<&FactsBlock> {
        match &self.content {
            SsContent::Facts(f) => Some(f),
            SsContent::Blob(_) => None,
        }
    }

    pub fn facts_mut(&mut self) -> Option<&mut FactsBlock> {
        match &mut self.content {
            SsContent::Facts(f) => Some(f),
            SsContent::Blob(_) => None,
        }
    }

    /// Replace the fact list with `table`. Only valid on a facts section whose `SXAP` header
    /// equals the table's (so the version ints can never silently change). The `SS` length
    /// follows by itself when written.
    pub fn set_facts(&mut self, table: &FactTable) -> Result<()> {
        let same_header = self.sxap.is_some_and(|x| x.ints == table.sxap.map(|i| i as u32));
        match (&mut self.content, same_header) {
            (SsContent::Facts(f), true) => {
                *f = FactsBlock::from(table);
                Ok(())
            }
            _ => Err(Error::Payload("not a facts section with a matching SXAP header".into())),
        }
    }
}

impl Sized {
    /// The bytes after the `SXAP` header of a blob section, when the blob holds plain bytes only
    /// (no nested table entries). `None` for the facts section or a blob with nested nodes.
    pub fn blob_bytes(&self) -> Option<Vec<u8>> {
        self.sxap?;
        let SsContent::Blob(parts) = &self.content else { return None };
        let mut out = Vec::new();
        for p in parts {
            match p {
                Part::Bytes(b) => out.extend_from_slice(b),
                Part::Node(_) => return None,
            }
        }
        Some(out)
    }

    /// Replace the bytes after the `SXAP` header (the header itself stays). Only valid for a blob
    /// that holds plain bytes (see [`Sized::blob_bytes`]); the `SS` length follows by itself.
    pub fn set_blob_bytes(&mut self, rest: Vec<u8>) -> Result<()> {
        if self.blob_bytes().is_none() {
            return Err(Error::Payload("not a plain-bytes blob section with an SXAP header".into()));
        }
        self.content = SsContent::Blob(vec![Part::Bytes(rest)]);
        Ok(())
    }
}

impl SsContent {
    fn encoded_len(&self) -> usize {
        match self {
            SsContent::Facts(f) => f.encoded_len(),
            SsContent::Blob(p) => parts_len(p),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            SsContent::Facts(f) => f.write(out),
            SsContent::Blob(p) => write_parts(p, out),
        }
    }
}

/// The fact table inside the facts `SS`:
///
/// ```text
/// "SBDF" u32 fact_count   fact_count x fact   "EBDF"
/// fact = packed string id, u16 counter, u16 n, n x record(10 bytes)
/// record = u16 value, f32 time, f32 expiry      (expiry -1.0 = permanent)
/// ```
/// Layout from the read-only research on 49 saves (coordinator note, 2026-11-04) and
/// confirmed by byte-exact round trip here. The meaning of `counter` is inferred only; it is
/// kept verbatim. Ids are Latin-1 and may contain spaces and 0x1f. This type is deliberately
/// dumb: interpreting facts (which id is which quest) is `facts.rs`'s job.
#[derive(Debug, Clone, PartialEq)]
pub struct FactsBlock {
    pub facts: Vec<FactEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FactEntry {
    pub id: String,
    /// Unknown meaning (maybe the number of records that carry an expiry).
    pub counter: u16,
    pub records: Vec<FactValueRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactValueRecord {
    /// HYPOTHESIS: this is the fact's value (u16, not i32).
    pub value: u16,
    /// f32 bit pattern.
    pub time: u32,
    /// f32 bit pattern; -1.0 on permanent facts.
    pub expiry: u32,
}

impl FactValueRecord {
    pub fn time_f32(&self) -> f32 {
        f32::from_bits(self.time)
    }
    pub fn expiry_f32(&self) -> f32 {
        f32::from_bits(self.expiry)
    }
}

impl From<&FactTable> for FactsBlock {
    fn from(t: &FactTable) -> Self {
        let facts = t
            .facts()
            .iter()
            .map(|f| FactEntry {
                id: f.name.clone(),
                counter: f.c1,
                records: f
                    .records
                    .iter()
                    .map(|r| FactValueRecord { value: r.value, time: r.time.to_bits(), expiry: r.expiry.to_bits() })
                    .collect(),
            })
            .collect();
        Self { facts }
    }
}

fn packed_len(s: &str) -> usize {
    let mut v = Vec::new();
    packed::write(&mut v, s);
    v.len()
}

impl FactsBlock {
    pub fn encoded_len(&self) -> usize {
        4 + 4 + self.facts.iter().map(|f| packed_len(&f.id) + 4 + f.records.len() * 10).sum::<usize>() + 4
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"SBDF");
        out.extend_from_slice(&(self.facts.len() as u32).to_le_bytes());
        for f in &self.facts {
            packed::write(out, &f.id);
            out.extend_from_slice(&f.counter.to_le_bytes());
            out.extend_from_slice(&(f.records.len() as u16).to_le_bytes());
            for r in &f.records {
                out.extend_from_slice(&r.value.to_le_bytes());
                out.extend_from_slice(&r.time.to_le_bytes());
                out.extend_from_slice(&r.expiry.to_le_bytes());
            }
        }
        out.extend_from_slice(b"EBDF");
    }

    /// Parse `SBDF .. EBDF` spanning exactly `data`; `None` if anything is off (so the caller
    /// keeps the bytes as a blob instead).
    fn parse(data: &[u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        r.expect_tag(b"SBDF").ok()?;
        let count = r.u32().ok()? as usize;
        // Each fact needs at least an id byte and 4 counter bytes.
        if count > r.remaining() / 5 {
            return None;
        }
        let mut facts = Vec::with_capacity(count);
        for _ in 0..count {
            let at = r.pos();
            let id = packed::read(&mut r).ok()?;
            if data[at..r.pos()] != packed_bytes(&id)[..] {
                return None; // not the canonical encoding of this id: keep raw bytes
            }
            let counter = r.u16().ok()?;
            let n = r.u16().ok()? as usize;
            if n * 10 > r.remaining() {
                return None;
            }
            let mut records = Vec::with_capacity(n);
            for _ in 0..n {
                records.push(FactValueRecord { value: r.u16().ok()?, time: r.u32().ok()?, expiry: r.u32().ok()? });
            }
            facts.push(FactEntry { id, counter, records });
        }
        r.expect_tag(b"EBDF").ok()?;
        (r.remaining() == 0).then_some(Self { facts })
    }
}

fn packed_bytes(s: &str) -> Vec<u8> {
    let mut v = Vec::new();
    packed::write(&mut v, s);
    v
}

/// `MANU`: the NM name table, as raw one-byte-length Latin-1 names.
#[derive(Debug, Clone, PartialEq)]
pub struct Manu {
    /// Zero on all 73 saves; kept so any other value round-trips.
    pub unknown_a: u32,
    pub names: Vec<Vec<u8>>,
    pub unknown_b: u32,
}

/// Fallback: unknown tag, untagged sub-value, or a known tag whose shape did not match.
#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    /// Leading tag as `payload::tag_of` sees it (`?` when there is none).
    pub tag: String,
    pub why: &'static str,
    pub parts: Vec<Part>,
}

/// Width of a value whose layout is fixed, from Atvaark's `ReadValue` and confirmed by the
/// body sizes on real saves (`EngineTime` 3, `Vector` 35, `EulerAngles` 27, ...).
pub fn fixed_len(ty: &str) -> Option<usize> {
    Some(match ty {
        "Bool" | "Uint8" | "Int8" => 1,
        "Uint16" | "Int16" | "EJournalStatus" | "EZoneName" | "EDifficultyMode" | "eGwintFaction" | "CName" => 2,
        "Uint32" | "Int32" | "Float" => 4,
        "Uint64" | "Int64" | "Double" => 8,
        "CGUID" => 16,
        "EngineTime" => 3,
        "GameTime" => 11,
        "Vector2" => 19,
        "Vector" => 35,
        "EulerAngles" => 27,
        "IdTag" => 17,
        _ => return None,
    })
}

fn decode(ty: &str, body: &[u8]) -> Option<Value> {
    if ty == "String" {
        let mut r = Reader::new(body);
        let s = packed::read(&mut r).ok()?;
        if r.remaining() != 0 {
            return None;
        }
        let mut again = Vec::with_capacity(body.len());
        packed::write(&mut again, &s);
        return (again == body).then_some(Value::Str(s));
    }
    if fixed_len(ty)? != body.len() {
        return None;
    }
    Some(match ty {
        "Bool" => match body[0] {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            _ => return None,
        },
        "Uint8" => Value::U8(body[0]),
        "Int8" => Value::I8(body[0] as i8),
        "Uint16" => Value::U16(u16::from_le_bytes(body.try_into().ok()?)),
        "Int16" => Value::I16(i16::from_le_bytes(body.try_into().ok()?)),
        "Uint32" => Value::U32(u32::from_le_bytes(body.try_into().ok()?)),
        "Int32" => Value::I32(i32::from_le_bytes(body.try_into().ok()?)),
        "Float" => Value::Float(u32::from_le_bytes(body.try_into().ok()?)),
        "Uint64" => Value::U64(u64::from_le_bytes(body.try_into().ok()?)),
        "Int64" => Value::I64(i64::from_le_bytes(body.try_into().ok()?)),
        "Double" => Value::Double(u64::from_le_bytes(body.try_into().ok()?)),
        "CName" => Value::CName(u16::from_le_bytes(body.try_into().ok()?)),
        "EJournalStatus" | "EZoneName" | "EDifficultyMode" | "eGwintFaction" => {
            Value::Enum(u16::from_le_bytes(body.try_into().ok()?))
        }
        "CGUID" => Value::Guid(body.try_into().ok()?),
        _ => return None,
    })
}

fn parts_len(parts: &[Part]) -> usize {
    parts
        .iter()
        .map(|p| match p {
            Part::Bytes(b) => b.len(),
            Part::Node(n) => n.encoded_len(),
        })
        .sum()
}

fn write_parts(parts: &[Part], out: &mut Vec<u8>) {
    for p in parts {
        match p {
            Part::Bytes(b) => out.extend_from_slice(b),
            Part::Node(n) => n.write(out),
        }
    }
}

impl Value {
    pub fn encoded_len(&self) -> usize {
        match self {
            Value::Bool(_) | Value::U8(_) | Value::I8(_) => 1,
            Value::U16(_) | Value::I16(_) | Value::CName(_) | Value::Enum(_) => 2,
            Value::U32(_) | Value::I32(_) | Value::Float(_) => 4,
            Value::U64(_) | Value::I64(_) | Value::Double(_) => 8,
            Value::Guid(_) => 16,
            Value::Str(s) => {
                let mut v = Vec::new();
                packed::write(&mut v, s);
                v.len()
            }
            Value::Opaque(p) => parts_len(p),
        }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        match self {
            Value::Bool(b) => out.push(*b as u8),
            Value::U8(v) => out.push(*v),
            Value::I8(v) => out.push(*v as u8),
            Value::U16(v) | Value::CName(v) | Value::Enum(v) => out.extend_from_slice(&v.to_le_bytes()),
            Value::I16(v) => out.extend_from_slice(&v.to_le_bytes()),
            Value::U32(v) | Value::Float(v) => out.extend_from_slice(&v.to_le_bytes()),
            Value::I32(v) => out.extend_from_slice(&v.to_le_bytes()),
            Value::U64(v) | Value::Double(v) => out.extend_from_slice(&v.to_le_bytes()),
            Value::I64(v) => out.extend_from_slice(&v.to_le_bytes()),
            Value::Guid(g) => out.extend_from_slice(g),
            Value::Str(s) => packed::write(out, s),
            Value::Opaque(p) => write_parts(p, out),
        }
    }
}

impl Node {
    /// Number of bytes this node serializes to. Every size in the variable table is one of
    /// these, so a changed value changes the table through this function and nothing else.
    pub fn encoded_len(&self) -> usize {
        match &self.kind {
            Kind::Block(b) => 4 + b.children.iter().map(Node::encoded_len).sum::<usize>(),
            Kind::Var(v) => 6 + v.value.encoded_len(),
            Kind::Sized(s) => 6 + s.body_len(),
            Kind::Manu(m) => 4 + 4 + 4 + m.names.iter().map(|n| 1 + n.len()).sum::<usize>() + 4 + 4,
            Kind::Raw(r) => parts_len(&r.parts),
        }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        match &self.kind {
            Kind::Block(b) => {
                out.extend_from_slice(b"BS");
                out.extend_from_slice(&b.name.to_le_bytes());
                for c in &b.children {
                    c.write(out);
                }
            }
            Kind::Var(v) => {
                out.extend_from_slice(v.tag.as_bytes());
                out.extend_from_slice(&v.name.to_le_bytes());
                out.extend_from_slice(&v.ty.to_le_bytes());
                v.value.write(out);
            }
            Kind::Sized(s) => {
                out.extend_from_slice(b"SS");
                out.extend_from_slice(&(s.body_len() as u32).to_le_bytes());
                s.write_body(out);
            }
            Kind::Manu(m) => {
                out.extend_from_slice(b"MANU");
                out.extend_from_slice(&(m.names.len() as u32).to_le_bytes());
                out.extend_from_slice(&m.unknown_a.to_le_bytes());
                for n in &m.names {
                    out.push(n.len() as u8);
                    out.extend_from_slice(n);
                }
                out.extend_from_slice(&m.unknown_b.to_le_bytes());
                out.extend_from_slice(b"ENOD");
            }
            Kind::Raw(r) => write_parts(&r.parts, out),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.encoded_len());
        self.write(&mut v);
        v
    }

    /// Short tag for reports: `BS VL OP SS MANU`, or the raw node's tag (`?` if untagged).
    pub fn tag(&self) -> &str {
        match &self.kind {
            Kind::Block(_) => "BS",
            Kind::Var(v) => std::str::from_utf8(v.tag.as_bytes()).unwrap_or("?"),
            Kind::Sized(_) => "SS",
            Kind::Manu(_) => "MANU",
            Kind::Raw(r) => &r.tag,
        }
    }

    /// Direct child nodes in file order (table entries nested immediately inside).
    pub fn children(&self) -> Vec<&Node> {
        fn from_parts(p: &[Part]) -> Vec<&Node> {
            p.iter()
                .filter_map(|x| match x {
                    Part::Node(n) => Some(n),
                    Part::Bytes(_) => None,
                })
                .collect()
        }
        match &self.kind {
            Kind::Block(b) => b.children.iter().collect(),
            Kind::Var(Var { value: Value::Opaque(p), .. }) => from_parts(p),
            Kind::Sized(Sized { content: SsContent::Blob(p), .. }) => from_parts(p),
            Kind::Raw(r) => from_parts(&r.parts),
            _ => Vec::new(),
        }
    }

    /// Mutable direct child nodes, same order as [`Node::children`].
    pub fn children_mut(&mut self) -> Vec<&mut Node> {
        fn from_parts(p: &mut [Part]) -> Vec<&mut Node> {
            p.iter_mut()
                .filter_map(|x| match x {
                    Part::Node(n) => Some(n),
                    Part::Bytes(_) => None,
                })
                .collect()
        }
        match &mut self.kind {
            Kind::Block(b) => b.children.iter_mut().collect(),
            Kind::Var(Var { value: Value::Opaque(p), .. }) => from_parts(p),
            Kind::Sized(Sized { content: SsContent::Blob(p), .. }) => from_parts(p),
            Kind::Raw(r) => from_parts(&mut r.parts),
            _ => Vec::new(),
        }
    }

    /// Give this node and its descendants table indices in post-order (children first, then
    /// the node), starting at `*next`. On every real save the on-disk table index of a node IS its
    /// post-order rank (PROVEN on 122 of 122 fixture files, docs/research.md 2026-11-04 "journal"),
    /// so after nodes are added this numbers the table the way the game writes it.
    pub fn renumber_post_order(&mut self, next: &mut usize) {
        for c in self.children_mut() {
            c.renumber_post_order(next);
        }
        self.index = *next;
        *next += 1;
    }

    /// True when the indices of this subtree are consecutive post-order ranks from `*next`.
    pub fn is_post_order(&self, next: &mut usize) -> bool {
        for c in self.children() {
            if !c.is_post_order(next) {
                return false;
            }
        }
        let ok = self.index == *next;
        *next += 1;
        ok
    }

    /// Visit this node and every descendant, outermost first, in file order.
    pub fn visit<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        for c in self.children() {
            c.visit(f);
        }
    }

    /// Number of table entries strictly inside this node.
    pub fn descendants(&self) -> usize {
        let mut n = 0usize;
        self.visit(&mut |_| n += 1);
        n - 1
    }

    /// Append `(table index, start, size)` for this node and all descendants, where `start` is
    /// relative to the payload and this node begins at `at`.
    pub fn layout(&self, at: usize, out: &mut Vec<(usize, usize, usize)>) {
        out.push((self.index, at, self.encoded_len()));
        let mut pos = at;
        match &self.kind {
            Kind::Block(b) => {
                pos += 4;
                for c in &b.children {
                    c.layout(pos, out);
                    pos += c.encoded_len();
                }
            }
            Kind::Var(Var { value: Value::Opaque(p), .. }) => layout_parts(p, pos + 6, out),
            Kind::Sized(Sized { sxap, content: SsContent::Blob(p) }) => layout_parts(p, pos + 6 + sxap.map_or(0, |_| 16), out),
            Kind::Sized(_) => {}
            Kind::Raw(r) => layout_parts(&r.parts, pos, out),
            Kind::Var(_) | Kind::Manu(_) => {}
        }
    }
}

fn layout_parts(parts: &[Part], mut pos: usize, out: &mut Vec<(usize, usize, usize)>) {
    for p in parts {
        match p {
            Part::Bytes(b) => pos += b.len(),
            Part::Node(n) => {
                n.layout(pos, out);
                pos += n.encoded_len();
            }
        }
    }
}

/// How many nodes parsed into typed form vs fell back to raw bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TagStats {
    pub total: usize,
    pub typed: usize,
    pub raw: usize,
    /// `VL`/`OP` only: the value was decoded to a scalar, name index, GUID or string.
    pub value_decoded: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TokenStats {
    pub total: usize,
    pub typed: usize,
    pub raw: usize,
    pub by_tag: BTreeMap<String, TagStats>,
    /// Why raw nodes are raw: `(tag, reason) -> count`.
    pub raw_reasons: BTreeMap<String, usize>,
    /// `VL`/`OP` values left opaque, by NM type name: the work list for the next decoders.
    pub opaque_types: BTreeMap<String, usize>,
    /// The `top_level` field of the tree: tags of the nodes the table tiles the payload with.
    pub top_level: BTreeMap<String, usize>,
    /// Entries per nesting depth (0 = top level).
    pub max_depth: usize,
}

impl TokenStats {
    pub fn typed_percent(&self) -> f64 {
        if self.total == 0 {
            100.0
        } else {
            self.typed as f64 * 100.0 / self.total as f64
        }
    }

    pub(crate) fn add(&mut self, node: &Node, names: Option<&[String]>, depth: usize) {
        self.max_depth = self.max_depth.max(depth);
        self.total += 1;
        let tag = node.tag().to_string();
        let t = self.by_tag.entry(tag.clone()).or_default();
        t.total += 1;
        match &node.kind {
            Kind::Raw(r) => {
                t.raw += 1;
                self.raw += 1;
                *self.raw_reasons.entry(format!("{tag}: {}", r.why)).or_default() += 1;
            }
            Kind::Var(v) => {
                t.typed += 1;
                self.typed += 1;
                if matches!(v.value, Value::Opaque(_)) {
                    let ty = names
                        .and_then(|n| n.get((v.ty as usize).wrapping_sub(1)))
                        .cloned()
                        .unwrap_or_else(|| "?".into());
                    *self.opaque_types.entry(ty).or_default() += 1;
                } else {
                    t.value_decoded += 1;
                }
            }
            _ => {
                t.typed += 1;
                self.typed += 1;
            }
        }
        for c in node.children() {
            self.add(c, names, depth + 1);
        }
    }
}

/// Sorted spans plus the nesting derived from them. `spans` is `(start, end, table index)`
/// sorted by start, outermost first.
pub(crate) struct Tree<'a> {
    payload: &'a [u8],
    names: Option<&'a [String]>,
    spans: Vec<(usize, usize, usize)>,
    children: Vec<Vec<u32>>,
    pub top: Vec<u32>,
}

impl<'a> Tree<'a> {
    pub fn new(payload: &'a [u8], names: Option<&'a [String]>, mut spans: Vec<(usize, usize, usize)>) -> Result<Self> {
        spans.sort_by_key(|&(start, end, _)| (start, std::cmp::Reverse(end)));
        let mut children: Vec<Vec<u32>> = vec![Vec::new(); spans.len()];
        let mut top = Vec::new();
        let mut stack: Vec<u32> = Vec::new();
        for k in 0..spans.len() {
            let (start, end, index) = spans[k];
            while let Some(&t) = stack.last() {
                if start >= spans[t as usize].1 {
                    stack.pop();
                } else {
                    break;
                }
            }
            match stack.last() {
                Some(&t) => {
                    if end > spans[t as usize].1 {
                        return Err(Error::Payload(format!(
                            "variable {index} (bytes {start}..{end}) crosses the end of its enclosing variable at {}",
                            spans[t as usize].1
                        )));
                    }
                    children[t as usize].push(k as u32);
                }
                None => top.push(k as u32),
            }
            stack.push(k as u32);
        }
        Ok(Self { payload, names, spans, children, top })
    }

    pub fn span(&self, k: u32) -> (usize, usize, usize) {
        self.spans[k as usize]
    }

    fn name_ok(&self, idx: u16) -> bool {
        idx != 0 && self.names.is_none_or(|n| (idx as usize) <= n.len())
    }

    /// Cut `[from, end)` into bytes and the nodes of `kids` (sorted, inside the range).
    fn parts(&self, from: usize, end: usize, kids: &[u32], depth: usize) -> Result<Vec<Part>> {
        let mut out = Vec::with_capacity(kids.len() * 2 + 1);
        let mut cur = from;
        for &k in kids {
            let (s, e, _) = self.span(k);
            if s > cur {
                out.push(Part::Bytes(self.payload[cur..s].to_vec()));
            }
            out.push(Part::Node(self.build(k, depth + 1)?));
            cur = e;
        }
        if end > cur {
            out.push(Part::Bytes(self.payload[cur..end].to_vec()));
        }
        Ok(out)
    }

    pub fn build(&self, k: u32, depth: usize) -> Result<Node> {
        if depth > MAX_DEPTH {
            return Err(Error::Payload(format!("variables nest deeper than {MAX_DEPTH} levels")));
        }
        let (s, e, index) = self.span(k);
        let data = &self.payload[s..e];
        let kids = &self.children[k as usize];
        let first_kid = kids.first().map(|&c| self.span(c).0);
        let kids_from = |at: usize| first_kid.is_none_or(|f| f >= s + at);

        let raw = |why: &'static str| -> Result<Node> {
            Ok(Node {
                index,
                kind: Kind::Raw(Raw {
                    tag: payload::tag_of(data),
                    why,
                    parts: self.parts(s, e, kids, depth)?,
                }),
            })
        };

        if data.starts_with(b"MANU") {
            if !kids.is_empty() {
                return raw("MANU with nested entries");
            }
            return match parse_manu(data) {
                Some(m) => Ok(Node { index, kind: Kind::Manu(m) }),
                None => raw("MANU body is not the NM layout"),
            };
        }
        match data.get(..2) {
            Some(b"BS") => {
                if data.len() < 4 {
                    return raw("BS shorter than its header");
                }
                let name = u16::from_le_bytes([data[2], data[3]]);
                if !self.name_ok(name) {
                    return raw("BS name index outside the NM table");
                }
                // Children must tile [s+4, e) exactly.
                let mut cur = s + 4;
                for &c in kids {
                    let (cs, ce, _) = self.span(c);
                    if cs != cur {
                        return raw("BS children do not tile its body");
                    }
                    cur = ce;
                }
                if cur != e {
                    return raw("BS children do not tile its body");
                }
                let children = kids
                    .iter()
                    .map(|&c| self.build(c, depth + 1))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Node { index, kind: Kind::Block(Block { name, children }) })
            }
            Some(t @ (b"VL" | b"OP")) => {
                if data.len() < 6 {
                    return raw("VL/OP shorter than its header");
                }
                let name = u16::from_le_bytes([data[2], data[3]]);
                let ty = u16::from_le_bytes([data[4], data[5]]);
                if !self.name_ok(name) || !self.name_ok(ty) {
                    return raw("VL/OP name or type index outside the NM table");
                }
                let tag = if t == b"VL" { VarTag::Vl } else { VarTag::Op };
                let value = if !kids.is_empty() {
                    if !kids_from(6) {
                        return raw("nested entry inside the VL/OP header");
                    }
                    Value::Opaque(self.parts(s + 6, e, kids, depth)?)
                } else {
                    let decoded = self
                        .names
                        .and_then(|n| n.get(ty as usize - 1))
                        .and_then(|tn| decode(tn, &data[6..]));
                    decoded.unwrap_or_else(|| Value::Opaque(vec![Part::Bytes(data[6..].to_vec())]))
                };
                Ok(Node { index, kind: Kind::Var(Var { tag, name, ty, value }) })
            }
            Some(b"SS") => {
                if data.len() < 6 {
                    return raw("SS shorter than its header");
                }
                let len = u32::from_le_bytes(data[2..6].try_into().unwrap()) as usize;
                if len != data.len() - 6 {
                    return raw("SS length field is not span size - 6");
                }
                if !kids_from(6) {
                    return raw("nested entry inside the SS header");
                }
                let body = &data[6..];
                let sxap = (body.len() >= 16 && body.starts_with(b"SXAP")).then(|| Sxap {
                    ints: [0, 4, 8].map(|i| u32::from_le_bytes(body[4 + i..8 + i].try_into().unwrap())),
                });
                let from = s + 6 + sxap.map_or(0, |_| 16);
                if !kids_from(from - s) {
                    return raw("nested entry inside the SXAP header");
                }
                let facts = if sxap.is_some() && kids.is_empty() { FactsBlock::parse(&data[from - s..]) } else { None };
                let content = match facts {
                    Some(f) => SsContent::Facts(f),
                    None => SsContent::Blob(self.parts(from, e, kids, depth)?),
                };
                Ok(Node { index, kind: Kind::Sized(Sized { sxap, content }) })
            }
            _ => raw(untagged_shape(data)),
        }
    }
}

/// Reason text for an entry with no recognised tag. These are table entries that point INTO a
/// parent value (a string or GUID inside an array of structs); naming their type needs the
/// parent struct's layout, so they stay raw. The shape is only a hint for the report.
fn untagged_shape(data: &[u8]) -> &'static str {
    let mut r = Reader::new(data);
    if let Ok(s) = packed::read(&mut r) {
        if r.remaining() == 0 && packed_bytes(&s) == data {
            return "untagged: packed-string shaped";
        }
    }
    if data.len() == 16 {
        "untagged: 16 bytes (GUID?)"
    } else {
        "untagged: other"
    }
}

/// `MANU` token bytes -> names, only when the layout matches exactly.
fn parse_manu(data: &[u8]) -> Option<Manu> {
    let mut r = Reader::new(data);
    r.expect_tag(b"MANU").ok()?;
    let count = r.u32().ok()? as usize;
    let unknown_a = r.u32().ok()?;
    if count > r.remaining() {
        return None;
    }
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let len = r.u8().ok()? as usize;
        names.push(r.bytes(len).ok()?.to_vec());
    }
    let unknown_b = r.u32().ok()?;
    r.expect_tag(b"ENOD").ok()?;
    (r.remaining() == 0).then_some(Manu { unknown_a, names, unknown_b })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["a", "b", "Uint32", "String", "CName", "Bool", "array:2,0,X"].iter().map(|s| s.to_string()).collect()
    }

    fn tree(p: &[u8], spans: Vec<(usize, usize, usize)>, names: &[String]) -> (Vec<Node>, Vec<u8>) {
        let t = Tree::new(p, Some(names), spans).unwrap();
        let nodes: Vec<Node> = t.top.iter().map(|&k| t.build(k, 0).unwrap()).collect();
        let mut out = Vec::new();
        for n in &nodes {
            n.write(&mut out);
        }
        (nodes, out)
    }

    #[test]
    fn block_with_values_round_trips_and_is_typed() {
        // BS name=1 { VL a:Uint32=7, VL b:Bool=1 }
        let mut p = b"BS\x01\x00".to_vec();
        p.extend_from_slice(b"VL\x01\x00\x03\x00\x07\x00\x00\x00");
        p.extend_from_slice(b"VL\x02\x00\x06\x00\x01");
        let n = names();
        let (nodes, out) = tree(&p, vec![(0, 4 + 10 + 7, 0), (4, 14, 1), (14, 21, 2)], &n);
        assert_eq!(out, p);
        let Kind::Block(b) = &nodes[0].kind else { panic!("not a block") };
        assert!(matches!(b.children[0].kind, Kind::Var(Var { value: Value::U32(7), .. })));
        assert!(matches!(b.children[1].kind, Kind::Var(Var { value: Value::Bool(true), .. })));
        assert_eq!(nodes[0].descendants(), 2);
        let mut lay = Vec::new();
        nodes[0].layout(0, &mut lay);
        assert_eq!(lay, [(0, 0, 21), (1, 4, 10), (2, 14, 7)]);
    }

    #[test]
    fn strings_decode_only_when_canonical() {
        let n = names();
        // ANSI "hi" is canonical; the same text as UTF-16 (bit 7 clear) is not what we would write.
        let mut ok = b"VL\x01\x00\x04\x00".to_vec();
        ok.extend_from_slice(b"\x82hi");
        let (nodes, out) = tree(&ok, vec![(0, ok.len(), 0)], &n);
        assert_eq!(out, ok);
        assert!(matches!(&nodes[0].kind, Kind::Var(Var { value: Value::Str(s), .. }) if s == "hi"));
        let mut wide = b"VL\x01\x00\x04\x00".to_vec();
        wide.extend_from_slice(b"\x02h\0i\0");
        let (nodes, out) = tree(&wide, vec![(0, wide.len(), 0)], &n);
        assert_eq!(out, wide);
        assert!(matches!(&nodes[0].kind, Kind::Var(Var { value: Value::Opaque(_), .. })));
    }

    #[test]
    fn ss_length_is_derived() {
        let n = names();
        let p = b"SS\x03\x00\x00\x00abc".to_vec();
        let (mut nodes, out) = tree(&p, vec![(0, p.len(), 0)], &n);
        assert_eq!(out, p);
        let Kind::Sized(Sized { content: SsContent::Blob(p), .. }) = &mut nodes[0].kind else { panic!("not an SS") };
        p.push(Part::Bytes(b"de".to_vec()));
        assert_eq!(nodes[0].encoded_len(), 11);
        assert_eq!(&nodes[0].to_bytes()[2..6], &5u32.to_le_bytes());
    }

    /// Real bytes from a 5.01 early-game save (first facts of the `facts` SS, count patched to 2).
    fn facts_ss() -> Vec<u8> {
        let mut body = b"SXAP".to_vec();
        for i in [67u32, 29, 164] {
            body.extend_from_slice(&i.to_le_bytes());
        }
        body.extend_from_slice(b"SBDF\x02\x00\x00\x00");
        body.extend_from_slice(b"\x8dbugfix_132404\x00\x00\x01\x00\x01\x00\x8d\x46\x8d\x44\x00\x00\x80\xbf");
        body.extend_from_slice(b"\x97tutorial_journal_opened\x02\x00\x02\x00");
        body.extend_from_slice(b"\x01\x00\xcd\x62\x8e\x44\xcd\x82\x8e\x44\x01\x00\xcd\x62\x8e\x44\xcd\x82\x8e\x44EBDF");
        let mut p = b"SS".to_vec();
        p.extend_from_slice(&(body.len() as u32).to_le_bytes());
        p.extend_from_slice(&body);
        p
    }

    #[test]
    fn facts_ss_is_typed_round_trips_and_length_follows_edits() {
        let n = names();
        let p = facts_ss();
        let (mut nodes, out) = tree(&p, vec![(0, p.len(), 0)], &n);
        assert_eq!(out, p);
        let Kind::Sized(s) = &mut nodes[0].kind else { panic!("not an SS") };
        assert_eq!(s.sxap, Some(Sxap { ints: [67, 29, 164] }));
        let f = s.facts_mut().expect("facts recognised");
        assert_eq!(f.facts.len(), 2);
        assert_eq!(f.facts[0].id, "bugfix_132404");
        assert_eq!((f.facts[0].counter, f.facts[0].records.len()), (0, 1));
        assert_eq!(f.facts[0].records[0].value, 1);
        assert_eq!(f.facts[0].records[0].expiry_f32(), -1.0);
        assert_eq!((f.facts[1].counter, f.facts[1].records.len()), (2, 2));
        // Grow the table: the SS length field is derived, the node reports its new size.
        let rec = f.facts[0].records[0];
        f.facts.push(FactEntry { id: "new_fact".into(), counter: 0, records: vec![rec] });
        let grown = nodes[0].to_bytes();
        assert_eq!(grown.len(), p.len() + 1 + 8 + 4 + 10);
        assert_eq!(nodes[0].encoded_len(), grown.len());
        assert_eq!(u32::from_le_bytes(grown[2..6].try_into().unwrap()) as usize, grown.len() - 6);
    }

    #[test]
    fn damaged_fact_tables_stay_blobs() {
        let n = names();
        let good = facts_ss();
        // Drop EBDF, lie about the count, or leave a trailing byte: raw blob, same bytes back.
        for mutate in [0usize, 1, 2] {
            let mut p = good.clone();
            match mutate {
                0 => { let l = p.len(); p[l - 1] = b'X'; }
                1 => p[6 + 16 + 4] = 9,
                _ => { p.push(0); let l = (p.len() - 6) as u32; p[2..6].copy_from_slice(&l.to_le_bytes()); }
            }
            let (nodes, out) = tree(&p, vec![(0, p.len(), 0)], &n);
            assert_eq!(out, p);
            let Kind::Sized(s) = &nodes[0].kind else { panic!("not an SS") };
            assert!(s.facts().is_none(), "case {mutate}");
        }
    }

    #[test]
    fn mismatches_fall_back_to_raw_and_still_round_trip() {
        let n = names();
        // SS with a wrong length, VL with an out-of-range type, BS with a gap, unknown tag, untagged.
        let cases: Vec<Vec<u8>> = vec![
            b"SS\x09\x00\x00\x00abc".to_vec(),
            b"VL\x01\x00\x63\x00xyz".to_vec(),
            b"BS\x01\x00zz".to_vec(),
            b"ZZ\x01\x00\x03\x00".to_vec(),
            b"\x01\x02\x03".to_vec(),
            b"MANU\x01".to_vec(),
            b"VL".to_vec(),
        ];
        for p in cases {
            let (nodes, out) = tree(&p, vec![(0, p.len(), 0)], &n);
            assert_eq!(out, p);
            assert!(matches!(nodes[0].kind, Kind::Raw(_)), "{:?}", p);
        }
    }

    #[test]
    fn untagged_children_inside_a_value_stay_in_the_tree() {
        let n = names();
        // VL a:array = u32 count, then a nested untagged entry (bytes 10..13) and a trailing byte.
        let mut p = b"VL\x01\x00\x07\x00".to_vec();
        p.extend_from_slice(b"\x01\x00\x00\x00\x82hi\x09");
        let (nodes, out) = tree(&p, vec![(0, p.len(), 0), (10, 13, 1)], &n);
        assert_eq!(out, p);
        assert_eq!(nodes[0].descendants(), 1);
        let mut lay = Vec::new();
        nodes[0].layout(100, &mut lay);
        assert_eq!(lay, [(0, 100, p.len()), (1, 110, 3)]);
    }

    #[test]
    fn crossing_spans_and_runaway_nesting_are_errors() {
        let p = vec![0u8; 400];
        assert!(Tree::new(&p, None, vec![(0, 10, 0), (5, 15, 1)]).is_err());
        let spans: Vec<_> = (0..MAX_DEPTH + 5).map(|i| (i, 400, i)).collect();
        let t = Tree::new(&p, None, spans).unwrap();
        assert!(t.build(t.top[0], 0).is_err());
    }

    #[test]
    fn manu_round_trips() {
        let mut p = b"MANU".to_vec();
        p.extend_from_slice(&2u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(b"\x03abc\x02d\xe9");
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(b"ENOD");
        let (nodes, out) = tree(&p, vec![(0, p.len(), 0)], &[]);
        assert_eq!(out, p);
        assert!(matches!(&nodes[0].kind, Kind::Manu(m) if m.names.len() == 2));
    }
}
