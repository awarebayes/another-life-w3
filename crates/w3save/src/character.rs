//! The player character: level, experience, ability points, max vitality, crowns and the item list.
//!
//! Where it lives (all of this was read from real 5.01 saves and from the game's scripts; see
//! `docs/findings/character-inventory.md`, labels `[proven]` / `[hypothesis]` there):
//!
//! * the player is the `entityData` of `universe > world > layerStorage` whose `idTag` is
//!   `01 00*12 01 00 00 00`; its `SS` body is `SXAP` + a record stream;
//! * the stream is `BLCK u16 name, u32 size, records` / `AVAL|PORP u16 name, u16 type, u32 size, value`
//!   (the same grammar as `graph::remap_records`), top level `BLCK Entity { nP, PORP .. }`, `BLCK stats`,
//!   `BLCK timers`, `BLCK ai`, `BLCK at`, `BLCK tags`, `AVAL a`, then a raw tail that is the native
//!   `CInventoryComponent` data (the item list);
//! * a `PORP` of type `handle:<Class>` holds an object: `00 01 u32 id`, `u16 class`, `u8 0`, then
//!   properties `u16 name, u16 type, u32 size (counting itself), value`, then `u16 0`;
//! * arrays of structs: `u32 count`, each struct `00`, fields (`u16 name, u16 type, u32 size, value`),
//!   `u16 0`. The game leaves out fields whose value is 0 (`SSpendablePoints.free = 0` is absent);
//! * `W3LevelManager` (`levelManager`): `levelDefinitions` (the XP table, in the save itself), `level`,
//!   `points[ESkillPoint, EExperiencePoint]` (`free` / `used`), `lastCustomLevel`.
//!
//! Everything that depends on an unconfirmed detail carries `HYPOTHESIS:`; what cannot be done from the
//! evidence returns [`Error::Unverified`].
use crate::document::Document;
use crate::error::{Error, Result};
use crate::token::{Kind, Node, Value};

/// `IdTag` of the player entity: `01`, twelve zero bytes, `01 00 00 00` (proven on every fixture).
pub const PLAYER_ID: [u8; 17] = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
/// `ESpendablePointType` order (script `levelTypes.ws`).
pub const SKILL_POINT: usize = 0;
pub const EXPERIENCE_POINT: usize = 1;

fn le16(b: &[u8], i: usize) -> Result<u16> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| Error::Payload("character: short read".into()))
}
fn le32(b: &[u8], i: usize) -> Result<u32> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| Error::Payload("character: short read".into()))
}
fn unv<T>(why: impl Into<String>) -> Result<T> {
    Err(Error::Unverified(format!("character: {}", why.into())))
}

// ---------------------------------------------------------------------------------------------
// Record stream
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Rec {
    Block { name: u16, kids: Vec<Rec> },
    /// `AVAL` (`porp == false`) or `PORP` (`true`).
    Val { porp: bool, name: u16, ty: u16, data: Vec<u8> },
}

/// The record stream of the player entity (everything after `SXAP` + 3 ints) plus the raw tail.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub recs: Vec<Rec>,
    pub tail: Vec<u8>,
    /// First `SXAP` int of the section (64, 66 or 67): the item-record layout depends on it.
    pub version: u32,
}

fn tag_at(b: &[u8], i: usize) -> Option<&[u8]> {
    let t = b.get(i..i + 4)?;
    matches!(t, b"BLCK" | b"AVAL" | b"PORP").then_some(t)
}

fn parse_recs(b: &[u8], top: bool) -> Result<(Vec<Rec>, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(tag) = tag_at(b, i) else {
            if top {
                break;
            }
            return Err(Error::Payload(format!("character: unknown record tag inside a block at {i}")));
        };
        if tag == b"BLCK" {
            let name = le16(b, i + 4)?;
            let size = le32(b, i + 6)? as usize;
            let inner = b.get(i + 10..i + 10 + size).ok_or_else(|| Error::Payload("character: BLCK overruns its stream".into()))?;
            let (kids, used) = parse_recs(inner, false)?;
            debug_assert_eq!(used, size);
            out.push(Rec::Block { name, kids });
            i += 10 + size;
        } else {
            let (name, ty, size) = (le16(b, i + 4)?, le16(b, i + 6)?, le32(b, i + 8)? as usize);
            let data = b.get(i + 12..i + 12 + size).ok_or_else(|| Error::Payload("character: value overruns its stream".into()))?;
            out.push(Rec::Val { porp: tag == b"PORP", name, ty, data: data.to_vec() });
            i += 12 + size;
        }
    }
    Ok((out, i))
}

fn write_recs(recs: &[Rec], out: &mut Vec<u8>) {
    for r in recs {
        match r {
            Rec::Block { name, kids } => {
                let mut inner = Vec::new();
                write_recs(kids, &mut inner);
                out.extend_from_slice(b"BLCK");
                out.extend_from_slice(&name.to_le_bytes());
                out.extend_from_slice(&(inner.len() as u32).to_le_bytes());
                out.extend_from_slice(&inner);
            }
            Rec::Val { porp, name, ty, data } => {
                out.extend_from_slice(if *porp { b"PORP" } else { b"AVAL" });
                out.extend_from_slice(&name.to_le_bytes());
                out.extend_from_slice(&ty.to_le_bytes());
                out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                out.extend_from_slice(data);
            }
        }
    }
}

impl Stream {
    pub fn parse(rest: &[u8]) -> Result<Self> {
        let (recs, used) = parse_recs(rest, true)?;
        Ok(Self { recs, tail: rest[used..].to_vec(), version: 0 })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_recs(&self.recs, &mut out);
        out.extend_from_slice(&self.tail);
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Handle objects and struct arrays
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    pub name: u16,
    pub ty: u16,
    pub value: Vec<u8>,
}

/// The value of a `handle:<Class>` property that holds an object (see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct Obj {
    pub head: [u8; 6],
    pub class: u16,
    pub flag: u8,
    pub props: Vec<Prop>,
}

