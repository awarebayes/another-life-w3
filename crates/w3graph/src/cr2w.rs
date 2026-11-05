//! `CR2W` resources (Witcher 3 file version 163/164): the container of every `.w2phase`,
//! `.w2quest` and `.w2scene`. Only what the quest walker needs is decoded: the names table, the
//! imports table, the exports (one typed object each) and their property streams.
//!
//! Layout as read from the 5.01 files (all 1,041 quest graphs parse to the last byte):
//! * header: `CR2W`, then ten 12-byte table descriptors from offset 0x28: `(offset, count, crc)`.
//!   Table 0 = string blob, 1 = names (8 bytes each: u32 string offset + u32 hash), 2 = imports
//!   (8 bytes: u32 string offset + u16 class name + u16 flags), 4 = exports (24 bytes: u16 class
//!   name index, u16 object flags, u32 parent export index (1-based, 0 = none), u32 size,
//!   u32 offset, ...).
//! * an export body: one 0 byte, then properties `{u16 name, u16 type, u32 size, value}` until a
//!   name index of 0; `size` counts from the type field, so the value is `size - 4` bytes.
//! * values: `CName`/enums = u16 name index; `CGUID` = 16 bytes; `Bool` = 1; ints/floats by type;
//!   `String` = 1 length byte (bit 7: 1-byte chars, bit 6: long form) + chars; `ptr:`/`handle:` =
//!   i32 (positive = export index + 1, negative = import index, 0 = null); `soft:` = u16 import
//!   index + 1; `array:2,0,T` = u32 count + elements, struct elements and struct values start with
//!   one 0 byte and are property lists.
//!
//! HYPOTHESIS: the long-form string (bit 6) and the few value types not listed are kept as raw
//! bytes; nothing the walker reads uses them.
use std::fmt;

use crate::quest::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Name(String),
    Guid([u8; 16]),
    Bool(bool),
    Int(i64),
    Float(f32),
    Str(String),
    /// Export index (`->#n`), or `None` for null.
    Ptr(Option<usize>),
    /// Import path (`handle:` to another file, or a negative `ptr`).
    Import(String),
    /// Soft handle: the import path.
    Soft(String),
    Names(Vec<String>),
    Ptrs(Vec<Option<usize>>),
    Strs(Vec<String>),
    /// Array of structs: one property list per element.
    Structs(Vec<Vec<Prop>>),
    Struct(Vec<Prop>),
    Raw(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    pub name: String,
    pub ty: String,
    pub value: Val,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub class: String,
    /// Index of the parent export, `None` at the root.
    pub parent: Option<usize>,
    pub props: Vec<Prop>,
    /// Bytes after the property list: a second property list for a `CStorySceneScript` (its
    /// function parameters) and a `CQuestScriptBlock`; the serialized event list of a
    /// `CStorySceneSection` (see `scene.rs`); empty for most exports.
    pub tail: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Cr2w {
    pub names: Vec<String>,
    pub imports: Vec<String>,
    pub exports: Vec<Export>,
}

impl fmt::Display for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Val::Name(s) | Val::Str(s) | Val::Import(s) | Val::Soft(s) => write!(f, "{s}"),
            Val::Guid(g) => write!(f, "{}", hex(g)),
            Val::Bool(b) => write!(f, "{b}"),
            Val::Int(i) => write!(f, "{i}"),
            Val::Float(x) => write!(f, "{x}"),
            Val::Ptr(Some(i)) => write!(f, "->#{i}"),
            Val::Ptr(None) => write!(f, "null"),
            Val::Names(v) | Val::Strs(v) => write!(f, "[{}]", v.join(" ")),
            Val::Ptrs(v) => write!(f, "[{}]", v.iter().map(|p| p.map_or("null".into(), |i| format!("->#{i}"))).collect::<Vec<_>>().join(" ")),
            Val::Structs(v) => write!(f, "[{} structs]", v.len()),
            Val::Struct(p) => write!(f, "{{{} props}}", p.len()),
            Val::Raw(b) => write!(f, "hex {}", hex(&b[..b.len().min(24)])),
        }
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn cstr(d: &[u8], o: usize) -> String {
    d.get(o..).map_or_else(String::new, |s| s.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect())
}

struct Reader<'a> {
    names: &'a [String],
    imports: &'a [String],
}

