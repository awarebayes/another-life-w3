//! The facts database: named integers the quest scripts query via `FactsQuerySum` /
//! `FactsSet`. This is the region `w3al` edits most.
//!
//! Layout CONFIRMED on 5.01 / 5.0 saves (see docs/research.md, "Facts blob"). The facts
//! live in ONE variable-table entry that is an `SS` token:
//!
//! ```text
//! "SS" u32 L                         L = length of everything below
//!   "SXAP" i32 i32 i32               67, 29, 164 on 5.01 (equal to the payload header ints)
//!   "SBDF" u32 N                     fact count
//!   N x fact:
//!     packed string                  the fact name (Latin-1, see `packed`; may hold spaces)
//!     u16 c1                         meaning inferred, see [`Fact::c1`]
//!     u16 n                          record count
//!     n x 10 bytes                   u16 value, f32 time, f32 expiry (see [`FactRecord`])
//!   "EBDF"                           nothing follows
//! ```
//!
//! The `SS` variable sits inside a `BS` variable (a tree node, which has no length of its
//! own). `L` is derived when the tree is written; the variable-table entries of the `BS`
//! ancestor and of everything after it are fixed by `Document::rebase`.
//!
//! Everything in this module is pure: no I/O, no hidden state. [`FactTable::parse`] and
//! [`FactTable::to_bytes`] round trip byte for byte on every save that parses.
use std::collections::HashMap;
use std::ops::Range;

use serde::Serialize;

use crate::diag::Issue;
use crate::error::{Error, Result};
use crate::packed;
use crate::payload::{self, PayloadMap};
use crate::reader::Reader;

const SXAP: &[u8; 4] = b"SXAP";
const SBDF: &[u8; 4] = b"SBDF";
const EBDF: &[u8; 4] = b"EBDF";
/// `SS` tag + u32 length in front of the body.
const SS_HEADER_LEN: usize = 6;
/// `SXAP` + 3 x i32: where the `SBDF` tag starts inside the body.
const SBDF_AT: usize = 4 + 12;
/// u16 value + f32 time + f32 expiry.
const RECORD_LEN: usize = 10;

/// One value/time record of a fact.
///
/// A fact is a *list* of these: a second `FactsAdd` on the same name appends a record
/// (seen on a real save: `q001_gwint_game_won` goes from one record to two), which is
/// why the script API is a *sum* (`FactsQuerySum`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FactRecord {
    /// The value this record adds. u16 on every save seen.
    pub value: u16,
    /// Game time (seconds, f32) at which the record was written. Observed values grow
    /// monotonically with play time (about 1,100 to 2,400 on early-game saves).
    pub time: f32,
    /// `-1.0` = permanent (the usual case), `-2.0` on dialogue-id facts such as
    /// `83805989_2_1`, otherwise an absolute game time at which the record lapses
    /// (observed: `time` + 1..15).
    pub expiry: f32,
}

impl FactRecord {
    pub const PERMANENT: f32 = -1.0;

    pub fn is_permanent(&self) -> bool {
        self.expiry == Self::PERMANENT
    }

    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            value: r.u16()?,
            time: f32::from_bits(r.u32()?),
            expiry: f32::from_bits(r.u32()?),
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.value.to_le_bytes());
        out.extend_from_slice(&self.time.to_bits().to_le_bytes());
        out.extend_from_slice(&self.expiry.to_bits().to_le_bytes());
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fact {
    /// Latin-1 identifier; never widened to UTF-8 (5.0.0c bug).
    pub name: String,
    /// The u16 between the name and the record count. Kept raw so it round trips.
    ///
    /// INFERRED (not from a script): `c1` equals the number of records whose expiry is
    /// `>= 0` (a real expiry time, i.e. neither permanent `-1` nor dialogue `-2`). Holds
    /// for 57,955 of 57,955 facts on 45 consecutive 5.01 saves; a fact never mixes
    /// timed and permanent records. See [`Fact::timed_count`].
    pub c1: u16,
    pub records: Vec<FactRecord>,
}

impl Fact {
    /// Records with a real expiry time (`expiry >= 0`). Equals `c1` on every real fact.
    pub fn timed_count(&self) -> usize {
        self.records.iter().filter(|r| r.expiry >= 0.0).count()
    }

    /// Sum of all record values, read as signed 16-bit: what `FactsQuerySum` is expected
    /// to return. Scripts add negative values on purpose (`FactsAdd(f, -1, -1)` in `riddleNode.ws`,
    /// `FactsAdd(factID, -sum)` in `ResetFactQuest`), so a record can be negative; the sum is a
    /// script `int`. HYPOTHESIS: the on-disk field is i16 (118 of ~140k records hold 65535, which
    /// reads as -1; a value outside `-32768..=65535` cannot be stored) and lapsed records are
    /// ignored at query time, not here.
    pub fn sum(&self) -> i64 {
        self.records.iter().map(|r| r.value as i16 as i64).sum()
    }

    fn refresh_c1(&mut self) {
        self.c1 = self.timed_count().min(u16::MAX as usize) as u16;
    }
}

/// What [`FactTable::set_value`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOutcome {
    /// The name was new: a fact was appended at the end of the table.
    Created,
    /// The fact existed (possibly with zero records); its records were replaced.
    Replaced { previous_records: usize },
}

