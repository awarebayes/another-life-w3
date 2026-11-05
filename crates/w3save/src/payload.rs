//! Layer 2: the decompressed `SAV3` payload.
//!
//! Offsets stored *inside* the payload are absolute file offsets that include the
//! outer container header (`base`, normally 3,084), not offsets into the decoded
//! buffer. Every function here takes `base` and converts with [`rel`].
//!
//! Status of each claim (see docs/research.md):
//!   * footer = u32 variable-table offset + `SE`        – plan, 3 sources
//!   * two u32 (NM, RB offsets) at table offset - 10     – plan; trailing 2 bytes unknown
//!   * variable table = u32 count + count x (u32 offset, u32 size)
//!   * `RB` = tag, u32 count, count x (u16 name index, u32 offset) of the top-level `BS` nodes (proven)
//!   * `NM` + `MANU` name table (u8-length Latin-1 names)  – confirmed on 73 saves, see `read_names`
use serde::Serialize;

use crate::error::{Error, Result};
use crate::reader::Reader;

pub const MAGIC: &[u8; 4] = b"SAV3";
pub const FOOTER_LEN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VarEntry {
    pub offset: u32,
    pub size: u32,
}

/// One `RB` entry: the 1-based NM name index of a top-level `BS` and the absolute offset of
/// that `BS`. PROVEN on 118 of 118 fixture files: entry `i` is the `i`-th top-level `BS` in
/// file order (docs/research.md, 2026-11-04 "RB decoded"). Earlier code called `name` "size".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RbEntry {
    pub name: u16,
    pub offset: u32,
}

/// Everything locatable without understanding the variables themselves.
#[derive(Debug, Clone, Serialize)]
pub struct PayloadMap {
    pub base: u32,
    /// Three i32 after `SAV3`. Atvaark saw 54/10/162 on 1.x; 5.x reportedly
    /// saveVersion 66 / gameVersion 29 / buildPatch 0. Field order is UNVERIFIED.
    pub header_ints: [i32; 3],
    pub variable_table_offset: u32,
    pub nm_offset: u32,
    pub rb_offset: u32,
    /// The two bytes after the NM/RB offsets (probably a tag). Kept for Phase 0.
    pub pre_table_bytes: [u8; 2],
    pub variables: Vec<VarEntry>,
    /// Layout expectations that did not hold. Empty on a save that matches the plan.
    pub notes: Vec<String>,
}

/// Absolute file offset -> index into the decoded payload.
pub fn rel(offset: u32, base: u32, len: usize) -> Result<usize> {
    let r = offset
        .checked_sub(base)
        .ok_or_else(|| Error::Payload(format!("offset {offset} is below the container base {base}")))?
        as usize;
    if r > len {
        return Err(Error::Payload(format!("offset {offset} is past the end of the payload")));
    }
    Ok(r)
}

pub fn read_map(payload: &[u8], base: u32) -> Result<PayloadMap> {
    if payload.len() < 4 + 12 + FOOTER_LEN {
        return Err(Error::Truncated("payload shorter than header + footer".into()));
    }
    if &payload[..4] != MAGIC {
        return Err(Error::Payload(format!(
            "expected SAV3 magic, found {:?}",
            String::from_utf8_lossy(&payload[..4])
        )));
    }
    let mut hdr = Reader::at(payload, 4)?;
    let header_ints = [hdr.i32()?, hdr.i32()?, hdr.i32()?];

    // Footer is read first: the whole file is addressed from here.
    let mut foot = Reader::at(payload, payload.len() - FOOTER_LEN)?;
    let variable_table_offset = foot.u32()?;
    if foot.bytes(2)? != b"SE" {
        return Err(Error::Payload("footer does not end in `SE`".into()));
    }

    let vt = rel(variable_table_offset, base, payload.len())?;
    let pre = vt
        .checked_sub(10)
        .ok_or_else(|| Error::Payload("variable table offset leaves no room for the NM/RB pointers".into()))?;
    let mut p = Reader::at(payload, pre)?;
    let nm_offset = p.u32()?;
    let rb_offset = p.u32()?;
    let pre_table_bytes = [p.u8()?, p.u8()?];

    let mut t = Reader::at(payload, vt)?;
    let count = t.u32()? as usize;
    if count.checked_mul(8).is_none_or(|n| n > t.remaining()) {
        return Err(Error::Payload(format!("variable table claims {count} entries but the file is too short")));
    }
    let mut variables = Vec::with_capacity(count);
    for _ in 0..count {
        variables.push(VarEntry {
            offset: t.u32()?,
            size: t.u32()?,
        });
    }

    let mut notes = Vec::new();
    if t.remaining() != FOOTER_LEN {
        notes.push(format!(
            "{} bytes between the end of the variable table and the footer (expected {FOOTER_LEN} for the footer alone)",
            t.remaining() as isize - FOOTER_LEN as isize
        ));
    }

    Ok(PayloadMap {
        base,
        header_ints,
        variable_table_offset,
        nm_offset,
        rb_offset,
        pre_table_bytes,
        variables,
        notes,
    })
}