impl Reader<'_> {
    fn name(&self, i: u16) -> String {
        self.names.get(i as usize).cloned().unwrap_or_else(|| format!("#{i}"))
    }

    fn import(&self, i: usize) -> String {
        self.imports.get(i).cloned().unwrap_or_else(|| format!("import#{i}"))
    }

    fn prim(&self, ty: &str, v: &[u8]) -> Option<Val> {
        Some(match ty {
            "CName" | "EnumName" if v.len() >= 2 => Val::Name(self.name(u16_at(v, 0)?)),
            "CGUID" if v.len() == 16 => Val::Guid(v.try_into().ok()?),
            "Bool" if v.len() == 1 => Val::Bool(v[0] != 0),
            "Uint8" if v.len() == 1 => Val::Int(i64::from(v[0])),
            "Int8" if v.len() == 1 => Val::Int(i64::from(v[0] as i8)),
            "Uint16" if v.len() == 2 => Val::Int(i64::from(u16_at(v, 0)?)),
            "Int16" if v.len() == 2 => Val::Int(i64::from(u16_at(v, 0)? as i16)),
            "Uint32" if v.len() == 4 => Val::Int(i64::from(u32_at(v, 0)?)),
            "Int32" if v.len() == 4 => Val::Int(i64::from(u32_at(v, 0)? as i32)),
            "Float" if v.len() == 4 => Val::Float(f32::from_bits(u32_at(v, 0)?)),
            "String" | "StringAnsi" if !v.is_empty() => {
                let b = v[0];
                let n = (b & 0x3f) as usize;
                if b & 0x40 != 0 {
                    return None;
                }
                if b & 0x80 != 0 {
                    Val::Str(v.get(1..1 + n).map(|s| String::from_utf8_lossy(s).into_owned())?)
                } else {
                    let s: Option<Vec<u16>> = (0..n).map(|i| u16_at(v, 1 + 2 * i)).collect();
                    Val::Str(String::from_utf16_lossy(&s?))
                }
            }
            t if t.starts_with("soft:") => Val::Soft(self.import((u16_at(v, 0)? as usize).wrapping_sub(1))),
            t if t.starts_with("ptr:") || t.starts_with("handle:") => {
                let x = u32_at(v, 0)? as i32;
                if x > 0 {
                    Val::Ptr(Some(x as usize - 1))
                } else if x < 0 {
                    Val::Import(self.import((-x - 1) as usize))
                } else {
                    Val::Ptr(None)
                }
            }
            "CVariant" if v.len() >= 6 => {
                // `{u16 type name, u32 size, value}` (quest script parameters).
                let ty = self.name(u16_at(v, 0)?);
                let size = u32_at(v, 2)? as usize;
                let inner = v.get(6..6 + size.saturating_sub(0).min(v.len() - 6))?;
                self.prim(&ty, inner).unwrap_or_else(|| Val::Raw(inner.to_vec()))
            }
            t if t.starts_with('E') && v.len() == 2 => Val::Name(self.name(u16_at(v, 0)?)),
            _ => return None,
        })
    }

    fn value(&self, ty: &str, v: &[u8]) -> Val {
        if let Some(p) = self.prim(ty, v) {
            return p;
        }
        if let Some(rest) = ty.strip_prefix("array:") {
            let et = rest.splitn(3, ',').nth(2).unwrap_or("");
            let Some(cnt) = u32_at(v, 0).map(|c| c as usize) else { return Val::Raw(v.to_vec()) };
            let body = &v[4.min(v.len())..];
            match et {
                "CName" => {
                    let names: Option<Vec<String>> = (0..cnt).map(|i| u16_at(body, 2 * i).map(|x| self.name(x))).collect();
                    return names.map_or_else(|| Val::Raw(v.to_vec()), Val::Names);
                }
                "String" | "StringAnsi" => {
                    let mut o = 0;
                    let mut out = Vec::new();
                    for _ in 0..cnt {
                        let Some(&b) = body.get(o) else { return Val::Raw(v.to_vec()) };
                        let n = (b & 0x3f) as usize;
                        let (len, s) = if b & 0x80 != 0 {
                            (1 + n, body.get(o + 1..o + 1 + n).map(|s| String::from_utf8_lossy(s).into_owned()))
                        } else {
                            let u: Option<Vec<u16>> = (0..n).map(|i| u16_at(body, o + 1 + 2 * i)).collect();
                            (1 + 2 * n, u.map(|u| String::from_utf16_lossy(&u)))
                        };
                        let Some(s) = s else { return Val::Raw(v.to_vec()) };
                        out.push(s);
                        o += len;
                    }
                    return Val::Strs(out);
                }
                t if t.starts_with("ptr:") || t.starts_with("handle:") => {
                    let ptrs: Option<Vec<Option<usize>>> = (0..cnt)
                        .map(|i| u32_at(body, 4 * i).map(|x| if x as i32 > 0 { Some(x as usize - 1) } else { None }))
                        .collect();
                    return ptrs.map_or_else(|| Val::Raw(v.to_vec()), Val::Ptrs);
                }
                _ => {}
            }
            let esz = match et {
                "Uint16" | "Int16" => 2,
                "CGUID" => 16,
                "Bool" | "Uint8" | "Int8" => 1,
                "Float" | "Int32" | "Uint32" => 4,
                t if t.starts_with('E') => 2,
                _ => 0,
            };
            if esz > 0 {
                return Val::Raw(v.to_vec());
            }
            // Array of structs: each element is `0` + property list.
            let mut o = 0;
            let mut out = Vec::new();
            for _ in 0..cnt {
                if o >= body.len() {
                    return Val::Raw(v.to_vec());
                }
                if body[o] == 0 {
                    o += 1;
                }
                let Some((props, end)) = self.props(body, o) else { return Val::Raw(v.to_vec()) };
                out.push(props);
                o = end;
            }
            return Val::Structs(out);
        }
        // A struct value: `0` + property list that ends exactly at the value's end.
        if v.len() >= 3 && v[0] == 0 {
            if let Some((props, end)) = self.props(v, 1) {
                if end <= v.len() {
                    return Val::Struct(props);
                }
            }
        }
        Val::Raw(v.to_vec())
    }

    /// A property list starting at `o`: `Some((props, end))` when it is well formed.
    fn props(&self, d: &[u8], mut o: usize) -> Option<(Vec<Prop>, usize)> {
        let mut out = Vec::new();
        loop {
            let ni = u16_at(d, o)?;
            if ni == 0 {
                return Some((out, o + 2));
            }
            let ti = u16_at(d, o + 2)?;
            let sz = u32_at(d, o + 4)? as usize;
            if sz < 4 || o + 4 + sz > d.len() || ni as usize >= self.names.len() || ti as usize >= self.names.len() {
                return None;
            }
            let (name, ty) = (self.name(ni), self.name(ti));
            let value = self.value(&ty, &d[o + 8..o + 4 + sz]);
            out.push(Prop { name, ty, value });
            o += 4 + sz;
        }
    }
}