/// Convert a script-level value to the raw u16 of a record. HYPOTHESIS: the field is
/// signed 16-bit, so `-32768..=65535` is accepted (negative values wrap to their i16 bits).
fn raw_value(value: i32) -> Result<u16> {
    match value {
        0..=65535 => Ok(value as u16),
        -32768..=-1 => Ok(value as i16 as u16),
        _ => Err(Error::Payload(format!("fact value {value} does not fit a 16-bit record field"))),
    }
}

/// A name we are willing to write: non-empty, Latin-1 only (never widen to UTF-16/UTF-8,
/// 5.0.0c bug) and not longer than [`MAX_PLAUSIBLE_ID_LEN`].
fn check_new_name(name: &str) -> Result<()> {
    if name.is_empty() || name.chars().count() > MAX_PLAUSIBLE_ID_LEN || name.chars().any(|c| c as u32 > 0xFF) {
        return Err(Error::Payload(format!(
            "{name:?} is not a writable fact name (empty, over {MAX_PLAUSIBLE_ID_LEN} chars, or not Latin-1)"
        )));
    }
    Ok(())
}

/// Parsed facts blob. Facts keep their file order; unknown fields are kept raw.
#[derive(Debug, Clone, PartialEq)]
pub struct FactTable {
    /// The three ints after `SXAP` (67, 29, 164 on 5.01; 66, 29, 164 on 5.0). Kept raw.
    pub sxap: [i32; 3],
    facts: Vec<Fact>,
    index: HashMap<String, usize>,
}

impl FactTable {
    /// Parse the body of the facts `SS` variable (from `SXAP` to the end of `EBDF`, i.e.
    /// without the `SS` tag and its u32 length). Strict: the declared count must match,
    /// nothing may be left over, names must be unique and every name must be in the
    /// canonical packed-string form so that [`FactTable::to_bytes`] is byte-exact.
    pub fn parse(body: &[u8]) -> Result<FactTable> {
        let mut r = Reader::new(body);
        r.expect_tag(SXAP)?;
        let sxap = [r.i32()?, r.i32()?, r.i32()?];
        r.expect_tag(SBDF)?;
        let count = r.u32()? as usize;
        // Every fact takes at least 1 (name) + 2 + 2 bytes.
        if count > r.remaining() / 5 {
            return Err(Error::Payload(format!("fact table claims {count} facts but the data is too short")));
        }
        let mut facts = Vec::with_capacity(count);
        let mut index = HashMap::with_capacity(count);
        let mut scratch = Vec::new();
        for i in 0..count {
            let start = r.pos();
            let name = packed::read(&mut r)?;
            // Byte-exact guarantee: our writer must reproduce what the game wrote.
            scratch.clear();
            packed::write(&mut scratch, &name);
            if scratch != body[start..r.pos()] {
                return Err(Error::Payload(format!(
                    "fact #{i} {name:?}: name is not in canonical packed-string form (a round trip would change bytes)"
                )));
            }
            let c1 = r.u16()?;
            let n = r.u16()? as usize;
            if n * RECORD_LEN > r.remaining() {
                return Err(Error::Truncated(format!("fact {name:?} claims {n} records but the data ends")));
            }
            let records = (0..n).map(|_| FactRecord::read(&mut r)).collect::<Result<Vec<_>>>()?;
            if index.insert(name.clone(), i).is_some() {
                return Err(Error::Payload(format!("duplicate fact name {name:?}")));
            }
            facts.push(Fact { name, c1, records });
        }
        r.expect_tag(EBDF)?;
        if r.remaining() != 0 {
            return Err(Error::Payload(format!("{} unexpected bytes after EBDF", r.remaining())));
        }
        Ok(FactTable { sxap, facts, index })
    }

    /// Serialize the body (see [`FactTable::parse`]). The caller owns the enclosing `SS`
    /// tag and length, which must equal `to_bytes().len()`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SBDF_AT + 8 + self.facts.len() * 40);
        out.extend_from_slice(SXAP);
        for v in self.sxap {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(SBDF);
        out.extend_from_slice(&(self.facts.len() as u32).to_le_bytes());
        for f in &self.facts {
            packed::write(&mut out, &f.name);
            out.extend_from_slice(&f.c1.to_le_bytes());
            out.extend_from_slice(&(f.records.len() as u16).to_le_bytes());
            for rec in &f.records {
                rec.write(&mut out);
            }
        }
        out.extend_from_slice(EBDF);
        out
    }