/// `RB`: tag, u32 count, count x (u16 NM name index, u32 absolute offset), one entry per
/// top-level `BS` in file order. The offsets move when anything before a block changes size;
/// `Document::rebase` rewrites them.
pub fn read_rb(payload: &[u8], map: &PayloadMap) -> Result<Vec<RbEntry>> {
    let at = rel(map.rb_offset, map.base, payload.len())?;
    let mut r = Reader::at(payload, at)?;
    r.expect_tag(b"RB")?;
    let count = r.u32()? as usize;
    if count.checked_mul(6).is_none_or(|n| n > r.remaining()) {
        return Err(Error::Payload(format!("RB claims {count} entries but the file is too short")));
    }
    (0..count)
        .map(|_| {
            Ok(RbEntry {
                name: r.u16()?,
                offset: r.u32()?,
            })
        })
        .collect()
}

/// Closing marker of the name section: the ASCII bytes of "DONE" reversed.
const NAME_TABLE_END: &[u8; 4] = b"ENOD";

/// `NM`: the name table that variables index (1-based) for their name and type.
///
/// Confirmed on 73 distinct real saves (118 files; 5.01 `saveVersion` 67 and older), see
/// docs/research.md. The section runs from the `NM` offset to the 10-byte pointer
/// block before the variable table (its `MANU` token is the last variable-table entry):
///
/// ```text
/// "NM" "MANU"          tags
/// u32 count            number of names
/// u32 0                unknown; 0 on every save, ignored
/// count x (u8 len, len bytes)   Latin-1 names, no terminator
/// u32 0                unknown; 0 on every save, ignored
/// "ENOD"
/// ```
///
/// The section must be consumed exactly; anything else is an error, not a partial table.
/// The layout was first described by Atvaark's W3SavegameEditor (MIT),
/// `ManuVariableParser`; the bytes of real saves decided every detail.
pub fn read_names(payload: &[u8], map: &PayloadMap) -> Result<Vec<String>> {
    let at = rel(map.nm_offset, map.base, payload.len())?;
    let end = rel(map.variable_table_offset, map.base, payload.len())?
        .checked_sub(10)
        .filter(|&e| e >= at)
        .ok_or_else(|| Error::Payload("NM offset is not before the pointer block".into()))?;
    parse_name_section(&payload[at..end])
}

/// Decode the bytes from the `NM` tag up to (not including) the pointer block.
pub fn parse_name_section(section: &[u8]) -> Result<Vec<String>> {
    let mut r = Reader::new(section);
    r.expect_tag(b"NM")?;
    r.expect_tag(b"MANU")?;
    let count = r.u32()? as usize;
    let _unknown = r.u32()?;
    // Every name takes at least its length byte.
    if count > r.remaining() {
        return Err(Error::Payload(format!("name table claims {count} names")));
    }
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let len = r.u8()? as usize;
        // Latin-1: each byte is the code point of the same value (`Azurite\xa0mineral`).
        names.push(r.bytes(len)?.iter().map(|&b| b as char).collect());
    }
    let _unknown = r.u32()?;
    r.expect_tag(NAME_TABLE_END)?;
    if r.remaining() != 0 {
        return Err(Error::Payload(format!(
            "{} unexpected bytes after ENOD in the name section",
            r.remaining()
        )));
    }
    Ok(names)
}

/// Four-letter tags from the plan. Anything else is read as a two-letter tag: reading
/// "up to four uppercase letters" wrongly swallows the u16 name index that follows a
/// two-letter tag (seen in the first Phase 0 dump: `BSA`, `VLB`, ...).
const LONG_TAGS: [&[u8; 4]; 7] = [b"MANU", b"AVAL", b"PORP", b"SXAP", b"BLCK", b"SBDF", b"EBDF"];

/// The leading ASCII tag of a variable (`BS`, `SS`, `VL`, `MANU`, ...), for the dump.
pub fn tag_of(bytes: &[u8]) -> String {
    if let Some(t) = LONG_TAGS.iter().find(|t| bytes.starts_with(&t[..])) {
        return String::from_utf8_lossy(&t[..]).into_owned();
    }
    // Only the four two-letter tags Atvaark documents (`BS`, `VL`, `OP`, `SS`). Any other pair of
    // uppercase letters is the start of an untagged sub-value (a packed string whose length byte
    // happens to be a letter, e.g. `Q` = 0x51) and counted as `?`.
    match bytes {
        [b'B', b'S', ..] | [b'V', b'L', ..] | [b'O', b'P', ..] | [b'S', b'S', ..] => {
            String::from_utf8_lossy(&bytes[..2]).into_owned()
        }
        _ => "?".into(),
    }
}