impl Obj {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < 11 {
            return unv("a handle object is shorter than its header");
        }
        let mut head = [0u8; 6];
        head.copy_from_slice(&b[..6]);
        let (class, flag) = (le16(b, 6)?, b[8]);
        let mut props = Vec::new();
        let mut i = 9;
        loop {
            let name = le16(b, i)?;
            if name == 0 {
                if i + 2 != b.len() {
                    return unv("data after the end of a handle object");
                }
                break;
            }
            let (ty, size) = (le16(b, i + 2)?, le32(b, i + 4)? as usize);
            if size < 4 || i + 4 + size > b.len() {
                return unv("a handle object property overruns the object");
            }
            props.push(Prop { name, ty, value: b[i + 8..i + 4 + size].to_vec() });
            i += 4 + size;
        }
        Ok(Self { head, class, flag, props })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.head);
        out.extend_from_slice(&self.class.to_le_bytes());
        out.push(self.flag);
        for p in &self.props {
            out.extend_from_slice(&p.name.to_le_bytes());
            out.extend_from_slice(&p.ty.to_le_bytes());
            out.extend_from_slice(&((p.value.len() + 4) as u32).to_le_bytes());
            out.extend_from_slice(&p.value);
        }
        out.extend_from_slice(&[0, 0]);
        out
    }

    pub fn prop(&self, name: u16) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }
}

/// One struct of a struct array: its fields in file order.
pub type Fields = Vec<Prop>;

pub fn parse_struct_array(b: &[u8]) -> Result<Vec<Fields>> {
    let n = le32(b, 0)? as usize;
    let mut i = 4;
    let mut out = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        if b.get(i) != Some(&0) {
            return unv("a struct in an array does not start with 00");
        }
        i += 1;
        let mut f = Vec::new();
        loop {
            let name = le16(b, i)?;
            if name == 0 {
                i += 2;
                break;
            }
            let (ty, size) = (le16(b, i + 2)?, le32(b, i + 4)? as usize);
            if size < 4 || i + 4 + size > b.len() {
                return unv("a struct field overruns its array");
            }
            f.push(Prop { name, ty, value: b[i + 8..i + 4 + size].to_vec() });
            i += 4 + size;
        }
        out.push(f);
    }
    if i != b.len() {
        return unv("data after the last struct of an array");
    }
    Ok(out)
}

pub fn write_struct_array(items: &[Fields]) -> Vec<u8> {
    let mut out = (items.len() as u32).to_le_bytes().to_vec();
    for f in items {
        out.push(0);
        for p in f {
            out.extend_from_slice(&p.name.to_le_bytes());
            out.extend_from_slice(&p.ty.to_le_bytes());
            out.extend_from_slice(&((p.value.len() + 4) as u32).to_le_bytes());
            out.extend_from_slice(&p.value);
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Locating the player entity in a document
// ---------------------------------------------------------------------------------------------

fn opaque_bytes(v: &Value) -> Option<Vec<u8>> {
    use crate::token::Part;
    let Value::Opaque(parts) = v else { return None };
    let mut out = Vec::new();
    for p in parts {
        match p {
            Part::Bytes(b) => out.extend_from_slice(b),
            Part::Node(_) => return None,
        }
    }
    Some(out)
}

fn block_name<'a>(doc: &'a Document, n: &Node) -> Option<&'a str> {
    match &n.kind {
        Kind::Block(b) => doc.name(b.name),
        _ => None,
    }
}

fn is_player_entity(doc: &Document, e: &Node) -> bool {
    let Kind::Block(b) = &e.kind else { return false };
    b.children.iter().any(|c| match &c.kind {
        Kind::Var(v) => doc.name(v.name) == Some("idTag") && opaque_bytes(&v.value).is_some_and(|x| x == PLAYER_ID),
        _ => false,
    })
}

/// Path (top index, child indices) of the player's `entityData` block, `Err` unless exactly one.
fn player_path(doc: &Document) -> Result<Vec<usize>> {
    let mut found: Vec<Vec<usize>> = Vec::new();
    for (ti, top) in doc.top_nodes().enumerate() {
        if block_name(doc, top) != Some("universe") {
            continue;
        }
        let Kind::Block(u) = &top.kind else { continue };
        for (wi, w) in u.children.iter().enumerate() {
            if block_name(doc, w) != Some("world") {
                continue;
            }
            let Kind::Block(wb) = &w.kind else { continue };
            for (li, l) in wb.children.iter().enumerate() {
                if block_name(doc, l) != Some("layerStorage") {
                    continue;
                }
                let Kind::Block(lb) = &l.kind else { continue };
                for (ei, e) in lb.children.iter().enumerate() {
                    if block_name(doc, e) == Some("entityData") && is_player_entity(doc, e) {
                        found.push(vec![ti, wi, li, ei]);
                    }
                }
            }
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        n => unv(format!("expected exactly one player entityData, found {n}")),
    }
}

fn node_at<'a>(doc: &'a Document, path: &[usize]) -> &'a Node {
    let mut n = doc.top_nodes().nth(path[0]).expect("path from player_path");
    for &i in &path[1..] {
        let Kind::Block(b) = &n.kind else { unreachable!() };
        n = &b.children[i];
    }
    n
}

fn node_at_mut<'a>(doc: &'a mut Document, path: &[usize]) -> &'a mut Node {
    let mut n = doc.top_nodes_mut().nth(path[0]).expect("path from player_path");
    for &i in &path[1..] {
        let Kind::Block(b) = &mut n.kind else { unreachable!() };
        n = &mut b.children[i];
    }
    n
}

fn player_ss<'a>(doc: &'a Document, path: &[usize]) -> Result<&'a crate::token::Sized> {
    let Kind::Block(b) = &node_at(doc, path).kind else { unreachable!() };
    b.children
        .iter()
        .find_map(|c| match &c.kind {
            Kind::Sized(s) => Some(s),
            _ => None,
        })
        .ok_or_else(|| Error::Unverified("character: the player entityData has no SS section".into()))
}

/// The player's record stream.
pub fn player_stream(doc: &Document) -> Result<Stream> {
    let path = player_path(doc)?;
    let ss = player_ss(doc, &path)?;
    let rest = ss.blob_bytes().ok_or_else(|| Error::Unverified("character: the player SS is not a plain blob".into()))?;
    let mut s = Stream::parse(&rest)?;
    s.version = ss.sxap.map_or(0, |x| x.ints[0]);
    if s.to_bytes() != rest {
        return unv("the player record stream does not re-serialize to the same bytes");
    }
    Ok(s)
}

