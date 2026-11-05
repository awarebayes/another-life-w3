//! One-call summary of a save, shared by `w3al dump`, `check-save` and the app's
//! `detect_game` command.
use std::collections::BTreeMap;

use serde::Serialize;

use crate::container::{ChunkRecord, Container};
use crate::diag::Issue;
use crate::error::Result;
use crate::facts;
use crate::document::Document;
use crate::payload::{self, PayloadMap};
use crate::token::TokenStats;

#[derive(Debug, Serialize)]
pub struct SaveInfo {
    pub file_size: usize,
    pub chunk_count: usize,
    pub data_start: u32,
    pub payload_size: usize,
    pub chunks: Vec<ChunkRecord>,
    pub payload: Option<PayloadMap>,
    /// Why the payload could not be mapped, if it could not.
    pub payload_error: Option<String>,
    pub variable_tags: BTreeMap<String, usize>,
    pub name_count: Option<usize>,
    pub name_error: Option<String>,
    pub rb_count: Option<usize>,
    /// Facts in the facts blob (`SS` + `SXAP` + `SBDF`), when it parses (5.x layout).
    pub fact_count: Option<usize>,
    pub fact_error: Option<String>,
    /// Typed-token coverage (PHASE0.md step 7); `None` when the document could not be parsed.
    pub tokens: Option<TokenStats>,
    pub tokens_error: Option<String>,
    /// typed parse -> serialize reproduced the decoded payload byte for byte.
    pub tokens_roundtrip: Option<bool>,
    pub issues: Vec<Issue>,
    /// True when the middle header int is 29. Derived from ONE 5.x save (2026-10-18:
    /// header ints [67, 29, 164]); older saves had [64, 18, 163] and [54, 10, 162] on
    /// 1.x. The plan's "66 / 29 / 0" did not match, so only the 29 is trusted, and
    /// only as an informational hint until more 5.x saves confirm it.
    pub looks_like_5x: bool,
}

pub fn inspect(file: &[u8]) -> Result<SaveInfo> {
    let c = Container::read(file)?;
    let mut issues = c.diagnostics();

    let (payload, payload_error) = match payload::read_map(&c.payload, c.data_start) {
        Ok(m) => (Some(m), None),
        Err(e) => (None, Some(e.to_string())),
    };

    let mut variable_tags = BTreeMap::new();
    let (mut name_count, mut name_error, mut rb_count) = (None, None, None);
    let mut looks_like_5x = false;
    let (mut fact_count, mut fact_error) = (None, None);
    let (mut tokens, mut tokens_error, mut tokens_roundtrip) = (None, None, None);

    if let Some(m) = &payload {
        looks_like_5x = m.header_ints[1] == 29;
        for n in &m.notes {
            issues.push(Issue::warn(n.clone()));
        }
        for v in &m.variables {
            if let Ok(start) = payload::rel(v.offset, m.base, c.payload.len()) {
                let end = (start + v.size as usize).min(c.payload.len());
                *variable_tags
                    .entry(payload::tag_of(&c.payload[start..end]))
                    .or_insert(0) += 1;
            }
        }
        match payload::read_names(&c.payload, m) {
            Ok(names) => {
                name_count = Some(names.len());
                // The name table holds variable/type names and CName strings, not only fact ids, but the 5.0.0c
                // damage shows up here too, so it is a cheap early signal.
                issues.extend(facts::check_ids(names.iter().map(String::as_str)));
            }
            Err(e) => name_error = Some(e.to_string()),
        }
        rb_count = payload::read_rb(&c.payload, m).ok().map(|r| r.len());
        // The 5.0.0c damage check on the real fact names (the NM check above is only an early signal).
        match facts::parse_payload(&c.payload, m) {
            Ok(t) => {
                fact_count = Some(t.len());
                issues.extend(t.check_ids());
            }
            Err(e) => fact_error = Some(e.to_string()),
        }
        match Document::parse(&c.payload, c.data_start) {
            Ok(doc) => {
                tokens_roundtrip = Some(doc.to_bytes() == c.payload);
                tokens = Some(doc.token_stats());
            }
            Err(e) => tokens_error = Some(e.to_string()),
        }
    }

    Ok(SaveInfo {
        file_size: file.len(),
        chunk_count: c.records.len(),
        data_start: c.data_start,
        payload_size: c.payload.len(),
        chunks: c.records,
        payload,
        payload_error,
        variable_tags,
        name_count,
        name_error,
        rb_count,
        fact_count,
        fact_error,
        tokens,
        tokens_error,
        tokens_roundtrip,
        issues,
        looks_like_5x,
    })
}