/// Synthetic fixtures, public so `w3al-core` tests can exercise export end to end
/// without a real save. Not part of the supported API.
#[doc(hidden)]
pub mod testutil {
    use super::*;

    /// Append `RB`, `NM`/`MANU`, the pointer block, the variable table and the footer to
    /// the payload built so far. Layout as proven on real saves (docs/research.md):
    /// `RB` lists (u16 NM name index, u32 absolute offset) of every top-level `BS`.
    fn finish(mut p: Vec<u8>, base: u32, rb_entries: &[(u16, u32)], mut vars: Vec<(u32, u32)>) -> Vec<u8> {
        let abs = |p: &Vec<u8>| base + p.len() as u32;
        // RB section, immediately before NM (as on real saves).
        let rb = abs(&p);
        p.extend_from_slice(b"RB");
        p.extend_from_slice(&(rb_entries.len() as u32).to_le_bytes());
        for (name, off) in rb_entries {
            p.extend_from_slice(&name.to_le_bytes());
            p.extend_from_slice(&off.to_le_bytes());
        }
        // NM section; its `MANU` token is the last variable-table entry.
        let nm = abs(&p);
        p.extend_from_slice(b"NM");
        let manu = abs(&p);
        p.extend_from_slice(b"MANU");
        p.extend_from_slice(&2u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        for name in ["questSystem", "facts"] {
            p.push(name.len() as u8);
            p.extend_from_slice(name.as_bytes());
        }
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(b"ENOD");
        vars.push((manu, abs(&p) - manu));
        // pre-table pointers
        p.extend_from_slice(&nm.to_le_bytes());
        p.extend_from_slice(&rb.to_le_bytes());
        p.extend_from_slice(b"SC");
        // variable table
        let vt = abs(&p);
        p.extend_from_slice(&(vars.len() as u32).to_le_bytes());
        for (o, s) in vars {
            p.extend_from_slice(&o.to_le_bytes());
            p.extend_from_slice(&s.to_le_bytes());
        }
        // footer
        p.extend_from_slice(&vt.to_le_bytes());
        p.extend_from_slice(b"SE");
        p
    }

    /// Build a self-consistent synthetic payload following the plan's layout. It
    /// proves our reader and writer agree with each other, NOT that the layout
    /// matches the game; only Phase 0 on a real save does that.
    pub fn synthetic(base: u32) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(MAGIC);
        for v in [66i32, 29, 0] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        let abs = |p: &Vec<u8>| base + p.len() as u32;
        let v0 = (abs(&p), 4u32);
        p.extend_from_slice(b"BS\x01\x00");
        let v1 = (abs(&p), 7u32);
        p.extend_from_slice(b"VL\x01\x00\x02\x00\x00");
        finish(p, base, &[(1, v0.0)], vec![v0, v1])
    }

    /// One fact for [`synthetic_with_facts`]: name, `c1`, records `(value, time, expiry)`.
    pub type SynthFact<'a> = (&'a str, u16, &'a [(u16, f32, f32)]);

