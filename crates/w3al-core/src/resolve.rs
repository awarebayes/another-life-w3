//! Answers -> effects. Walks the questbook in chronological order.
//!
//! Status semantics from the plan:
//!   * completed: chosen option effects, then the quest's `done_effects`
//!   * started:   only `started_effects`
//!   * skipped:   nothing; skipping a main quest locks every later main quest
//!
//! A quest with no entry in the run counts as skipped.
use std::collections::HashSet;

use serde::Serialize;

use crate::diagnostic::Diagnostic;
use crate::questbook::{Effect, Kind, Questbook};
use crate::run::{Run, Status};

#[derive(Debug, Clone, Serialize)]
pub struct AppliedEffect {
    pub quest: String,
    /// "choice:option", "done" or "started" - where in the quest this came from.
    pub origin: String,
    pub effect: Effect,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Resolution {
    pub effects: Vec<AppliedEffect>,
    pub completed: Vec<String>,
    pub started: Vec<String>,
    /// Quests the run marked done/started that were dropped because a prerequisite
    /// was missing or an excluded quest was completed.
    pub blocked: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Resolution {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Every quest of the run, then the side quests it marks completed (`crate::side::resolve`, after the story's
/// own effects).
pub fn resolve(book: &Questbook, run: &Run) -> Resolution {
    let mut res = resolve_only(book, run, |_| true);
    let side = crate::side::resolve(&book.side, run);
    res.completed.extend(side.iter().map(|a| a.quest.clone()));
    res.effects.extend(side);
    res
}

/// [`resolve`] over the quests `keep` accepts; every other quest is left out entirely (it neither
/// skips nor locks anything, and a `requires` on it counts as met). The story chain uses this for the
/// quests of one decision window (docs/plan/story-chain.md): the window's earlier quests are already
/// in its template.
pub fn resolve_only(book: &Questbook, run: &Run, keep: impl Fn(&crate::questbook::Quest) -> bool) -> Resolution {
    let mut res = Resolution::default();
    let mut main_locked = false;
    let mut done: HashSet<&str> = HashSet::new();
    // Quests left out count as met for `requires` (they are in the template), never for `excludes`.
    let outside: HashSet<&str> = book.quests.iter().filter(|q| !keep(q)).map(|q| q.id.as_str()).collect();

    if let Some(stop) = &run.stop_after {
        if book.get(stop).is_none() {
            res.diagnostics
                .push(Diagnostic::error(None, format!("stop_after refers to unknown quest `{stop}`")));
        }
    }
    for id in run.quests.keys() {
        if book.get(id).is_none() {
            res.diagnostics
                .push(Diagnostic::warn(None, format!("run mentions unknown quest `{id}`; ignored")));
        }
    }

    for q in &book.quests {
        if !keep(q) {
            if run.stop_after.as_deref() == Some(q.id.as_str()) {
                break;
            }
            continue;
        }
        let status = run.quests.get(&q.id).map_or(Status::Skipped, |e| e.status);

        if status == Status::Skipped {
            if q.kind == Kind::Main {
                main_locked = true;
            }
        } else {
            let missing: Vec<&str> = q
                .requires
                .iter()
                .filter(|r| !done.contains(r.as_str()) && !outside.contains(r.as_str()))
                .map(String::as_str)
                .collect();
            let excluded: Vec<&str> = q
                .excludes
                .iter()
                .filter(|x| done.contains(x.as_str()))
                .map(String::as_str)
                .collect();
            let locked_main = q.kind == Kind::Main && main_locked;

            if locked_main || !missing.is_empty() || !excluded.is_empty() {
                let why = if locked_main {
                    "an earlier main quest was skipped".to_string()
                } else if !missing.is_empty() {
                    format!("requires {}", missing.join(", "))
                } else {
                    format!("excluded by {}", excluded.join(", "))
                };
                res.blocked.push(q.id.clone());
                res.diagnostics
                    .push(Diagnostic::warn(Some(&q.id), format!("not applied: {why}")));
                // A blocked main quest is effectively skipped for what follows.
                if q.kind == Kind::Main {
                    main_locked = true;
                }
            } else if status == Status::Started {
                res.started.push(q.id.clone());
                done.insert(&q.id);
                if let Some(key) = &q.started_effects {
                    push(&mut res, q, "started", key);
                }
            } else {
                let entry = &run.quests[&q.id];
                for c in &q.choices {
                    let picked = entry.answers.get(&c.id).unwrap_or(&c.default);
                    match c.options.iter().find(|o| &o.id == picked) {
                        None => res.diagnostics.push(Diagnostic::error(
                            Some(&q.id),
                            format!("choice `{}`: `{picked}` is not an option", c.id),
                        )),
                        Some(o) => {
                            if let Some(key) = &o.effects {
                                push(&mut res, q, &format!("{}:{}", c.id, o.id), key);
                            }
                        }
                    }
                }
                if let Some(key) = &q.done_effects {
                    push(&mut res, q, "done", key);
                }
                res.completed.push(q.id.clone());
                done.insert(&q.id);
            }
        }

        if run.stop_after.as_deref() == Some(q.id.as_str()) {
            break;
        }
    }
    res
}

fn push(res: &mut Resolution, q: &crate::questbook::Quest, origin: &str, key: &str) {
    match q.effects.get(key) {
        Some(e) => res.effects.push(AppliedEffect {
            quest: q.id.clone(),
            origin: origin.to_owned(),
            effect: e.clone(),
        }),
        None => res
            .diagnostics
            .push(Diagnostic::error(Some(&q.id), format!("undefined effects `{key}`"))),
    }
}
