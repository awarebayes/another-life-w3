//! Content validation behind `w3al lint`, plus the soft level warning the UI shows.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::diagnostic::Diagnostic;
use crate::questbook::{Kind, Questbook};

/// A side quest this many levels above the nearest main quest gets a soft warning.
pub const LEVEL_GAP: u8 = 5;

pub fn lint(book: &Questbook) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    let mut ids: HashMap<&str, usize> = HashMap::new();
    let mut orders: HashMap<u32, &str> = HashMap::new();
    for q in &book.quests {
        *ids.entry(&q.id).or_default() += 1;
        if let Some(prev) = orders.insert(q.order, &q.id) {
            out.push(Diagnostic::error(
                Some(&q.id),
                format!("order {} is also used by `{prev}`; chronological order must be unambiguous", q.order),
            ));
        }
    }
    for (id, n) in &ids {
        if *n > 1 {
            out.push(Diagnostic::error(Some(id), format!("quest id used {n} times")));
        }
    }

    let order_of: HashMap<&str, u32> = book.quests.iter().map(|q| (q.id.as_str(), q.order)).collect();

    for q in &book.quests {
        let id = Some(q.id.as_str());

        if q.title.trim().is_empty() {
            out.push(Diagnostic::error(id, "empty title"));
        }
        if q.summary.trim().is_empty() {
            out.push(Diagnostic::error(id, "empty summary"));
        }

        for r in &q.requires {
            match order_of.get(r.as_str()) {
                None => out.push(Diagnostic::error(id, format!("requires unknown quest `{r}`"))),
                Some(_) if *r == q.id => out.push(Diagnostic::error(id, "requires itself")),
                Some(&o) if o >= q.order => out.push(Diagnostic::error(
                    id,
                    format!("requires `{r}` (order {o}) which does not come earlier than order {}", q.order),
                )),
                Some(_) => {}
            }
        }
        for x in &q.excludes {
            if !order_of.contains_key(x.as_str()) {
                out.push(Diagnostic::error(id, format!("excludes unknown quest `{x}`")));
            }
        }

        let mut used_effects: BTreeSet<&str> = BTreeSet::new();
        let mut reference = |key: &str, what: &str, out: &mut Vec<Diagnostic>| {
            if q.effects.contains_key(key) {
                used_effects.insert(q.effects.get_key_value(key).unwrap().0.as_str());
            } else {
                out.push(Diagnostic::error(id, format!("{what} refers to undefined effects `{key}`")));
            }
        };
        if let Some(k) = &q.started_effects {
            reference(k, "started_effects", &mut out);
        }
        if let Some(k) = &q.done_effects {
            reference(k, "done_effects", &mut out);
        }

        let mut choice_ids = HashSet::new();
        for c in &q.choices {
            if !choice_ids.insert(c.id.as_str()) {
                out.push(Diagnostic::error(id, format!("duplicate choice id `{}`", c.id)));
            }
            if c.options.len() < 2 {
                out.push(Diagnostic::error(id, format!("choice `{}` needs at least two options", c.id)));
            }
            if !c.options.iter().any(|o| o.id == c.default) {
                out.push(Diagnostic::error(
                    id,
                    format!("choice `{}`: default `{}` is not one of its options", c.id, c.default),
                ));
            }
            let mut opt_ids = HashSet::new();
            for o in &c.options {
                if !opt_ids.insert(o.id.as_str()) {
                    out.push(Diagnostic::error(id, format!("choice `{}`: duplicate option `{}`", c.id, o.id)));
                }
                if let Some(k) = &o.effects {
                    reference(k, &format!("choice `{}` option `{}`", c.id, o.id), &mut out);
                }
            }
        }

        for (name, e) in &q.effects {
            if e.source.trim().is_empty() {
                out.push(Diagnostic::error(id, format!("effects `{name}` has no `source`")));
            }
            if e.placeholder {
                out.push(Diagnostic::warn(id, format!("effects `{name}` is a PLACEHOLDER (not traced to a script or save diff)")));
            }
            if let Err(msg) = e.journal_edit() {
                out.push(Diagnostic::error(id, format!("effects `{name}`: {msg}")));
            }
            if let Err(msg) = e.graph_ops() {
                out.push(Diagnostic::error(id, format!("effects `{name}`: {msg}")));
            }
            if let Some(s) = &e.synth {
                if let Err(msg) = s.check() {
                    out.push(Diagnostic::error(id, format!("effects `{name}`: {msg}")));
                }
                if e.graph.is_some() {
                    out.push(Diagnostic::error(
                        id,
                        format!("effects `{name}`: `synth` and `graph` on the same effect (the walk writes the quest graph itself; use one)"),
                    ));
                }
                if e.placeholder {
                    out.push(Diagnostic::error(
                        id,
                        format!("effects `{name}`: a `synth` effect cannot be a placeholder (its blocks come from the game's graphs)"),
                    ));
                }
            }
            if let Some(Err(msg)) = e.late.as_ref().map(crate::synth_effect::LateEffect::check) {
                out.push(Diagnostic::error(id, format!("effects `{name}`: {msg}")));
            }
            if let Some(n) = e.facts_remove.iter().find(|n| e.facts.contains_key(n.as_str())) {
                out.push(Diagnostic::error(id, format!("effects `{name}`: fact `{n}` is both set and removed")));
            }
            if e.facts.is_empty()
                && e.facts_remove.is_empty()
                && e.quests.is_empty()
                && e.journal.is_empty()
                && !e.has_journal_edit()
                && !e.has_graph_edit()
                && !e.has_synth()
            {
                out.push(Diagnostic::warn(id, format!("effects `{name}` changes nothing")));
            }
            if !used_effects.contains(name.as_str()) {
                out.push(Diagnostic::warn(id, format!("effects `{name}` is never referenced")));
            }
        }
    }

    cycle_check(book, &mut out);
    // The generated side quests (`questbook/side/`, crate::side).
    out.extend(book.side.lint(book));
    out
}