    /// Like [`synthetic`] but with a real-shaped facts section: a top-level `BS` named
    /// `facts` (NM name 2) whose only child is the facts `SS`, then a second top-level `BS`
    /// named `questSystem` (NM name 1) holding one `VL`. Header ints are 5.01's.
    pub fn synthetic_with_facts(base: u32, facts: &[SynthFact]) -> Vec<u8> {
        let mut body = b"SXAP".to_vec();
        for v in [67i32, 29, 164] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(b"SBDF");
        body.extend_from_slice(&(facts.len() as u32).to_le_bytes());
        for (name, c1, recs) in facts {
            crate::packed::write(&mut body, name);
            body.extend_from_slice(&c1.to_le_bytes());
            body.extend_from_slice(&(recs.len() as u16).to_le_bytes());
            for (v, t, x) in *recs {
                body.extend_from_slice(&v.to_le_bytes());
                body.extend_from_slice(&t.to_le_bytes());
                body.extend_from_slice(&x.to_le_bytes());
            }
        }
        body.extend_from_slice(b"EBDF");
        let mut p = Vec::new();
        p.extend_from_slice(MAGIC);
        for v in [67i32, 29, 164] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        let abs = |p: &Vec<u8>| base + p.len() as u32;
        let ss_len = 6 + body.len() as u32;
        let facts_bs = (abs(&p), 4 + ss_len);
        let facts_ss = (facts_bs.0 + 4, ss_len);
        p.extend_from_slice(b"BS\x02\x00SS");
        p.extend_from_slice(&(body.len() as u32).to_le_bytes());
        p.extend_from_slice(&body);
        let quest_bs = (abs(&p), 4 + 7);
        let quest_vl = (quest_bs.0 + 4, 7);
        p.extend_from_slice(b"BS\x01\x00VL\x01\x00\x02\x00\x00");
        finish(p, base, &[(2, facts_bs.0), (1, quest_bs.0)], vec![facts_bs, facts_ss, quest_bs, quest_vl])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_synthetic_map() {
        let p = testutil::synthetic(3084);
        let m = read_map(&p, 3084).unwrap();
        assert_eq!(m.header_ints, [66, 29, 0]);
        assert_eq!(m.variables.len(), 3);
        assert!(m.notes.is_empty(), "{:?}", m.notes);
        assert_eq!(read_names(&p, &m).unwrap(), ["questSystem", "facts"]);
        assert_eq!(read_rb(&p, &m).unwrap(), [RbEntry { name: 1, offset: m.variables[0].offset }]);
    }

    #[test]
    fn reads_synthetic_facts_payload() {
        let p = testutil::synthetic_with_facts(3084, &[("a", 0, &[(1, 5.0, -1.0)])]);
        let m = read_map(&p, 3084).unwrap();
        assert_eq!(m.variables.len(), 5);
        assert!(m.notes.is_empty(), "{:?}", m.notes);
        let rb = read_rb(&p, &m).unwrap();
        assert_eq!(rb.iter().map(|e| e.name).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(rb[0].offset, m.variables[0].offset);
        assert_eq!(rb[1].offset, m.variables[2].offset);
        assert_eq!(read_names(&p, &m).unwrap(), ["questSystem", "facts"]);
    }

    /// Real bytes from a 5.01 save (`saveVersion` 67): the section header with the count
    /// patched from 5,554 to 3, the first three names, and the real tail.
    const REAL_NAMES: [u8; 39] = [
        0x4e, 0x4d, 0x4d, 0x41, 0x4e, 0x55, // NM MANU
        0x03, 0x00, 0x00, 0x00, // count (patched)
        0x00, 0x00, 0x00, 0x00, // unknown, 0 on every save
        0x06, 0x45, 0x6e, 0x74, 0x69, 0x74, 0x79, // "Entity"
        0x02, 0x6e, 0x50, // "nP"
        0x06, 0x55, 0x69, 0x6e, 0x74, 0x33, 0x32, // "Uint32"
        0x00, 0x00, 0x00, 0x00, 0x45, 0x4e, 0x4f, 0x44, // 0, "ENOD"
    ];

    #[test]
    fn decodes_real_name_section_bytes() {
        assert_eq!(parse_name_section(&REAL_NAMES).unwrap(), ["Entity", "nP", "Uint32"]);
    }

    #[test]
    fn names_are_latin_1() {
        // Real name `Azurite<NBSP>mineral` (byte 0xa0), wrapped in a one-name section.
        let mut s = b"NMMANU\x01\0\0\0\0\0\0\0".to_vec();
        s.extend_from_slice(b"\x0fAzurite\xa0mineral");
        s.extend_from_slice(b"\0\0\0\0ENOD");
        assert_eq!(parse_name_section(&s).unwrap(), ["Azurite\u{a0}mineral"]);
    }

    #[test]
    fn name_section_must_be_consumed_exactly() {
        // Missing marker, wrong marker, trailing byte, count too high, truncated name.
        assert!(parse_name_section(&REAL_NAMES[..REAL_NAMES.len() - 4]).is_err());
        let mut wrong = REAL_NAMES;
        wrong[REAL_NAMES.len() - 1] = b'X';
        assert!(parse_name_section(&wrong).is_err());
        let mut extra = REAL_NAMES.to_vec();
        extra.push(0);
        assert!(parse_name_section(&extra).is_err());
        let mut many = REAL_NAMES;
        many[6] = 4;
        assert!(parse_name_section(&many).is_err());
        let mut huge = REAL_NAMES;
        huge[9] = 0x7f;
        assert!(parse_name_section(&huge).is_err());
        let mut long = REAL_NAMES;
        long[14] = 0xff;
        assert!(parse_name_section(&long).is_err());
        assert!(parse_name_section(&[]).is_err());
    }


    #[test]
    fn tag_detection() {
        assert_eq!(tag_of(b"MANUxx"), "MANU");
        assert_eq!(tag_of(b"BS\x01"), "BS");
        assert_eq!(tag_of(b"\x01\x02"), "?");
    }

    #[test]
    fn malformed_payloads_error() {
        let p = testutil::synthetic(3084);
        assert!(read_map(&p[..10], 3084).is_err());
        let mut bad = p.clone();
        let n = bad.len();
        bad[n - 1] = b'X';
        assert!(read_map(&bad, 3084).is_err());
        // Wrong base shifts every pointer: must error, not panic.
        assert!(read_map(&p, 9_999_999).is_err());
    }
}