    pub fn len(&self) -> usize {
        self.facts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// Facts in file order.
    pub fn facts(&self) -> &[Fact] {
        &self.facts
    }

    pub fn get(&self, name: &str) -> Option<&Fact> {
        self.index.get(name).map(|&i| &self.facts[i])
    }

    /// Run the 5.0.0c damage check ([`check_ids`]) over the real fact names.
    pub fn check_ids(&self) -> Vec<Issue> {
        check_ids(self.facts.iter().map(|f| f.name.as_str()))
    }

    /// The complete replacement for the facts variable: `SS` + u32 body length + body.
    /// Splice this over the old variable, then rebase (variable table, `RB`, NM pointer,
    /// footer; the only ancestor is the `BS` node, which has no length of its own).
    pub fn to_ss_variable(&self) -> Vec<u8> {
        let body = self.to_bytes();
        let mut out = Vec::with_capacity(SS_HEADER_LEN + body.len());
        out.extend_from_slice(b"SS");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Script-style `FactsSet(name, value)`: afterwards the fact has exactly ONE record
    /// `{value, time, expiry: permanent}` and `c1 = 0`.
    ///
    /// Evidence (lukwraith saves): `CurrentlySelectedSign` goes `[1 t=1546.2 x=-1] ->
    /// [2 t=1845.0 x=-1]` and `statistic_finesse_kills` `[0 t=1557.9] -> [0 t=1809.3]`
    /// between two saves, i.e. a set replaces the record list rather than appending.
    /// SCRIPT EVIDENCE (`scripts/game/facts.ws`, `function FactsSet`): `FactsRemove(ID);` then
    /// `if(validFor == 0) validFor = -1;` then `FactsAdd(ID, val, validFor);`, i.e. the fact ends up with
    /// ONE record that is permanent (expiry -1). `FactsAdd`, `FactsRemove` and the queries are
    /// `import function`s (native), so how the engine stamps `time`, derives `c1` and stores a negative
    /// value stays a HYPOTHESIS backed by saves only (a game-written `FactsSet` result is on disk
    /// already: `gwent_difficulty c1=0 [v=1 t=22079.6 x=-1.0]`, `ingameMenu.ws` sets it from the options menu).
    ///
    /// A missing name is appended at the END of the table. Fact order in real saves is
    /// not stable between saves (new facts appear at arbitrary positions: hash order),
    /// so position is assumed not to matter. A zero-record fact (the game leaves these
    /// behind: 1,211 of 1,992 on the last lukwraith save) is reused, not duplicated.
    ///
    /// `time` is the game time to stamp the record with; pick a value near the template
    /// save's own times (the field is game seconds).
    pub fn set_value(&mut self, name: &str, value: i32, time: f32) -> Result<SetOutcome> {
        let rec = FactRecord { value: raw_value(value)?, time: finite(time)?, expiry: FactRecord::PERMANENT };
        match self.index.get(name) {
            Some(&i) => {
                let f = &mut self.facts[i];
                let previous_records = f.records.len();
                f.records = vec![rec];
                f.refresh_c1();
                Ok(SetOutcome::Replaced { previous_records })
            }
            None => {
                check_new_name(name)?;
                self.push_new(Fact { name: name.to_string(), c1: 0, records: vec![rec] });
                Ok(SetOutcome::Created)
            }
        }
    }

    /// Script-style `FactsAdd`: append one record to the fact (creating it if missing) and
    /// recompute `c1` with the confirmed-by-observation rule (number of records with
    /// `expiry >= 0`). Evidence: `player_had_quen` and `tutorial_player_moved` gain records across saves
    /// with the old ones untouched. Errors past 65,535 records.
    pub fn add_record(&mut self, name: &str, value: i32, time: f32, expiry: f32) -> Result<()> {
        let rec = FactRecord { value: raw_value(value)?, time: finite(time)?, expiry: finite(expiry)? };
        match self.index.get(name) {
            Some(&i) => {
                let f = &mut self.facts[i];
                if f.records.len() >= u16::MAX as usize {
                    return Err(Error::Payload(format!("fact {name:?} already has the maximum number of records")));
                }
                f.records.push(rec);
                f.refresh_c1();
            }
            None => {
                check_new_name(name)?;
                let mut f = Fact { name: name.to_string(), c1: 0, records: vec![rec] };
                f.refresh_c1();
                self.push_new(f);
            }
        }
        Ok(())
    }

    /// Exact-state helper: afterwards the fact holds exactly `records`, in that order
    /// (creating it at the end of the table when missing). `c1` is recomputed with the
    /// inferred rule (number of records with `expiry >= 0`), which held on every real fact
    /// seen, so a record list copied from a real save reproduces that save's `c1` too; use
    /// [`FactTable::set_fact`] to copy a fact verbatim including `c1`.
    ///
    /// Unlike [`FactTable::set_value`] nothing is stamped or replaced by "one permanent
    /// record": the caller supplies the complete list (zero records is allowed, the game
    /// leaves such facts behind).
    pub fn set_records(&mut self, name: &str, records: Vec<FactRecord>) -> Result<SetOutcome> {
        if records.len() > u16::MAX as usize {
            return Err(Error::Payload(format!("fact {name:?}: {} records do not fit the u16 count", records.len())));
        }
        for r in &records {
            finite(r.time)?;
            finite(r.expiry)?;
        }
        match self.index.get(name) {
            Some(&i) => {
                let f = &mut self.facts[i];
                let previous_records = f.records.len();
                f.records = records;
                f.refresh_c1();
                Ok(SetOutcome::Replaced { previous_records })
            }
            None => {
                check_new_name(name)?;
                let mut f = Fact { name: name.to_string(), c1: 0, records };
                f.refresh_c1();
                self.push_new(f);
                Ok(SetOutcome::Created)
            }
        }
    }

    /// Copy a whole fact, `c1` and every record bit pattern included (no recomputation, no
    /// finiteness check: this reproduces what a real save held). An existing fact keeps its
    /// position in the table; a missing one is appended at the end.
    pub fn set_fact(&mut self, fact: Fact) -> Result<SetOutcome> {
        if fact.records.len() > u16::MAX as usize {
            return Err(Error::Payload(format!("fact {:?}: too many records for the u16 count", fact.name)));
        }
        match self.index.get(&fact.name) {
            Some(&i) => {
                let previous_records = self.facts[i].records.len();
                self.facts[i] = fact;
                Ok(SetOutcome::Replaced { previous_records })
            }
            None => {
                check_new_name(&fact.name)?;
                self.push_new(fact);
                Ok(SetOutcome::Created)
            }
        }
    }

    /// Remove a fact; the others keep their relative order. `None` when it is not there.
    /// HYPOTHESIS: the game treats a missing fact like a fact with zero records (real saves
    /// hold both; 1,211 of 1,992 facts on the last lukwraith save have zero records).
    pub fn remove(&mut self, name: &str) -> Option<Fact> {
        let i = self.index.remove(name)?;
        let f = self.facts.remove(i);
        for v in self.index.values_mut() {
            if *v > i {
                *v -= 1;
            }
        }
        Some(f)
    }

    /// Names whose fact differs between the two tables (changed, or present in only one),
    /// sorted. File order is ignored. Records compare bit for bit, so NaN does not self-differ.
    pub fn differing(&self, other: &FactTable) -> Vec<String> {
        fn same(a: &Fact, b: &Fact) -> bool {
            a.name == b.name
                && a.c1 == b.c1
                && a.records.len() == b.records.len()
                && a.records.iter().zip(&b.records).all(|(x, y)| {
                    x.value == y.value && x.time.to_bits() == y.time.to_bits() && x.expiry.to_bits() == y.expiry.to_bits()
                })
        }
        let mut out: Vec<String> = self
            .facts
            .iter()
            .filter(|f| other.get(&f.name).is_none_or(|g| !same(f, g)))
            .map(|f| f.name.clone())
            .collect();
        out.extend(other.facts.iter().filter(|g| self.get(&g.name).is_none()).map(|g| g.name.clone()));
        out.sort();
        out
    }

    /// Make this table's fact `name` exactly what `source` has: copy it verbatim when
    /// `source` has it, remove it when it does not. Returns true when the table changed.
    pub fn match_fact(&mut self, name: &str, source: &FactTable) -> Result<bool> {
        match source.get(name) {
            Some(f) => {
                let changed = self.get(name) != Some(f);
                self.set_fact(f.clone())?;
                Ok(changed)
            }
            None => Ok(self.remove(name).is_some()),
        }
    }

    /// The latest game time stamped on any record (0.0 for an empty table). It is a lower
    /// bound of the save's own clock (records are stamped when written), so a record stamped
    /// with it is never in the future of the save. HYPOTHESIS: this is an acceptable `time`
    /// for a fact we add; the real clock lives in `timeManger` > `time:GameTime`, which is not
    /// decoded yet.
    pub fn latest_time(&self) -> f32 {
        self.facts
            .iter()
            .flat_map(|f| f.records.iter())
            .map(|r| r.time)
            .filter(|t| t.is_finite())
            .fold(0.0, f32::max)
    }

    fn push_new(&mut self, f: Fact) {
        self.index.insert(f.name.clone(), self.facts.len());
        self.facts.push(f);
    }
}

fn finite(x: f32) -> Result<f32> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err(Error::Payload("fact record time/expiry must be a finite number".into()))
    }
}

