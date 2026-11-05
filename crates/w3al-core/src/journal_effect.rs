//! Questbook journal effects: how a quest's quest-log changes are written in TOML and turned
//! into a `w3save::journal::JournalEdit`. Design and capture workflow:
//! docs/findings/journal-editor-design.md. Entries are identified by GUID paths that cannot be
//! derived from quest names, so every value here is captured from a pair of real saves with
//! `w3al journal-diff before.sav after.sav` and pasted as is (with the save names as `source`).
//!
//! Export applies them (after the same effect's `graph` table, before its facts): journal + graph +
//! facts together made a quest count as done in game (gates G, P, S, F, R; the journal alone did not,
//! gate J). `w3al journal-set --from-toml` applies them too, as a dev tool. See
//! docs/findings/export-wiring.md.
use serde::{Deserialize, Serialize};
use w3save::journal::{self, CounterEdit, Entry, Guid, JournalEdit, StatusChange, Step, STATUSES};

/// `[[effects.X.journal_status]]`: set the status of an entry the template already has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalStatusEffect {
    /// Readable path (file names / short GUIDs) for reviewers; not used to match.
    #[serde(default)]
    pub label: Option<String>,
    /// The entry's path GUIDs, outermost first (32 hex digits each, file byte order).
    pub path: Vec<String>,
    /// Status expected in the template. A template with another status (that is not already `to`) is refused.
    #[serde(default)]
    pub from: Option<String>,
    pub to: String,
    #[serde(default)]
    pub unread: Option<bool>,
}

/// One path step of an appended entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalStepEffect {
    pub guid: String,
    /// `.journal` resource of the step; empty when it is the parent's file.
    #[serde(default)]
    pub resource: String,
}

/// `[[effects.X.journal_append]]`: make sure an entry exists with this status (appended at the
/// end of the list when the template lacks it). `flags` are not written: they are 0 on every
/// step but the last and 1 on the last, on every entry of every fixture save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalAppendEffect {
    #[serde(default)]
    pub label: Option<String>,
    pub steps: Vec<JournalStepEffect>,
    pub status: String,
    pub unread: bool,
}

/// `[[effects.X.journal_counters]]`: set (or add) a `JObjectiveCounter`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalCounterEffect {
    pub guid: String,
    pub count: i32,
    /// Position of a NEW counter in the save it was captured from (the game keeps counters in
    /// hash order). Only meaningful when the template is that capture's "before" save; omit it
    /// to append at the end.
    #[serde(default)]
    pub at: Option<usize>,
}

/// `[effects.X.journal_track]`: the tracked quest and highlighted objective after the effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalTrackEffect {
    #[serde(default)]
    pub quest: Option<String>,
    #[serde(default)]
    pub objective: Option<String>,
}

fn guid(s: &str) -> Result<Guid, String> {
    journal::parse_guid(s).map_err(|e| e.to_string())
}

fn status(s: &str) -> Result<String, String> {
    if STATUSES.contains(&s) {
        Ok(s.to_owned())
    } else {
        Err(format!("unknown journal status {s:?} (expected one of {STATUSES:?})"))
    }
}

/// Build the edit for one effect's journal tables. Errors are readable lint messages.
pub fn to_edit(
    status_fx: &[JournalStatusEffect],
    append_fx: &[JournalAppendEffect],
    counters_fx: &[JournalCounterEffect],
    track: Option<&JournalTrackEffect>,
) -> Result<JournalEdit, String> {
    let mut e = JournalEdit::default();
    for s in status_fx {
        if s.path.is_empty() || s.path.len() > journal::MAX_PATH_STEPS {
            return Err(format!("journal_status: path must have 1..{} GUIDs", journal::MAX_PATH_STEPS));
        }
        e.status.push(StatusChange {
            key: s.path.iter().map(|g| guid(g)).collect::<Result<_, _>>()?,
            from: s.from.as_deref().map(status).transpose()?,
            to: status(&s.to)?,
            unread: s.unread,
        });
    }
    for a in append_fx {
        if a.steps.is_empty() || a.steps.len() > journal::MAX_PATH_STEPS {
            return Err(format!("journal_append: steps must have 1..{} entries", journal::MAX_PATH_STEPS));
        }
        let n = a.steps.len();
        let path = a
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| Ok(Step { guid: guid(&s.guid)?, resource: s.resource.clone(), flags: u8::from(i + 1 == n) }))
            .collect::<Result<_, String>>()?;
        e.append.push(Entry { path, status: status(&a.status)?, unread: a.unread });
    }
    for c in counters_fx {
        e.counters.push(CounterEdit { guid: guid(&c.guid)?, count: c.count, at: c.at });
    }
    if let Some(t) = track {
        e.tracked_quest = t.quest.as_deref().map(guid).transpose()?;
        e.highlighted_objective = t.objective.as_deref().map(guid).transpose()?;
    }
    Ok(e)
}

/// Merge several edits (several effects of a run) into one, in order.
pub fn merge(into: &mut JournalEdit, more: JournalEdit) {
    into.status.extend(more.status);
    into.append.extend(more.append);
    into.counters.extend(more.counters);
    if more.tracked_quest.is_some() {
        into.tracked_quest = more.tracked_quest;
    }
    if more.highlighted_objective.is_some() {
        into.highlighted_objective = more.highlighted_objective;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G1: &str = "67e5afe3ceb5c148a9cde5c4a99c7a84";
    const G2: &str = "0f3305c02a250d4aa75c6fba97048763";

    #[test]
    fn builds_an_edit_and_derives_flags() {
        let st = [JournalStatusEffect { label: None, path: vec![G1.into(), G2.into()], from: Some("JS_Active".into()), to: "JS_Success".into(), unread: None }];
        let ap = [JournalAppendEffect {
            label: None,
            steps: vec![
                JournalStepEffect { guid: G1.into(), resource: "a.journal".into() },
                JournalStepEffect { guid: G2.into(), resource: String::new() },
            ],
            status: "JS_Success".into(),
            unread: true,
        }];
        let e = to_edit(&st, &ap, &[], Some(&JournalTrackEffect { quest: Some(G2.into()), objective: None })).unwrap();
        assert_eq!(e.status[0].key.len(), 2);
        assert_eq!(e.append[0].path.iter().map(|s| s.flags).collect::<Vec<_>>(), [0, 1]);
        assert!(e.tracked_quest.is_some() && e.highlighted_objective.is_none());
    }

    #[test]
    fn rejects_bad_values() {
        let bad_status = [JournalStatusEffect { label: None, path: vec![G1.into()], from: None, to: "Done".into(), unread: None }];
        assert!(to_edit(&bad_status, &[], &[], None).is_err());
        let bad_guid = [JournalCounterEffect { guid: "0f33".into(), count: 0, at: None }];
        assert!(to_edit(&[], &[], &bad_guid, None).is_err());
        let empty = [JournalStatusEffect { label: None, path: vec![], from: None, to: "JS_Success".into(), unread: None }];
        assert!(to_edit(&empty, &[], &[], None).is_err());
    }
}