/// Write `stream` back as the player's `SS` body (the `SXAP` header stays). Call `Document::rebase`
/// afterwards (`edit_player` does).
fn set_player_stream(doc: &mut Document, stream: &Stream) -> Result<()> {
    let path = player_path(doc)?;
    let Kind::Block(b) = &mut node_at_mut(doc, &path).kind else { unreachable!() };
    let ss = b
        .children
        .iter_mut()
        .find_map(|c| match &mut c.kind {
            Kind::Sized(s) => Some(s),
            _ => None,
        })
        .ok_or_else(|| Error::Unverified("character: the player entityData has no SS section".into()))?;
    ss.set_blob_bytes(stream.to_bytes())
}

// ---------------------------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------------------------

struct Names<'a> {
    list: &'a [String],
}

impl Names<'_> {
    fn idx(&self, name: &str) -> Option<u16> {
        self.list.iter().position(|n| n == name).and_then(|i| u16::try_from(i + 1).ok())
    }
    fn get(&self, i: u16) -> &str {
        (i as usize).checked_sub(1).and_then(|k| self.list.get(k)).map_or("", String::as_str)
    }
}

fn names(doc: &Document) -> Result<Names<'_>> {
    Ok(Names { list: doc.names.as_deref().ok_or_else(|| Error::Unverified("character: the NM table could not be read".into()))? })
}

// ---------------------------------------------------------------------------------------------
// Level manager
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Points {
    pub free: i32,
    pub used: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LevelDef {
    pub number: i32,
    pub required_total_exp: i32,
    pub added_skill_points: i32,
    pub required_exp: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelInfo {
    pub level: i32,
    pub skill: Points,
    pub xp: Points,
    pub last_custom_level: i32,
    /// `levelDefinitions` as saved; entry `k` describes level `k`, entry 0 is the "infinite levels" row.
    pub table: Vec<LevelDef>,
}

impl LevelInfo {
    /// Total experience the character has: `free + used` of the experience pool.
    pub fn total_xp(&self) -> i64 {
        i64::from(self.xp.free) + i64::from(self.xp.used)
    }
    /// `requiredTotalExp` of `level` (the game's `GetTotalExpForGivenLevel` for levels in the table).
    pub fn exp_for_level(&self, level: i32) -> Option<i32> {
        if level < 1 || self.last_custom_level < 1 {
            return None;
        }
        if level > self.last_custom_level {
            // `GetLevelDefinition` of the script: past the table every level costs row 0's `requiredExp`.
            let past = level - self.last_custom_level;
            return Some(self.table.get(self.last_custom_level as usize)?.required_total_exp + past * self.table.first()?.required_exp);
        }
        self.table.get(level as usize).map(|d| d.required_total_exp)
    }
    /// Skill points the table grants for reaching `level` from level 1 (levels 2..=level; past the table row 0's
    /// `addedSkillPoints` per level).
    pub fn skill_points_up_to(&self, level: i32) -> Option<i32> {
        if level < 1 || self.last_custom_level < 1 || self.table.len() <= self.last_custom_level as usize {
            return None;
        }
        let inside: i32 = (2..=level.min(self.last_custom_level)).map(|l| self.table[l as usize].added_skill_points).sum();
        let past = (level - self.last_custom_level).max(0);
        Some(inside + past * self.table.first()?.added_skill_points)
    }
}

fn field_i32(nm: &Names, f: &Fields, name: &str) -> i32 {
    let Some(ix) = nm.idx(name) else { return 0 };
    f.iter().find(|p| p.name == ix).and_then(|p| le32(&p.value, 0).ok()).map_or(0, |v| v as i32)
}

fn entity_block(s: &Stream) -> Option<&Vec<Rec>> {
    s.recs.iter().find_map(|r| match r {
        Rec::Block { kids, .. } => Some(kids),
        _ => None,
    })
}

fn entity_prop<'a>(nm: &Names, s: &'a Stream, name: &str) -> Option<&'a Vec<u8>> {
    let ix = nm.idx(name)?;
    entity_block(s)?.iter().find_map(|r| match r {
        Rec::Val { porp: true, name, data, .. } if *name == ix => Some(data),
        _ => None,
    })
}

fn level_obj(nm: &Names, s: &Stream) -> Result<Obj> {
    let data = entity_prop(nm, s, "levelManager").ok_or_else(|| Error::Unverified("character: no levelManager property".into()))?;
    let o = Obj::parse(data)?;
    if nm.get(o.class) != "W3LevelManager" {
        return unv("levelManager is not a W3LevelManager object");
    }
    if o.to_bytes() != *data {
        return unv("the levelManager object does not re-serialize to the same bytes");
    }
    Ok(o)
}

fn points_of(nm: &Names, o: &Obj) -> Result<[Points; 2]> {
    let ix = nm.idx("points").ok_or_else(|| Error::Unverified("character: no `points` name".into()))?;
    let p = o.prop(ix).ok_or_else(|| Error::Unverified("character: levelManager has no points".into()))?;
    let arr = parse_struct_array(&p.value)?;
    if arr.len() != 2 {
        return unv(format!("expected 2 SSpendablePoints, found {}", arr.len()));
    }
    let g = |f: &Fields| Points { free: field_i32(nm, f, "free"), used: field_i32(nm, f, "used") };
    Ok([g(&arr[0]), g(&arr[1])])
}

fn read_level(nm: &Names, s: &Stream) -> Result<LevelInfo> {
    let o = level_obj(nm, s)?;
    let int = |name: &str| -> Result<i32> {
        let ix = nm.idx(name).ok_or_else(|| Error::Unverified(format!("character: no `{name}` name")))?;
        let p = o.prop(ix).ok_or_else(|| Error::Unverified(format!("character: levelManager has no {name}")))?;
        Ok(le32(&p.value, 0)? as i32)
    };
    let defs_ix = nm.idx("levelDefinitions").ok_or_else(|| Error::Unverified("character: no levelDefinitions name".into()))?;
    let defs = o.prop(defs_ix).ok_or_else(|| Error::Unverified("character: levelManager has no levelDefinitions".into()))?;
    let table = parse_struct_array(&defs.value)?
        .iter()
        .map(|f| LevelDef {
            number: field_i32(nm, f, "number"),
            required_total_exp: field_i32(nm, f, "requiredTotalExp"),
            added_skill_points: field_i32(nm, f, "addedSkillPoints"),
            required_exp: field_i32(nm, f, "requiredExp"),
        })
        .collect();
    let pts = points_of(nm, &o)?;
    Ok(LevelInfo { level: int("level")?, skill: pts[SKILL_POINT], xp: pts[EXPERIENCE_POINT], last_custom_level: int("lastCustomLevel")?, table })
}