/// Locate the facts `SS` variable and return the range of its **body** in `payload`
/// (from `SXAP` to the end of `EBDF`). The `SS` tag is at `range.start - 6` and the u32
/// body length at `range.start - 4`.
///
/// Scans the variable table for an `SS`-tagged variable whose length field matches its
/// table size and whose body starts `SXAP ... SBDF`. Exactly one must exist; none or
/// several is an error (never a guess).
pub fn find_facts_blob(payload: &[u8], map: &PayloadMap) -> Result<Range<usize>> {
    let mut found: Option<Range<usize>> = None;
    for (i, v) in map.variables.iter().enumerate() {
        if (v.size as usize) < SS_HEADER_LEN + SBDF_AT + 8 {
            continue;
        }
        let Ok(start) = payload::rel(v.offset, map.base, payload.len()) else { continue };
        let Some(end) = start.checked_add(v.size as usize).filter(|&e| e <= payload.len()) else { continue };
        let var = &payload[start..end];
        if !var.starts_with(b"SS") || &var[SS_HEADER_LEN..SS_HEADER_LEN + 4] != SXAP {
            continue;
        }
        if &var[SS_HEADER_LEN + SBDF_AT..SS_HEADER_LEN + SBDF_AT + 4] != SBDF {
            continue;
        }
        let declared = u32::from_le_bytes(var[2..6].try_into().unwrap()) as usize;
        if declared != var.len() - SS_HEADER_LEN {
            return Err(Error::Payload(format!(
                "facts SS variable #{i}: length field {declared} does not match its table size {}",
                var.len() - SS_HEADER_LEN
            )));
        }
        if found.replace(start + SS_HEADER_LEN..end).is_some() {
            return Err(Error::Payload("more than one SS variable looks like the facts blob".into()));
        }
    }
    found.ok_or_else(|| Error::Payload("no SS variable with an SXAP/SBDF body (facts blob not found)".into()))
}

/// Locate and parse the facts of a decoded payload.
pub fn parse_payload(payload: &[u8], map: &PayloadMap) -> Result<FactTable> {
    let body = find_facts_blob(payload, map)?;
    FactTable::parse(&payload[body])
}

/// Identifiers longer than this are suspicious. Real fact names are short
/// (`q103_botchling_saved`); the 5.0.0c bug grew them up to 512x.
pub const MAX_PLAUSIBLE_ID_LEN: usize = 128;