/// Parse one CR2W file.
pub fn parse(d: &[u8], what: &str) -> Result<Cr2w, Error> {
    let bad = |why: &str| Error::Format(format!("{what}: {why}"));
    if d.len() < 0x28 + 120 || &d[0..4] != b"CR2W" {
        return Err(bad("not a CR2W file"));
    }
    let table = |i: usize| -> Result<(usize, usize), Error> {
        Ok((u32_at(d, 0x28 + 12 * i).ok_or_else(|| bad("short header"))? as usize, u32_at(d, 0x2c + 12 * i).ok_or_else(|| bad("short header"))? as usize))
    };
    let (so, _) = table(0)?;
    let (no, nc) = table(1)?;
    let (io, ic) = table(2)?;
    let (eo, ec) = table(4)?;
    if no + 8 * nc > d.len() || io + 8 * ic > d.len() || eo + 24 * ec > d.len() {
        return Err(bad("table past the end of the file"));
    }
    let names: Vec<String> = (0..nc).map(|i| cstr(d, so + u32_at(d, no + 8 * i).unwrap_or(0) as usize)).collect();
    let imports: Vec<String> = (0..ic).map(|i| cstr(d, so + u32_at(d, io + 8 * i).unwrap_or(0) as usize)).collect();
    let r = Reader { names: &names, imports: &imports };
    let mut exports = Vec::with_capacity(ec);
    for i in 0..ec {
        let e = eo + 24 * i;
        let class = r.name(u16_at(d, e).unwrap_or(0));
        let parent = u32_at(d, e + 4).unwrap_or(0) as usize;
        let size = u32_at(d, e + 8).unwrap_or(0) as usize;
        let off = u32_at(d, e + 12).unwrap_or(0) as usize;
        if off + size > d.len() {
            return Err(bad(&format!("export #{i} ({class}) past the end of the file")));
        }
        let body = &d[off..off + size];
        // The property list starts after one leading byte; a body that does not parse is kept
        // as an object without properties (the walker only needs block classes it understands).
        let (props, tail) = match (body.len() > 1).then(|| r.props(body, 1)).flatten() {
            Some((p, end)) => (p, body[end.min(body.len())..].to_vec()),
            None => (Vec::new(), Vec::new()),
        };
        exports.push(Export { class, parent: parent.checked_sub(1), props, tail });
    }
    Ok(Cr2w { names, imports, exports })
}