// ---------------------------------------------------------------------------------------------
// Abilities (Lvl<N>) and base stats
// ---------------------------------------------------------------------------------------------

/// Where the player's ability list lives: `BLCK stats > BLCK characterStats > AVAL value (array of CName)`
/// next to `AVAL count`. Returns the record indices (stats block, characterStats block).
fn ability_list_path(nm: &Names, s: &Stream) -> Option<(usize, usize)> {
    let stats = nm.idx("stats")?;
    let cs = nm.idx("characterStats")?;
    let si = s.recs.iter().position(|r| matches!(r, Rec::Block { name, .. } if *name == stats))?;
    let Rec::Block { kids, .. } = &s.recs[si] else { return None };
    let ci = kids.iter().position(|r| matches!(r, Rec::Block { name, .. } if *name == cs))?;
    Some((si, ci))
}

fn ability_names(nm: &Names, s: &Stream) -> Result<Vec<u16>> {
    let (si, ci) = ability_list_path(nm, s).ok_or_else(|| Error::Unverified("character: no stats > characterStats block".into()))?;
    let Rec::Block { kids, .. } = &s.recs[si] else { unreachable!() };
    let Rec::Block { kids, .. } = &kids[ci] else { unreachable!() };
    let value = nm.idx("value").ok_or_else(|| Error::Unverified("character: no `value` name".into()))?;
    let Some(Rec::Val { data, .. }) = kids.iter().find(|r| matches!(r, Rec::Val { name, .. } if *name == value)) else {
        return unv("characterStats has no value list");
    };
    let n = le32(data, 0)? as usize;
    if data.len() != 4 + 2 * n {
        return unv("the ability list length does not match its count");
    }
    (0..n).map(|k| le16(data, 4 + 2 * k)).collect()
}

/// Level numbers of the `Lvl<N>` abilities on the character, in list order.
pub fn level_abilities(doc: &Document) -> Result<Vec<i32>> {
    let nm = names(doc)?;
    let s = player_stream(doc)?;
    Ok(ability_names(&nm, &s)?.iter().filter_map(|&i| nm.get(i).strip_prefix("Lvl").and_then(|n| n.parse().ok())).collect())
}

/// `statPoints` of the ability manager: (index in the array, stat name, current, max) per element. An
/// element without a `type` field is `BCS_Vitality`. HYPOTHESIS: the enum's first value is stored as an
/// absent field (SBaseStat.type is a name index and the game leaves out default values); it is
/// consistent with every save read (element 0 is the only one with the large vitality numbers).
#[derive(Debug, Clone, PartialEq)]
pub struct BaseStat {
    pub name: String,
    pub current: f32,
    pub max: f32,
}

fn ability_manager(nm: &Names, s: &Stream) -> Result<Obj> {
    let data = entity_prop(nm, s, "abilityManager").ok_or_else(|| Error::Unverified("character: no abilityManager property".into()))?;
    let o = Obj::parse(data)?;
    if o.to_bytes() != *data {
        return unv("the abilityManager object does not re-serialize to the same bytes");
    }
    Ok(o)
}

fn f32_field(nm: &Names, f: &Fields, name: &str) -> f32 {
    nm.idx(name).and_then(|ix| f.iter().find(|p| p.name == ix)).and_then(|p| le32(&p.value, 0).ok()).map_or(0.0, f32::from_bits)
}

fn stat_points(nm: &Names, am: &Obj) -> Result<Vec<Fields>> {
    let ix = nm.idx("statPoints").ok_or_else(|| Error::Unverified("character: no statPoints name".into()))?;
    let p = am.prop(ix).ok_or_else(|| Error::Unverified("character: abilityManager has no statPoints".into()))?;
    parse_struct_array(&p.value)
}