/// `requires` must be a DAG. The order check above already rules out most cycles
/// (a cycle needs some edge going to an equal-or-later order), but a direct DFS keeps
/// the message precise if that rule is ever relaxed.
fn cycle_check(book: &Questbook, out: &mut Vec<Diagnostic>) {
    let edges: HashMap<&str, Vec<&str>> = book
        .quests
        .iter()
        .map(|q| (q.id.as_str(), q.requires.iter().map(String::as_str).collect()))
        .collect();

    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit<'a>(
        n: &'a str,
        edges: &HashMap<&'a str, Vec<&'a str>>,
        marks: &mut HashMap<&'a str, Mark>,
        out: &mut Vec<Diagnostic>,
    ) {
        match marks.get(n) {
            Some(Mark::Done) => return,
            Some(Mark::Visiting) => {
                out.push(Diagnostic::error(Some(n), "`requires` forms a cycle"));
                return;
            }
            None => {}
        }
        marks.insert(n, Mark::Visiting);
        for &m in edges.get(n).into_iter().flatten() {
            if edges.contains_key(m) {
                visit(m, edges, marks, out);
            }
        }
        marks.insert(n, Mark::Done);
    }

    let mut marks = HashMap::new();
    for q in &book.quests {
        visit(&q.id, &edges, &mut marks, out);
    }
}

/// Soft per-quest warnings for the UI: a side quest far above the main quest around it.
/// The app walks quests by `order`, which roughly follows level, so a big jump is
/// probably a content mistake or a quest the player should do later.
pub fn level_warnings(book: &Questbook) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (i, q) in book.quests.iter().enumerate() {
        if q.kind == Kind::Main {
            continue;
        }
        let nearest = book.quests[..i]
            .iter()
            .rev()
            .find(|m| m.kind == Kind::Main)
            .or_else(|| book.quests[i..].iter().find(|m| m.kind == Kind::Main));
        if let Some(m) = nearest {
            if q.level > m.level.saturating_add(LEVEL_GAP) {
                out.insert(
                    q.id.clone(),
                    format!(
                        "Recommended level {} is well above the main quest `{}` (level {}) around it.",
                        q.level, m.title, m.level
                    ),
                );
            }
        }
    }
    out
}