impl Cr2w {
    /// Parse a property list (`{u16 name, u16 type, u32 size, value}.. 0`) that starts at the first
    /// byte of `d` (a `0` lead byte is skipped): the parameters of a scene script, a section event.
    /// `None` when `d` is not a well-formed list.
    pub fn props_in(&self, d: &[u8]) -> Option<Vec<Prop>> {
        let r = Reader { names: &self.names, imports: &self.imports };
        let start = usize::from(d.first() == Some(&0));
        r.props(d, start).map(|p| p.0)
    }
}

impl Export {
    pub fn prop(&self, name: &str) -> Option<&Val> {
        self.props.iter().find(|p| p.name == name).map(|p| &p.value)
    }

    pub fn str_prop(&self, name: &str) -> Option<&str> {
        match self.prop(name)? {
            Val::Str(s) | Val::Name(s) | Val::Import(s) | Val::Soft(s) => Some(s),
            _ => None,
        }
    }

    pub fn int_prop(&self, name: &str) -> Option<i64> {
        match self.prop(name)? {
            Val::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn guid(&self) -> Option<[u8; 16]> {
        match self.prop("guid")? {
            Val::Guid(g) => Some(*g),
            _ => None,
        }
    }
}

pub fn prop_of<'a>(props: &'a [Prop], name: &str) -> Option<&'a Val> {
    props.iter().find(|p| p.name == name).map(|p| &p.value)
}

/// Find a property by the end of its name: a few names in the cached-connection structs come out
/// of the names table shortened by two characters (`block` as `ock`, `inputName` as `putName`),
/// so lookups match on the suffix.
pub fn prop_ending<'a>(props: &'a [Prop], suffix: &str) -> Option<&'a Val> {
    props.iter().find(|p| p.name.ends_with(suffix)).map(|p| &p.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny synthetic CR2W: one name table, one export with two properties.
    fn build() -> Vec<u8> {
        let names = ["", "CFoo", "guid", "CGUID", "label", "String", "count", "Int32"];
        let mut strings = Vec::new();
        let mut offs = Vec::new();
        for n in names {
            offs.push(strings.len() as u32);
            strings.extend_from_slice(n.as_bytes());
            strings.push(0);
        }
        // body: 0, then props, then terminator
        let mut body = vec![0u8];
        let mut prop = |name: u16, ty: u16, val: &[u8]| {
            body.extend_from_slice(&name.to_le_bytes());
            body.extend_from_slice(&ty.to_le_bytes());
            body.extend_from_slice(&((val.len() + 4) as u32).to_le_bytes());
            body.extend_from_slice(val);
        };
        let g: [u8; 16] = [1; 16];
        prop(2, 3, &g);
        let mut s = vec![0x80 | 3];
        s.extend_from_slice(b"abc");
        prop(4, 5, &s);
        prop(6, 7, &7i32.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());

        let header_len = 0x28 + 120;
        let so = header_len;
        let no = so + strings.len();
        let eo = no + 8 * names.len();
        let bo = eo + 24;
        let mut d = vec![0u8; header_len];
        d[0..4].copy_from_slice(b"CR2W");
        let mut set = |i: usize, off: usize, cnt: usize| {
            d[0x28 + 12 * i..0x2c + 12 * i].copy_from_slice(&(off as u32).to_le_bytes());
            d[0x2c + 12 * i..0x30 + 12 * i].copy_from_slice(&(cnt as u32).to_le_bytes());
        };
        set(0, so, strings.len());
        set(1, no, names.len());
        set(2, no, 0);
        set(4, eo, 1);
        d.extend_from_slice(&strings);
        for o in offs {
            d.extend_from_slice(&o.to_le_bytes());
            d.extend_from_slice(&0u32.to_le_bytes());
        }
        let mut e = vec![0u8; 24];
        e[0..2].copy_from_slice(&1u16.to_le_bytes());
        e[4..8].copy_from_slice(&0u32.to_le_bytes());
        e[8..12].copy_from_slice(&(body.len() as u32).to_le_bytes());
        e[12..16].copy_from_slice(&(bo as u32).to_le_bytes());
        d.extend_from_slice(&e);
        d.extend_from_slice(&body);
        d
    }

    #[test]
    fn parses_names_exports_and_properties() {
        let c = parse(&build(), "test").unwrap();
        assert_eq!(c.names[1], "CFoo");
        assert_eq!(c.exports.len(), 1);
        let e = &c.exports[0];
        assert_eq!(e.class, "CFoo");
        assert_eq!(e.parent, None);
        assert_eq!(e.guid(), Some([1; 16]));
        assert_eq!(e.str_prop("label"), Some("abc"));
        assert_eq!(e.int_prop("count"), Some(7));
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(b"SNFHFZLC", "x").is_err());
    }
}