fn base_stats(nm: &Names, fields: &[Fields]) -> Vec<BaseStat> {
    fields
        .iter()
        .map(|f| {
            let ty = nm.idx("type").and_then(|ix| f.iter().find(|p| p.name == ix)).and_then(|p| le16(&p.value, 0).ok());
            BaseStat {
                name: ty.map_or_else(|| "BCS_Vitality".to_string(), |t| nm.get(t).to_string()),
                current: f32_field(nm, f, "current"),
                max: f32_field(nm, f, "max"),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Inventory tail
// ---------------------------------------------------------------------------------------------

/// One item record as far as it is understood (HYPOTHESIS for everything but name and quantity; see the
/// findings file). `at` is the offset of the item-name `u16` inside the tail.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemView {
    pub name: String,
    pub quantity: u32,
    /// `f32` after the quantity; `-1.0` for items without durability.
    pub durability: f32,
    pub at: usize,
    /// The record's flag word (see the layout comment at `parse_records`).
    pub flags: u16,
}

/// Header of the item list at the start of the tail: `00 00`, five bytes `01 a2 15 35 b6`, then three
/// `u16`: an unexplained number, the next free unique id, the item count (proven: count == number of
/// records on every fixture; next id == the id stored after the last record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemHeader {
    pub unknown: u16,
    pub next_uid: u16,
    pub count: u16,
}

const TAIL_MAGIC: [u8; 7] = [0, 0, 1, 0xa2, 0x15, 0x35, 0xb6];

fn item_header(tail: &[u8]) -> Result<ItemHeader> {
    if tail.len() < 13 || tail[2..7] != TAIL_MAGIC[2..] {
        return unv("the player tail does not start with the item-list header");
    }
    Ok(ItemHeader { unknown: le16(tail, 7)?, next_uid: le16(tail, 9)?, count: le16(tail, 11)? })
}

// Item records
//
// Layout (every field is checked by `parse_records`, which tiles the whole item list to the 6-byte trailer
// on 163 of 163 `SXAP 67`, 1 of 1 `66` and 117 of 117 `64` fixtures; docs/findings/character-inventory.md):
//
//   u16 name | u16 hash | u16 flags | 01 00 00 00 00 00 | u8 slots | u16 dye1 | u16 dye2
//   [flags & 0x4000: u32 unknown] | qty: u16, or u32 when flags & 0x2000 | f32 durability
//   u8 nExtras | nExtras x (u16 name, u32 value, 02)
//   u16 uid | u8 z | z x 9 bytes (u16 a, u16 b, 00, 00 01, u16 c: an enhancement entry) | u8 nAbilities | nAbilities x u16
//   [flags & 0x20: u16 slot name (equipped)] | [flags & 0x8000: u8 n, n x u16] | [SXAP 66/67: u16 0]
//
// `flags` bits seen: 0x0002/0x0008/0x000a/0x000f kind, 0x0020 equipped, 0x0100 looted, 0x1000 (unexplained),
// 0x2000 quantity is a u32 (all 8 Crowns records above 65535 carry it), 0x4000 an extra u32 before the
// quantity (modded NG+ save), 0x8000 a list of names after the slot (dyed pieces).

const FIXED6: [u8; 6] = [1, 0, 0, 0, 0, 0];
const TRAILER: [u8; 6] = [1, 0, 0, 0, 0, 0];

/// One item record located in the tail.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordSpan {
    pub start: usize,
    pub end: usize,
    pub name: u16,
    pub uid: u16,
    pub flags: u16,
    /// Offset of the quantity field and whether it is a `u32`.
    pub qty_at: usize,
    pub qty_wide: bool,
    pub quantity: u32,
    /// `-1.0` for items without durability.
    pub durability: f32,
}

/// Parse the item list sequentially. `Unverified` on any shape not understood, on non-ascending uids, or
/// when the list does not end at the trailer. `version` is the first `SXAP` int of the player section.
pub fn parse_records(tail: &[u8], version: u32) -> Result<(ItemHeader, Vec<RecordSpan>)> {
    let pad = match version {
        64 => false,
        66 | 67 => true,
        v => return unv(format!("item layout of SXAP {v} is not known")),
    };
    let header = item_header(tail)?;
    let mut out: Vec<RecordSpan> = Vec::with_capacity(header.count as usize);
    let mut p = 13;
    for _ in 0..header.count {
        let start = p;
        let shape = || Error::Unverified(format!("character: item record at {start} has a shape that is not understood"));
        if tail.get(p + 6..p + 12) != Some(&FIXED6[..]) {
            return Err(shape());
        }
        let flags = le16(tail, p + 4)?;
        let mut c = p + 17;
        if flags & 0x4000 != 0 {
            c += 4;
        }
        let qty_at = c;
        let qty_wide = flags & 0x2000 != 0;
        let quantity = if qty_wide { le32(tail, c)? } else { u32::from(le16(tail, c)?) };
        c += if qty_wide { 4 } else { 2 };
        let durability = f32::from_bits(le32(tail, c)?);
        c += 4;
        c += 1 + 7 * *tail.get(c).ok_or_else(shape)? as usize;
        let uid = le16(tail, c)?;
        let z = *tail.get(c + 2).ok_or_else(shape)? as usize;
        c += 3 + 9 * z;
        c += 1 + 2 * *tail.get(c).ok_or_else(shape)? as usize;
        if flags & 0x20 != 0 {
            c += 2;
        }
        if flags & 0x8000 != 0 {
            c += 1 + 2 * *tail.get(c).ok_or_else(shape)? as usize;
        }
        if pad {
            c += 2;
        }
        if c > tail.len() {
            return Err(shape());
        }
        out.push(RecordSpan { start, end: c, name: le16(tail, start)?, uid, flags, qty_at, qty_wide, quantity, durability });
        p = c;
    }
    if tail.len() - p != 6 || tail[p..] != TRAILER {
        return unv("the item list does not end at the 6-byte trailer");
    }
    Ok((header, out))
}

fn item_views(nm: &Names, tail: &[u8], version: u32) -> Result<Vec<ItemView>> {
    let (_, spans) = parse_records(tail, version)?;
    Ok(spans
        .iter()
        .map(|r| ItemView { name: nm.get(r.name).to_string(), quantity: r.quantity, durability: r.durability, at: r.start, flags: r.flags })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Public read API
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Character {
    pub level: LevelInfo,
    /// `Lvl<N>` abilities in list order.
    pub level_abilities: Vec<i32>,
    pub stats: Vec<BaseStat>,
    pub header: ItemHeader,
    pub items: Vec<ItemView>,
}

impl Character {
    pub fn crowns(&self) -> Option<u32> {
        self.items.iter().find(|i| i.name == "Crowns").map(|i| i.quantity)
    }
    pub fn vitality(&self) -> Option<&BaseStat> {
        self.stats.iter().find(|s| s.name == "BCS_Vitality")
    }
}

pub fn read(doc: &Document) -> Result<Character> {
    let nm = names(doc)?;
    let s = player_stream(doc)?;
    let level = read_level(&nm, &s)?;
    let am = ability_manager(&nm, &s)?;
    let stats = base_stats(&nm, &stat_points(&nm, &am)?);
    let level_abilities = ability_names(&nm, &s)?.iter().filter_map(|&i| nm.get(i).strip_prefix("Lvl").and_then(|n| n.parse().ok())).collect();
    Ok(Character { level, level_abilities, stats, header: item_header(&s.tail)?, items: item_views(&nm, &s.tail, s.version)? })
}

/// Run `f` on the player's stream, write it back and rebase. The document is unchanged on error from `f`.
fn edit_stream<R>(doc: &mut Document, f: impl FnOnce(&mut Document, &mut Stream) -> Result<R>) -> Result<R> {
    doc.rebase()?;
    let mut s = player_stream(doc)?;
    let out = f(doc, &mut s)?;
    set_player_stream(doc, &s)?;
    doc.rebase()?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Crowns
// ---------------------------------------------------------------------------------------------

/// Set the number of crowns: the quantity of the `Crowns` item record.
pub fn set_crowns(doc: &mut Document, crowns: u32) -> Result<()> {
    set_item_quantity(doc, "Crowns", crowns)
}

/// Set the quantity of an item stack that is already in the inventory (a record without durability).
///
/// The quantity is a `u16`, or a `u32` when the record's flag `0x2000` is set (proven: every Crowns record
/// above 65535 in the fixtures carries it, and the 163 saves tile with that rule). A value above 65535 in a
/// narrow record turns the flag on and widens the field (HYPOTHESIS: the game reads a record the same way
/// whether it set the flag itself or we did). Creating a record for an item the character does not carry is
/// [`add_item`].
pub fn set_item_quantity(doc: &mut Document, item: &str, quantity: u32) -> Result<()> {
    edit_stream(doc, |doc, s| {
        let (_, spans) = parse_records(&s.tail, s.version)?;
        let Some(ix) = doc.name_index(item) else {
            return unv(format!("expected exactly one `{item}` item record in the inventory"));
        };
        let mut hit = spans.iter().filter(|r| r.name == ix);
        let (Some(r), None) = (hit.next(), hit.next()) else {
            return unv(format!("expected exactly one `{item}` item record in the inventory (use add_item for a new one)"));
        };
        if r.durability != -1.0 {
            return unv(format!("`{item}` has a durability field: a gear record, quantity edits are for stacks"));
        }
        if r.qty_wide {
            s.tail[r.qty_at..r.qty_at + 4].copy_from_slice(&quantity.to_le_bytes());
        } else if let Ok(q) = u16::try_from(quantity) {
            s.tail[r.qty_at..r.qty_at + 2].copy_from_slice(&q.to_le_bytes());
        } else {
            let flags = r.flags | 0x2000;
            s.tail[r.start + 4..r.start + 6].copy_from_slice(&flags.to_le_bytes());
            s.tail[r.qty_at..r.qty_at + 2].copy_from_slice(&(quantity as u16).to_le_bytes());
            s.tail.splice(r.qty_at + 2..r.qty_at + 2, ((quantity >> 16) as u16).to_le_bytes());
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------------------------
// Level
// ---------------------------------------------------------------------------------------------

fn put_field(f: &mut Fields, name: u16, ty: u16, v: i32) {
    f.retain(|p| p.name != name);
    if v != 0 {
        f.push(Prop { name, ty, value: v.to_le_bytes().to_vec() });
    }
}

/// What `set_level` did, for reports and tests.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelChange {
    pub from: i32,
    pub to: i32,
    pub skill: Points,
    pub xp: Points,
    pub added_abilities: Vec<i32>,
    pub removed_abilities: Vec<i32>,
    pub vitality_delta: f32,
}

/// Max vitality one `Lvl<N>` ability adds. Source: the game's `gameplay\abilities\geralt_stats.xml`
/// (`<ability name="Lvl2" ...><vitality type="base" min="100" max="100"/>`); the values come in bands.
/// PROVEN against saves: see the findings file (maximum vitality in the saves moves by exactly this).
pub fn lvl_vitality(level: i32) -> Option<f32> {
    match level {
        2..=5 => Some(100.0),
        6..=10 => Some(80.0),
        11..=20 => Some(60.0),
        21..=29 => Some(50.0),
        30..=39 => Some(30.0),
        40..=100 => Some(25.0),
        _ => None,
    }
}

/// Set the character's level the way the game would have got there with no skills bought beyond the ones
/// already learned:
///
/// * `levelManager.level = target`;
/// * experience pool: `used = requiredTotalExp(target)`, `free = 0` (the game's own invariant, `GainLevel`);
/// * skill points: `free + used` = what the table grants for the target level plus the points the template
///   already had beyond its own level's table sum (quest / book points keep counting), `used` unchanged
///   (never lowered below what is spent: lowering below spent points is refused);
/// * the `Lvl<N>` abilities: appended for new levels, removed for dropped ones (what `AddPoints` ->
///   `AddAbility(GetLevelupAbility(level))` does);
/// * stored maximum and current vitality move by the vitality of the added / removed `Lvl<N>`
///   (HYPOTHESIS: the game does not recompute the stored stat from the ability list on load).
///
/// `target` must be within the save's own XP table (1..=`lastCustomLevel`).
pub fn set_level(doc: &mut Document, target: i32) -> Result<LevelChange> {
    edit_stream(doc, |doc, s| {
        // Read phase: nothing is touched until every check passed.
        let (info, cur_abilities, nm_snapshot) = {
            let nm = names(doc)?;
            let info = read_level(&nm, s)?;
            (info, ability_names(&nm, s)?, nm.list.to_vec())
        };
        let nm_view = Names { list: &nm_snapshot };
        if target > info.last_custom_level {
            return unv(format!("level {target} is above the save's XP table (1..={}): no Lvl<N> ability exists past it", info.last_custom_level));
        }
        let need = info.exp_for_level(target).ok_or_else(|| Error::Unverified(format!("character: level {target} is outside the save's XP table (1..={})", info.last_custom_level)))?;
        let from = info.level;
        let table_now = info.skill_points_up_to(from).ok_or_else(|| Error::Unverified("character: the current level is outside the XP table".into()))?;
        let table_new = info.skill_points_up_to(target).expect("target checked above");
        // HYPOTHESIS: points beyond the table (quests, books) stay.
        let bonus = info.skill.free + info.skill.used - table_now;
        let new_total = table_new + bonus.max(0);
        // A character that spent more points than the target level grants keeps them (free = 0); respeccing
        // skills is not done. HYPOTHESIS: the game does not check spent points against the level.
        let new_skill = Points { free: (new_total - info.skill.used).max(0), used: info.skill.used };
        let new_xp = Points { free: 0, used: need };

        // Names we may have to add.
        let mut add: Vec<i32> = Vec::new();
        let mut remove: Vec<i32> = Vec::new();
        if target > from {
            add.extend(from + 1..=target);
        } else {
            remove.extend(target + 1..=from);
        }
        for l in &add {
            lvl_vitality(*l).ok_or_else(|| Error::Unverified(format!("character: no vitality value for Lvl{l}")))?;
        }
        let vitality_delta: f32 = add.iter().filter_map(|l| lvl_vitality(*l)).sum::<f32>() - remove.iter().filter_map(|l| lvl_vitality(*l)).sum::<f32>();
        let _ = nm_view;

        let name_ix = |doc: &mut Document, n: &str| doc.intern_name(n);
        let (level_ix, points_ix, free_ix, used_ix, int32_ix) = (
            name_ix(doc, "level")?,
            name_ix(doc, "points")?,
            name_ix(doc, "free")?,
            name_ix(doc, "used")?,
            name_ix(doc, "Int32")?,
        );
        let add_ixs: Vec<u16> = add.iter().map(|l| name_ix(doc, &format!("Lvl{l}"))).collect::<Result<_>>()?;
        let remove_ixs: Vec<u16> = remove.iter().filter_map(|l| doc.name_index(&format!("Lvl{l}"))).collect();

        // levelManager
        {
            let nm = names(doc)?;
            let mut o = level_obj(&nm, s)?;
            for p in &mut o.props {
                if p.name == level_ix {
                    p.value = target.to_le_bytes().to_vec();
                } else if p.name == points_ix {
                    let mut arr = parse_struct_array(&p.value)?;
                    for (k, np) in [(SKILL_POINT, new_skill), (EXPERIENCE_POINT, new_xp)] {
                        put_field(&mut arr[k], free_ix, int32_ix, np.free);
                        put_field(&mut arr[k], used_ix, int32_ix, np.used);
                        // The game writes `free` before `used`.
                        arr[k].sort_by_key(|f| (f.name != free_ix) as u8);
                    }
                    p.value = write_struct_array(&arr);
                }
            }
            let lm_ix = nm.idx("levelManager").expect("level_obj found it");
            let bytes = o.to_bytes();
            let Some(Rec::Block { kids, .. }) = s.recs.iter_mut().find(|r| matches!(r, Rec::Block { .. })) else { return unv("no Entity block") };
            for r in kids {
                if let Rec::Val { name, data, .. } = r {
                    if *name == lm_ix {
                        *data = bytes.clone();
                    }
                }
            }
        }

        // ability list: `value` array and the `count` next to it
        {
            let nm = names(doc)?;
            let (si, ci) = ability_list_path(&nm, s).ok_or_else(|| Error::Unverified("character: no stats > characterStats block".into()))?;
            let (value_ix, count_ix) = (nm.idx("value"), nm.idx("count"));
            let Rec::Block { kids, .. } = &mut s.recs[si] else { unreachable!() };
            let Rec::Block { kids, .. } = &mut kids[ci] else { unreachable!() };
            let mut list = cur_abilities.clone();
            for ix in &remove_ixs {
                if let Some(pos) = list.iter().position(|a| a == ix) {
                    list.remove(pos);
                }
            }
            list.extend(&add_ixs);
            let mut bytes = (list.len() as u32).to_le_bytes().to_vec();
            for a in &list {
                bytes.extend_from_slice(&a.to_le_bytes());
            }
            for r in kids.iter_mut() {
                if let Rec::Val { name, data, .. } = r {
                    if Some(*name) == value_ix {
                        *data = bytes.clone();
                    } else if Some(*name) == count_ix && data.len() == 4 {
                        *data = (list.len() as u32).to_le_bytes().to_vec();
                    }
                }
            }
        }

        // stored vitality
        if vitality_delta != 0.0 {
            let nm = names(doc)?;
            let mut am = ability_manager(&nm, s)?;
            let sp_ix = nm.idx("statPoints").expect("ability_manager found it");
            let (cur_ix, max_ix) = (nm.idx("current"), nm.idx("max"));
            let (Some(cur_ix), Some(max_ix)) = (cur_ix, max_ix) else { return unv("no current / max names") };
            let ty_ix = nm.idx("type");
            for p in &mut am.props {
                if p.name == sp_ix {
                    let mut arr = parse_struct_array(&p.value)?;
                    // The vitality element is the one without a `type` field.
                    let vit = arr.iter_mut().find(|f| ty_ix.is_none_or(|t| !f.iter().any(|q| q.name == t)));
                    let Some(vit) = vit else { return unv("no vitality element in statPoints") };
                    for q in vit.iter_mut() {
                        if q.name == cur_ix || q.name == max_ix {
                            let v = f32::from_bits(le32(&q.value, 0)?) + vitality_delta;
                            q.value = v.to_bits().to_le_bytes().to_vec();
                        }
                    }
                    p.value = write_struct_array(&arr);
                }
            }
            let am_ix = nm.idx("abilityManager").expect("ability_manager found it");
            let bytes = am.to_bytes();
            let Some(Rec::Block { kids, .. }) = s.recs.iter_mut().find(|r| matches!(r, Rec::Block { .. })) else { return unv("no Entity block") };
            for r in kids {
                if let Rec::Val { name, data, .. } = r {
                    if *name == am_ix {
                        *data = bytes.clone();
                    }
                }
            }
        }

        Ok(LevelChange { from, to: target, skill: new_skill, xp: new_xp, added_abilities: add, removed_abilities: remove, vitality_delta })
    })
}

/// Set the character's ability points to `total` (`free + used` of the skill pool): `used` (points spent on
/// learned skills) stays, `free = total - used`. Refused when `total` is below what is already spent (no respec).
/// Only `levelManager > points[ESkillPoint]` changes. Returns `(before, after)`.
/// HYPOTHESIS: the game does not recompute the pool from the level table and Places of Power on load (it adds
/// to `free` on level-up / book / Place of Power only, `levelManager.ws`).
pub fn set_skill_points(doc: &mut Document, total: i32) -> Result<(Points, Points)> {
    edit_stream(doc, |doc, s| {
        let info = {
            let nm = names(doc)?;
            read_level(&nm, s)?
        };
        if total < info.skill.used {
            return unv(format!("{total} ability points is below the {} already spent on learned skills (unlearning is not supported)", info.skill.used));
        }
        let new = Points { free: total - info.skill.used, used: info.skill.used };
        let (points_ix, free_ix, used_ix, int32_ix) = (doc.intern_name("points")?, doc.intern_name("free")?, doc.intern_name("used")?, doc.intern_name("Int32")?);
        let nm = names(doc)?;
        let mut o = level_obj(&nm, s)?;
        for p in &mut o.props {
            if p.name == points_ix {
                let mut arr = parse_struct_array(&p.value)?;
                put_field(&mut arr[SKILL_POINT], free_ix, int32_ix, new.free);
                put_field(&mut arr[SKILL_POINT], used_ix, int32_ix, new.used);
                // The game writes `free` before `used`.
                arr[SKILL_POINT].sort_by_key(|f| (f.name != free_ix) as u8);
                p.value = write_struct_array(&arr);
            }
        }
        let lm_ix = nm.idx("levelManager").expect("level_obj found it");
        let bytes = o.to_bytes();
        let Some(Rec::Block { kids, .. }) = s.recs.iter_mut().find(|r| matches!(r, Rec::Block { .. })) else { return unv("no Entity block") };
        for r in kids {
            if let Rec::Val { name, data, .. } = r {
                if *name == lm_ix {
                    *data = bytes.clone();
                }
            }
        }
        Ok((info.skill, new))
    })
}

// ---------------------------------------------------------------------------------------------
// Adding items
// ---------------------------------------------------------------------------------------------
//
// What the new record's fields are (docs/findings/character-inventory.md section 6):
// * `slots` = `enhancement_slots` of the item definition (the byte after the six fixed ones);
// * `flags`: 0x000f potions, oils, bombs, stacks; 0x0008 sword and 0x000a armour piece as quest rewards;
// * `durability` = `initial_durability` (100) for a fresh piece, -1.0 for items without;
// * `uid` ascending; header field B = the last uid handed out (B == max uid on 132 of 140 saves at the time,
//   never below). A new record takes B + 1 and B moves with it;
// * `hash`: constant per item instance, differs between instances. HYPOTHESIS: free; derived from name and uid;
// * charged items (potions, oils, bombs) carry `ammo_current` and `is_initialized` extras.

/// What a new item looks like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemForm {
    /// A stack (food, bolts, ingredients): flags 0x000f, no durability, no extras.
    Stack,
    /// A potion, oil or bomb: flags 0x000f plus `ammo_current` = `charges` and `is_initialized` = 1.
    Charged { charges: u32 },
    /// A weapon or armour piece in the shape of a quest reward (no ability list, not equipped).
    Gear { weapon: bool, durability: f32, slots: u8 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewItem {
    pub name: String,
    pub quantity: u16,
    pub form: ItemForm,
}

/// The `hash` a new record gets (HYPOTHESIS: free): FNV-1a over name and uid, folded to 15 bits like the
/// values seen (all below 0x8000).
pub fn derived_hash(name: &str, uid: u16) -> u16 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.bytes().chain(uid.to_le_bytes()) {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    ((h ^ (h >> 16)) & 0x7fff) as u16
}

/// The bytes of one record. `ix` resolves a name to its NM index; `pad` = the layout has the trailing
/// `u16 0` (`SXAP` 66 / 67).
pub fn build_record(item: &NewItem, uid: u16, hash: u16, pad: bool, ix: &mut dyn FnMut(&str) -> Result<u16>) -> Result<Vec<u8>> {
    let (flags, slots, durability, extras): (u16, u8, f32, Vec<(&str, u32)>) = match &item.form {
        ItemForm::Stack => (0x000f, 0, -1.0, vec![]),
        ItemForm::Charged { charges } => (0x000f, 0, -1.0, vec![("ammo_current", *charges), ("is_initialized", 1)]),
        ItemForm::Gear { weapon, durability, slots } => (if *weapon { 0x0008 } else { 0x000a }, *slots, *durability, vec![]),
    };
    let mut r = Vec::with_capacity(48);
    r.extend_from_slice(&ix(&item.name)?.to_le_bytes());
    r.extend_from_slice(&hash.to_le_bytes());
    r.extend_from_slice(&flags.to_le_bytes());
    r.extend_from_slice(&FIXED6);
    r.push(slots);
    r.extend_from_slice(&ix("Dye Default")?.to_le_bytes());
    r.extend_from_slice(&ix("dye_default")?.to_le_bytes());
    r.extend_from_slice(&item.quantity.to_le_bytes());
    r.extend_from_slice(&durability.to_bits().to_le_bytes());
    r.push(extras.len() as u8);
    for (name, v) in extras {
        r.extend_from_slice(&ix(name)?.to_le_bytes());
        r.extend_from_slice(&v.to_le_bytes());
        r.push(2);
    }
    r.extend_from_slice(&uid.to_le_bytes());
    r.extend_from_slice(&[0, 0]); // z, nAbilities
    if pad {
        r.extend_from_slice(&[0, 0]);
    }
    Ok(r)
}

/// Add a new item record to the inventory.
///
/// All-or-nothing; refuses (`Unverified`) when the save's item list is not fully understood, when the item
/// is already carried (use [`set_item_quantity`] for stacks), or when the list has no `Dye Default` /
/// `dye_default` names. HYPOTHESIS (no in-game check yet): the game accepts a record with a derived `hash`,
/// the next uid and a moved `B`. The record SHAPES are proven offline: for the items in the tests the bytes
/// equal the record the game wrote.
pub fn add_item(doc: &mut Document, item: &NewItem) -> Result<u16> {
    doc.rebase()?;
    let mut s = player_stream(doc)?;
    let (header, spans) = parse_records(&s.tail, s.version)?;
    if doc.name_index("Dye Default").is_none() || doc.name_index("dye_default").is_none() {
        return unv("the save has no `Dye Default` / `dye_default` names");
    }
    if let Some(ix) = doc.name_index(&item.name) {
        if spans.iter().any(|r| r.name == ix) {
            return unv(format!("`{}` is already in the inventory", item.name));
        }
    }
    let max_uid = spans.iter().map(|r| r.uid).max().unwrap_or(0);
    if header.next_uid < max_uid {
        return unv("list header field B is below the highest uid: its meaning is not understood for this save");
    }
    let uid = header.next_uid.checked_add(1).ok_or_else(|| Error::Unverified("character: no free item uid".into()))?;
    let hash = derived_hash(&item.name, uid);
    let mut ix = |n: &str| doc.intern_name(n);
    let rec = build_record(item, uid, hash, s.version != 64, &mut ix)?;
    let at = s.tail.len() - TRAILER.len();
    s.tail.splice(at..at, rec);
    s.tail[9..11].copy_from_slice(&uid.to_le_bytes());
    s.tail[11..13].copy_from_slice(&(header.count + 1).to_le_bytes());
    set_player_stream(doc, &s)?;
    doc.rebase()?;
    Ok(uid)
}