/// Detect the 5.0.0c Latin-1 -> UTF-8 re-encoding damage in fact identifiers.
pub fn check_ids<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<Issue> {
    let mut out = Vec::new();
    let (mut long, mut mojibake) = (0usize, 0usize);
    let mut example = None;
    for n in names {
        let chars = n.chars().count();
        // "Ã" / "Â" followed by a C1 control or Latin-1 char is the UTF-8-read-as-Latin-1 fingerprint.
        let looks_mojibake = n.contains('\u{c3}') || n.contains('\u{c2}');
        if chars > MAX_PLAUSIBLE_ID_LEN {
            long += 1;
            example.get_or_insert_with(|| n.chars().take(40).collect::<String>());
        }
        if looks_mojibake {
            mojibake += 1;
            example.get_or_insert_with(|| n.chars().take(40).collect::<String>());
        }
    }
    if long > 0 {
        out.push(Issue::error(format!(
            "{long} fact identifier(s) longer than {MAX_PLAUSIBLE_ID_LEN} chars (5.0.0c re-encoding bug); e.g. {:?}...",
            example.clone().unwrap_or_default()
        )));
    }
    if mojibake > 0 {
        out.push(Issue::warn(format!(
            "{mojibake} fact identifier(s) contain mojibake characters (possible Latin-1 -> UTF-8 damage)"
        )));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_names_pass() {
        assert!(check_ids(["q103_botchling_saved", "ciri_alive"]).is_empty());
    }

    #[test]
    fn oversized_and_mojibake_names_are_flagged() {
        let long = "q".repeat(200);
        let issues = check_ids([long.as_str(), "caf\u{c3}\u{a9}"]);
        assert_eq!(issues.len(), 2);
    }

    /// Real bytes (5.01 save, lukwraith 06): `bugfix_132404`, c1 0, one permanent record
    /// `{v 1, t 1130.2047, x -1}`.
    const REAL_PERM: [u8; 28] = [
        0x8d, 0x62, 0x75, 0x67, 0x66, 0x69, 0x78, 0x5f, 0x31, 0x33, 0x32, 0x34, 0x30, 0x34, // name
        0x00, 0x00, 0x01, 0x00, // c1 = 0, n = 1
        0x01, 0x00, 0x8d, 0x46, 0x8d, 0x44, 0x00, 0x00, 0x80, 0xbf, // v, time, expiry
    ];
    /// Real bytes: `ach_attack`, c1 1, one record with a real expiry (time + 4).
    const REAL_TIMED: [u8; 25] = [
        0x8a, 0x61, 0x63, 0x68, 0x5f, 0x61, 0x74, 0x74, 0x61, 0x63, 0x6b, // name
        0x01, 0x00, 0x01, 0x00, // c1 = 1, n = 1
        0x01, 0x00, 0xf1, 0x56, 0xec, 0x44, 0xf1, 0xd6, 0xec, 0x44,
    ];
    /// Real bytes: `83805989_2_1`, a dialogue-id fact with expiry -2.
    const REAL_DIALOGUE: [u8; 27] = [
        0x8c, 0x38, 0x33, 0x38, 0x30, 0x35, 0x39, 0x38, 0x39, 0x5f, 0x32, 0x5f, 0x31, // name
        0x00, 0x00, 0x01, 0x00, // c1 = 0, n = 1
        0x01, 0x00, 0xe1, 0x04, 0x16, 0x45, 0x00, 0x00, 0x00, 0xc0,
    ];

    /// A body in the real layout: the real SXAP header, three real facts, EBDF.
    fn sample_body() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"SXAP");
        for v in [67i32, 29, 164] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(b"SBDF");
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&REAL_PERM);
        b.extend_from_slice(&REAL_TIMED);
        b.extend_from_slice(&REAL_DIALOGUE);
        b.extend_from_slice(b"EBDF");
        b
    }

    #[test]
    fn parses_real_fact_bytes() {
        let t = FactTable::parse(&sample_body()).unwrap();
        assert_eq!(t.sxap, [67, 29, 164]);
        assert_eq!(t.len(), 3);
        let f = t.get("bugfix_132404").unwrap();
        assert_eq!((f.c1, f.records.len()), (0, 1));
        assert_eq!(f.records[0].value, 1);
        assert_eq!(f.records[0].time, 1130.2047);
        assert!(f.records[0].is_permanent());
        let f = t.get("ach_attack").unwrap();
        assert_eq!((f.c1 as usize, f.timed_count()), (1, 1));
        assert_eq!(f.records[0].expiry - f.records[0].time, 4.0);
        assert_eq!(t.get("83805989_2_1").unwrap().records[0].expiry, -2.0);
        assert_eq!(t.facts()[2].name, "83805989_2_1", "file order is kept");
        assert!(t.get("nope").is_none());
        assert!(t.check_ids().is_empty());
    }

    #[test]
    fn round_trip_is_byte_exact() {
        let body = sample_body();
        let t = FactTable::parse(&body).unwrap();
        assert_eq!(t.to_bytes(), body);
        let ss = t.to_ss_variable();
        assert_eq!(&ss[..2], b"SS");
        assert_eq!(u32::from_le_bytes(ss[2..6].try_into().unwrap()) as usize, body.len());
        assert_eq!(&ss[6..], &body[..]);
    }

    #[test]
    fn unknown_c1_and_odd_floats_survive() {
        // c1 that breaks our inferred rule, and NaN / denormal time bits, must round trip.
        let mut body = sample_body();
        let c1_at = 4 + 12 + 4 + 4 + 14;
        body[c1_at] = 0x07;
        let time_at = c1_at + 4 + 2;
        body[time_at..time_at + 4].copy_from_slice(&0x7fc0_0001u32.to_le_bytes());
        let t = FactTable::parse(&body).unwrap();
        assert_eq!(t.facts()[0].c1, 7);
        assert_eq!(t.to_bytes(), body);
    }

    #[test]
    fn empty_fact_and_names_with_spaces_round_trip() {
        let mut body = b"SXAP".to_vec();
        for v in [67i32, 29, 164] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(b"SBDF\x02\0\0\0");
        // zero-record fact, then a name with spaces and a Latin-1 byte
        body.extend_from_slice(b"\x8aBookRead_x\x00\x00\x00\x00");
        body.extend_from_slice(b"\x8aBook\xa0a b c\x00\x00\x00\x00");
        body.extend_from_slice(b"EBDF");
        let t = FactTable::parse(&body).unwrap();
        assert!(t.get("BookRead_x").unwrap().records.is_empty());
        assert!(t.get("Book\u{a0}a b c").is_some());
        assert_eq!(t.to_bytes(), body);
    }

    #[test]
    fn malformed_bodies_error_not_panic() {
        let good = sample_body();
        // truncated at every length
        for n in 0..good.len() {
            assert!(FactTable::parse(&good[..n]).is_err(), "prefix {n} must not parse");
        }
        // trailing byte, wrong end tag, count too high / huge, duplicate name
        let mut extra = good.clone();
        extra.push(0);
        assert!(FactTable::parse(&extra).is_err());
        let mut bad_end = good.clone();
        let n = bad_end.len();
        bad_end[n - 1] = b'X';
        assert!(FactTable::parse(&bad_end).is_err());
        let mut many = good.clone();
        many[20] = 4;
        assert!(FactTable::parse(&many).is_err());
        many[23] = 0x7f;
        assert!(FactTable::parse(&many).is_err());
        let mut dup = good.clone();
        let (a, b) = (4 + 12 + 8, 4 + 12 + 8 + REAL_PERM.len());
        let first = good[a..b].to_vec();
        dup.splice(b..b, first); // second copy of bugfix_132404
        dup[20] = 4;
        assert!(FactTable::parse(&dup).is_err());
        // a Latin-1 name stored as UTF-16 is not canonical: refuse rather than change bytes
        let mut wide = b"SXAP".to_vec();
        for v in [67i32, 29, 164] {
            wide.extend_from_slice(&v.to_le_bytes());
        }
        wide.extend_from_slice(b"SBDF\x01\0\0\0\x01a\0\x00\x00\x00\x00EBDF");
        assert!(FactTable::parse(&wide).is_err());
    }

    #[test]
    fn set_value_creates_then_replaces() {
        let mut t = FactTable::parse(&sample_body()).unwrap();
        // existing permanent fact: replaced by ONE record with the new value and time
        assert_eq!(t.set_value("bugfix_132404", 2, 2000.0).unwrap(), SetOutcome::Replaced { previous_records: 1 });
        let f = t.get("bugfix_132404").unwrap();
        assert_eq!(f.records, [FactRecord { value: 2, time: 2000.0, expiry: -1.0 }]);
        assert_eq!(f.c1, 0);
        // timed fact: the timed record goes away, so c1 drops to 0
        assert_eq!(t.get("ach_attack").unwrap().c1, 1);
        t.set_value("ach_attack", 0, 1900.0).unwrap();
        assert_eq!(t.get("ach_attack").unwrap().c1, 0);
        assert_eq!(t.get("ach_attack").unwrap().sum(), 0);
        // new fact: appended at the end
        assert_eq!(t.set_value("q_new_fact", 1, 2001.5).unwrap(), SetOutcome::Created);
        assert_eq!(t.len(), 4);
        assert_eq!(t.facts()[3].name, "q_new_fact");
        assert_eq!(t.get("q_new_fact").unwrap().sum(), 1);
        // and the result re-parses to the same table
        let again = FactTable::parse(&t.to_bytes()).unwrap();
        assert_eq!(again, t);
    }

    /// The game's own script defines `FactsSet(id, val, validFor)` as `FactsRemove(id)` followed by
    /// `FactsAdd(id, val, validFor)` with `validFor == 0` turned into `-1` (permanent)
    /// (`scripts/game/facts.ws`, read 2026-11-04). So after a set the fact has exactly one record whatever it
    /// held before, and `set_value` must equal "remove, then add one permanent record" (except for the table
    /// position, which is hash order in real saves). `FactsSubstract` is `FactsSet(id, Max(0, sum - val))`.
    #[test]
    fn set_value_equals_script_remove_then_add() {
        let mut base = FactTable::parse(&sample_body()).unwrap();
        // a fact with several records, one of them timed (like `player_had_quen`)
        base.set_records("multi", vec![rec(1, 10.0, -1.0), rec(2, 11.0, -1.0), rec(1, 12.0, 15.0)]).unwrap();
        assert_eq!((base.get("multi").unwrap().sum(), base.get("multi").unwrap().c1), (4, 1));

        let mut by_set = base.clone();
        by_set.set_value("multi", 7, 99.0).unwrap();
        let mut by_script = base.clone();
        by_script.remove("multi"); // FactsRemove
        by_script.add_record("multi", 7, 99.0, FactRecord::PERMANENT).unwrap(); // FactsAdd(.., 7, -1)
        assert_eq!(by_set.get("multi"), by_script.get("multi"));
        assert_eq!(by_set.get("multi").unwrap().records, [rec(7, 99.0, -1.0)]);
        assert_eq!(by_set.get("multi").unwrap().c1, 0);

        // FactsSubstract(id, 3) on a sum of 4 is FactsSet(id, 1); on a sum of 2 it is FactsSet(id, 0): a real v=0 record
        let sum = base.get("multi").unwrap().sum();
        let mut sub = base.clone();
        sub.set_value("multi", (sum - 3).max(0) as i32, 99.0).unwrap();
        assert_eq!(sub.get("multi").unwrap().sum(), 1);
        let cur = sub.get("multi").unwrap().sum();
        sub.set_value("multi", (cur - 3).max(0) as i32, 100.0).unwrap();
        assert_eq!(sub.get("multi").unwrap().records, [rec(0, 100.0, -1.0)]);

        // ResetFactQuest is `FactsAdd(id, -sum)`: ANOTHER record with the negative sum, so the list grows and the sum is 0
        let mut reset = base.clone();
        reset.add_record("multi", -(sum as i32), 99.0, FactRecord::PERMANENT).unwrap();
        assert_eq!((reset.get("multi").unwrap().records.len(), reset.get("multi").unwrap().sum()), (4, 0));
    }

    #[test]
    fn set_value_reuses_zero_record_facts() {
        let mut body = b"SXAP".to_vec();
        for v in [67i32, 29, 164] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(b"SBDF\x01\0\0\0\x83abc\x00\x00\x00\x00EBDF");
        let mut t = FactTable::parse(&body).unwrap();
        assert_eq!(t.set_value("abc", 1, 10.0).unwrap(), SetOutcome::Replaced { previous_records: 0 });
        assert_eq!(t.len(), 1);
        assert_eq!(t.get("abc").unwrap().records.len(), 1);
    }

    #[test]
    fn add_record_appends_and_recomputes_c1() {
        let mut t = FactTable::parse(&sample_body()).unwrap();
        t.add_record("bugfix_132404", 1, 2100.0, FactRecord::PERMANENT).unwrap();
        let f = t.get("bugfix_132404").unwrap();
        assert_eq!((f.records.len(), f.c1, f.sum()), (2, 0, 2));
        assert_eq!(f.records[0].time, 1130.2047, "old record untouched");
        t.add_record("ach_attack", 1, 1950.0, 1954.0).unwrap();
        assert_eq!(t.get("ach_attack").unwrap().c1, 2);
        t.add_record("brand_new", 1, 5.0, 8.0).unwrap();
        assert_eq!(t.get("brand_new").unwrap().c1, 1);
        assert_eq!(FactTable::parse(&t.to_bytes()).unwrap(), t);
    }

    fn rec(value: u16, time: f32, expiry: f32) -> FactRecord {
        FactRecord { value, time, expiry }
    }

    #[test]
    fn set_records_reproduces_an_exact_list() {
        let mut t = FactTable::parse(&sample_body()).unwrap();
        // two records, one timed: c1 follows the inferred rule; the old list is gone
        let list = vec![rec(1, 10.0, -1.0), rec(2, 11.0, 15.0)];
        assert_eq!(t.set_records("bugfix_132404", list.clone()).unwrap(), SetOutcome::Replaced { previous_records: 1 });
        let f = t.get("bugfix_132404").unwrap();
        assert_eq!((f.records.clone(), f.c1), (list, 1));
        // empty list is allowed (the game leaves such facts), a missing name is appended
        t.set_records("ach_attack", vec![]).unwrap();
        assert_eq!((t.get("ach_attack").unwrap().records.len(), t.get("ach_attack").unwrap().c1), (0, 0));
        assert_eq!(t.set_records("brand_new", vec![rec(7, 1.0, -1.0)]).unwrap(), SetOutcome::Created);
        assert_eq!(t.facts()[3].name, "brand_new");
        // bad input changes nothing
        assert!(t.set_records("x", vec![rec(1, f32::NAN, -1.0)]).is_err());
        assert!(t.set_records("x", vec![rec(1, 1.0, -1.0); 65_536]).is_err());
        assert!(t.set_records("", vec![]).is_err());
        assert!(t.get("x").is_none() && t.len() == 4);
        assert_eq!(FactTable::parse(&t.to_bytes()).unwrap(), t);
    }

    #[test]
    fn set_fact_copies_verbatim_including_c1() {
        let mut t = FactTable::parse(&sample_body()).unwrap();
        let odd = Fact { name: "ach_attack".into(), c1: 9, records: vec![rec(3, f32::from_bits(0x7fc0_0001), -1.0)] };
        t.set_fact(odd.clone()).unwrap();
        assert_eq!(t.facts()[1].c1, 9, "same position, c1 untouched by the recompute rule");
        assert_eq!(t.to_bytes()[..], FactTable::parse(&t.to_bytes()).unwrap().to_bytes()[..]);
        assert_eq!(t.set_fact(Fact { name: "zz".into(), c1: 0, records: vec![] }).unwrap(), SetOutcome::Created);
        assert!(t.set_fact(Fact { name: String::new(), c1: 0, records: vec![] }).is_err());
    }

    #[test]
    fn remove_keeps_order_and_the_index() {
        let mut t = FactTable::parse(&sample_body()).unwrap();
        assert!(t.remove("nope").is_none());
        assert_eq!(t.remove("bugfix_132404").unwrap().name, "bugfix_132404");
        assert_eq!(t.facts().iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["ach_attack", "83805989_2_1"]);
        assert!(t.get("bugfix_132404").is_none());
        assert_eq!(t.get("83805989_2_1").unwrap().records[0].expiry, -2.0);
        t.set_value("late", 1, 1.0).unwrap();
        assert_eq!(t.facts()[2].name, "late");
        assert_eq!(FactTable::parse(&t.to_bytes()).unwrap(), t);
    }

    #[test]
    fn differing_and_match_fact_restore_exactly() {
        let before = FactTable::parse(&sample_body()).unwrap();
        let mut after = before.clone();
        after.add_record("bugfix_132404", 1, 3000.0, FactRecord::PERMANENT).unwrap(); // changed
        after.remove("83805989_2_1"); // missing
        after.set_value("new_one", 1, 3000.0).unwrap(); // extra
        assert_eq!(after.differing(&before), ["83805989_2_1", "bugfix_132404", "new_one"]);
        assert_eq!(before.differing(&after), after.differing(&before));
        for n in after.differing(&before) {
            assert!(after.match_fact(&n, &before).unwrap());
        }
        assert!(after.differing(&before).is_empty());
        assert!(!after.match_fact("bugfix_132404", &before).unwrap(), "already equal");
        assert_eq!(after.get("83805989_2_1"), before.get("83805989_2_1"));
        assert!(after.get("new_one").is_none());
    }

    #[test]
    fn latest_time_is_the_newest_record() {
        let t = FactTable::parse(&sample_body()).unwrap();
        // the three real records carry 1130.2, 1200.x and 2400.x style times; just check the max
        let max = t.facts().iter().flat_map(|f| f.records.iter()).map(|r| r.time).fold(0.0, f32::max);
        assert_eq!(t.latest_time(), max);
        assert!(max > 1000.0);
        let mut e = FactTable::parse(&sample_body()).unwrap();
        for n in ["bugfix_132404", "ach_attack", "83805989_2_1"] {
            e.remove(n);
        }
        assert_eq!(e.latest_time(), 0.0);
    }

    #[test]
    fn negative_values_use_i16_bits_and_bad_input_is_refused() {
        let mut t = FactTable::parse(&sample_body()).unwrap();
        t.set_value("n", -1, 1.0).unwrap();
        assert_eq!(t.get("n").unwrap().records[0].value, 65535);
        assert_eq!(t.get("n").unwrap().sum(), -1);
        t.set_value("big", 65535, 1.0).unwrap();
        assert!(t.set_value("x", 65536, 1.0).is_err());
        assert!(t.set_value("x", -32769, 1.0).is_err());
        assert!(t.set_value("x", 1, f32::NAN).is_err());
        assert!(t.set_value("", 1, 1.0).is_err());
        assert!(t.set_value("\u{141}\u{f3}d\u{17a}", 1, 1.0).is_err(), "never widen past Latin-1");
        assert!(t.set_value(&"q".repeat(129), 1, 1.0).is_err());
        assert!(t.add_record("x", 1, 1.0, f32::INFINITY).is_err());
        // refused edits leave the table untouched
        assert_eq!(t.len(), 5);
        assert!(t.get("x").is_none());
    }

    /// Build a minimal payload map around a facts variable to test `find_facts_blob`.
    fn payload_with(var: &[u8], extra_ss: bool) -> (Vec<u8>, PayloadMap) {
        use crate::payload::VarEntry;
        let base = 3084u32;
        let mut p = b"SAV3".to_vec();
        p.extend_from_slice(&[0; 12]);
        let mut vars = Vec::new();
        // a decoy SS variable that is not the facts blob
        let decoy = b"SS\x02\0\0\0AB";
        vars.push(VarEntry { offset: base + p.len() as u32, size: decoy.len() as u32 });
        p.extend_from_slice(decoy);
        let n = if extra_ss { 2 } else { 1 };
        for _ in 0..n {
            vars.push(VarEntry { offset: base + p.len() as u32, size: var.len() as u32 });
            p.extend_from_slice(var);
        }
        let map = PayloadMap {
            base,
            header_ints: [67, 29, 164],
            variable_table_offset: 0,
            nm_offset: 0,
            rb_offset: 0,
            pre_table_bytes: *b"SC",
            variables: vars,
            notes: vec![],
        };
        (p, map)
    }

    #[test]
    fn finds_the_facts_variable() {
        let t = FactTable::parse(&sample_body()).unwrap();
        let var = t.to_ss_variable();
        let (p, map) = payload_with(&var, false);
        let r = find_facts_blob(&p, &map).unwrap();
        assert_eq!(&p[r.start - 6..r.start - 4], b"SS");
        assert_eq!(&p[r.clone()], &sample_body()[..]);
        assert_eq!(parse_payload(&p, &map).unwrap(), t);
    }

    #[test]
    fn missing_ambiguous_or_inconsistent_blob_is_an_error() {
        let var = FactTable::parse(&sample_body()).unwrap().to_ss_variable();
        let (p, map) = payload_with(&var, true);
        assert!(find_facts_blob(&p, &map).is_err(), "two candidates");
        let (p, mut map) = payload_with(&var, false);
        map.variables.truncate(1); // only the decoy left
        assert!(find_facts_blob(&p, &map).is_err(), "none");
        let mut bad = var.clone();
        bad[2] += 1; // length field no longer matches the table size
        let (p, map) = payload_with(&bad, false);
        assert!(find_facts_blob(&p, &map).is_err());
    }
}
