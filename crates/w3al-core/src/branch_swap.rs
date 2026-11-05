//! Output swap: turn a decision delta captured for one output of a decision scene ("Keira to Kaer
//! Morhen") into the delta of a sibling output ("Keira to Radovid") without a donor that played it.
//! Method and evidence: docs/findings/branch-synthesis.md.
//!
//! The swap file names, for each thing the two outputs do differently, what output A wrote and what
//! output B writes instead. Every value is read from the quest graph (block GUIDs of the waiting
//! blocks, `CJournalPath` GUID chains of the journal blocks, `factID`s of the fact blocks on each
//! output's path), never guessed:
//!
//! ```toml
//! [swap]
//! source = "q109_wrapup d24f649d: `Keira to KM` -> `Keira to Radovid`"
//! [[swap.block]]      # character_journal_entries 7b32f7d6: keira_to_km -> keira_to_radovid
//! from = "f5290e77e94cfd4ca216b846c93d81ac"
//! to   = "c133524a87776c429ddf03180120419a"
//! [[swap.journal]]    # 9548cabd -> d16ab830 questEntry
//! from = ["419f978f...", "ddd5713d...", "5a772866...", "7801de37..."]
//! to   = ["419f978f...", "ddd5713d...", "5a772866...", "fdcc6168..."]
//! [[swap.fact]]       # b23ada11 -> 005478a5
//! from = "q109_keira_to_km"
//! to   = "q109_keira_to_radovid"
//! ```
//!
//! Applying it rewrites the delta (graph op paths, appended journal entries and their counters /
//! tracking GUIDs, fact names) and lists the donor nodes to [`w3save::graph::rekey`], so the copied
//! subtrees come from a synthetic donor that holds output B's blocks. Every swap item must hit
//! something in the delta: an item that matches nothing means the delta is not output A's.
//!
//! HYPOTHESIS (see [`w3save::graph::rekey`]): output B's waiting block saves the same bytes as
//! output A's apart from its GUID when both are the same block class, entered through the same input
//! socket, with the same number of conditions. The swap file's author checks that in the graph.
//! Only rejoining outputs (both paths meet again after instant blocks, before any player wait)
//! can be swapped this way; a diverging output needs its own donor.
use serde::Deserialize;
use w3save::graph::{self, GraftOp};
use w3save::journal::{self, Guid, JournalEdit};
use w3save::Document;

use crate::graph_effect::GuardedOp;

/// `[swap]` of a swap file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSwap {
    /// Where the substitutions were read (graph file, scene block, sockets). Informational.
    #[serde(default)]
    pub source: Option<String>,
    /// Waiting blocks (quest-graph GUIDs, 32 hex digits).
    #[serde(default)]
    pub block: Vec<GuidSwap>,
    /// Journal entries (full GUID paths, outermost first).
    #[serde(default)]
    pub journal: Vec<PathSwap>,
    /// Fact names.
    #[serde(default)]
    pub fact: Vec<FactSwap>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidSwap {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathSwap {
    pub from: Vec<String>,
    pub to: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactSwap {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SwapFile {
    swap: OutputSwap,
}

/// The delta after the swap.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Swapped {
    /// The ops with rewritten paths. Guards of a rewritten op are cleared (they hashed output A's
    /// nodes); check the original ops' guards before swapping.
    pub ops: Vec<GuardedOp>,
    /// Donor nodes to rekey before copying: `(path in the original donor, new GUID)`, deepest first.
    pub rekeys: Vec<(String, Guid)>,
    pub journal: JournalEdit,
    pub facts: Vec<(String, i64)>,
    /// One line per rewritten item, for reports.
    pub log: Vec<String>,
}

fn guid(s: &str) -> Result<Guid, String> {
    journal::parse_guid(s).map_err(|e| e.to_string())
}

fn short(g: &Guid) -> String {
    journal::guid_hex(g)[..8].to_owned()
}

/// Parse and check a swap file.
pub fn from_toml(text: &str) -> Result<OutputSwap, String> {
    let f: SwapFile = toml::from_str(text).map_err(|e| e.to_string())?;
    f.swap.check()?;
    Ok(f.swap)
}

impl OutputSwap {
    /// Shape checks: GUIDs parse, `from` differs from `to`, a block's short key (first 8 hex digits,
    /// what graph paths use) differs too, journal paths have equal length, no `from` twice.
    pub fn check(&self) -> Result<(), String> {
        if self.block.is_empty() && self.journal.is_empty() && self.fact.is_empty() {
            return Err("swap: no [[swap.block]], [[swap.journal]] or [[swap.fact]] items".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for b in &self.block {
            let (f, t) = (guid(&b.from)?, guid(&b.to)?);
            if short(&f) == short(&t) {
                return Err(format!("swap.block {}: `from` and `to` share their short key", b.from));
            }
            if !seen.insert(short(&f)) {
                return Err(format!("swap.block {}: listed twice", b.from));
            }
        }
        for j in &self.journal {
            if j.from.is_empty() || j.from.len() > journal::MAX_PATH_STEPS || j.from.len() != j.to.len() {
                return Err(format!("swap.journal {:?}: `from` and `to` need the same number (1..{}) of GUIDs", j.from, journal::MAX_PATH_STEPS));
            }
            for g in j.from.iter().chain(&j.to) {
                guid(g)?;
            }
            if j.from == j.to || !seen.insert(j.from.join("/")) {
                return Err(format!("swap.journal {:?}: `from` equals `to` or is listed twice", j.from));
            }
        }
        for x in &self.fact {
            if x.from.is_empty() || x.to.is_empty() || x.from == x.to || !seen.insert(format!("fact:{}", x.from)) {
                return Err(format!("swap.fact {:?} -> {:?}: empty, equal, or listed twice", x.from, x.to));
            }
        }
        Ok(())
    }

    /// Rewrite a delta from output A to output B. Errors when a swap item matches nothing in the
    /// delta, or when an appended entry would change a step that names a `.journal` resource (the
    /// two outputs' entries must live in the same file).
    pub fn apply(&self, ops: &[GuardedOp], edit: &JournalEdit, facts: &[(String, i64)]) -> Result<Swapped, String> {
        self.check()?;
        let mut out = Swapped { journal: edit.clone(), facts: facts.to_vec(), ..Default::default() };
        let mut hits = vec![0usize; self.block.len() + self.journal.len() + self.fact.len()];

        // Graph: rewrite every `[from8]` step; copied nodes at or under a rekeyed step need the donor rekeyed.
        let blocks: Vec<(String, Guid)> = self.block.iter().map(|b| Ok((short(&guid(&b.from)?), guid(&b.to)?))).collect::<Result<_, String>>()?;
        for g in ops {
            let old = g.op.path();
            let mut steps: Vec<String> = Vec::new();
            let mut changed = false;
            for s in old.split('/') {
                let hit = blocks.iter().position(|(f, _)| s.strip_suffix(']').and_then(|x| x.split_once('[')).is_some_and(|(_, k)| k == f));
                let Some(i) = hit else {
                    steps.push(s.to_owned());
                    continue;
                };
                hits[i] += 1;
                changed = true;
                let name = s.split('[').next().unwrap_or(s);
                if g.copies() {
                    // Path of this node in the ORIGINAL donor (earlier steps are not rewritten there yet).
                    let orig: Vec<&str> = old.split('/').take(steps.len() + 1).collect();
                    let entry = (orig.join("/"), blocks[i].1);
                    if !out.rekeys.contains(&entry) {
                        out.rekeys.push(entry);
                    }
                }
                steps.push(format!("{name}[{}]", short(&blocks[i].1)));
            }
            if !changed {
                out.ops.push(g.clone());
                continue;
            }
            let new = steps.join("/");
            out.log.push(format!("graph op path {old} -> {new}"));
            let op = match &g.op {
                GraftOp::Replace { .. } => GraftOp::Replace { path: new },
                GraftOp::Remove { .. } => GraftOp::Remove { path: new },
                GraftOp::SwapRemove { .. } => GraftOp::SwapRemove { path: new },
                GraftOp::Insert { .. } => GraftOp::Insert { path: new },
                GraftOp::Recount { of, .. } => GraftOp::Recount { path: new, of: of.clone() },
            };
            out.ops.push(GuardedOp { op, before: None, after: None });
        }
        // Deepest first, so an outer rekey does not move an inner node's original path.
        out.rekeys.sort_by_key(|(p, _)| std::cmp::Reverse(p.matches('/').count()));

        // Journal: whole entries by exact path; counters and tracking by the entry's last GUID.
        let n_blocks = self.block.len();
        for (k, j) in self.journal.iter().enumerate() {
            let from: Vec<Guid> = j.from.iter().map(|g| guid(g)).collect::<Result<_, _>>()?;
            let to: Vec<Guid> = j.to.iter().map(|g| guid(g)).collect::<Result<_, _>>()?;
            let (fl, tl) = (*from.last().expect("checked non-empty"), *to.last().expect("checked non-empty"));
            let hit = &mut hits[n_blocks + k];
            for e in &mut out.journal.append {
                if e.key() != from {
                    continue;
                }
                for (i, s) in e.path.iter_mut().enumerate() {
                    if s.guid != to[i] {
                        if !s.resource.is_empty() {
                            return Err(format!("swap.journal {}: step {i} names {:?}; a swap across journal files is not supported", journal::key_hex(&from), s.resource));
                        }
                        s.guid = to[i];
                    }
                }
                *hit += 1;
                out.log.push(format!("journal append {} -> {}", journal::key_hex(&from), journal::key_hex(&to)));
            }
            for s in &mut out.journal.status {
                if s.key == from {
                    s.key.clone_from(&to);
                    *hit += 1;
                    out.log.push(format!("journal status {} -> {}", journal::key_hex(&from), journal::key_hex(&to)));
                }
            }
            for c in &mut out.journal.counters {
                if c.guid == fl {
                    c.guid = tl;
                    *hit += 1;
                    out.log.push(format!("journal counter {} -> {}", short(&fl), short(&tl)));
                }
            }
            for t in [&mut out.journal.tracked_quest, &mut out.journal.highlighted_objective] {
                if *t == Some(fl) {
                    *t = Some(tl);
                    *hit += 1;
                    out.log.push(format!("journal tracking {} -> {}", short(&fl), short(&tl)));
                }
            }
        }

        // Facts by name.
        let n_before = n_blocks + self.journal.len();
        for (k, x) in self.fact.iter().enumerate() {
            for f in &mut out.facts {
                if f.0 == x.from {
                    f.0.clone_from(&x.to);
                    hits[n_before + k] += 1;
                    out.log.push(format!("fact {} -> {} (= {})", x.from, x.to, f.1));
                }
            }
        }

        let unused: Vec<String> = self
            .block
            .iter()
            .map(|b| format!("block {}", b.from))
            .chain(self.journal.iter().map(|j| format!("journal {}", j.from.join("/"))))
            .chain(self.fact.iter().map(|x| format!("fact {}", x.from)))
            .zip(&hits)
            .filter(|(_, h)| **h == 0)
            .map(|(s, _)| s)
            .collect();
        if !unused.is_empty() {
            return Err(format!("swap items match nothing in the delta (is it output A's delta?): {}", unused.join(", ")));
        }
        Ok(out)
    }
}

/// A copy of `donor` with the [`Swapped::rekeys`] applied: the donor the swapped ops copy from.
pub fn synth_donor(donor: &Document, rekeys: &[(String, Guid)]) -> w3save::Result<Document> {
    let mut d = donor.clone();
    for (path, g) in rekeys {
        graph::rekey(&mut d, path, g)?;
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use w3save::journal::{CounterEdit, Entry, StatusChange, Step};

    const T: &str = "questSystem/q/questThread[143c1d62]/questThread/questThread[a536c75d]/questThread";
    const A: &str = "f5290e77e94cfd4ca216b846c93d81ac";
    const B: &str = "c133524a87776c429ddf03180120419a";
    const P: &str = "419f978ff624ed49a444903714632529";
    const JA: &str = "7801de377aab6a4381fee5b713690733";
    const JB: &str = "fdcc61685ad3364fb611b82ed5a6f23d";

    fn swap() -> OutputSwap {
        from_toml(&format!(
            "[swap]\nsource = \"test\"\n[[swap.block]]\nfrom = \"{A}\"\nto = \"{B}\"\n[[swap.journal]]\nfrom = [\"{P}\", \"{JA}\"]\nto = [\"{P}\", \"{JB}\"]\n[[swap.fact]]\nfrom = \"x_to_km\"\nto = \"x_to_radovid\"\n"
        ))
        .unwrap()
    }

    fn g(op: GraftOp) -> GuardedOp {
        GuardedOp { op, before: Some("#00000000".into()), after: Some("#11111111".into()) }
    }

    fn entry(last: &str) -> Entry {
        Entry {
            path: vec![
                Step { guid: guid(P).unwrap(), resource: "a.journal".into(), flags: 0 },
                Step { guid: guid(last).unwrap(), resource: String::new(), flags: 1 },
            ],
            status: "JS_Active".into(),
            unread: true,
        }
    }

    #[test]
    fn swaps_graph_paths_journal_and_facts() {
        let ops = vec![
            g(GraftOp::Remove { path: format!("{T}/questBlock[b3a72321]") }),
            g(GraftOp::Insert { path: format!("{T}/questBlock[f5290e77]") }),
            g(GraftOp::Insert { path: format!("{T}/questThread[f5290e77]/questThread/questBlock[0000aaaa]") }),
            g(GraftOp::Recount { path: format!("{T}/numBlocks"), of: "questBlock".into() }),
        ];
        let edit = JournalEdit {
            append: vec![entry("b47bfb415fc1e746ba2e4b78da1d792a"), entry(JA)],
            status: vec![StatusChange { key: entry(JA).key(), from: None, to: "JS_Success".into(), unread: None }],
            counters: vec![CounterEdit { guid: guid(JA).unwrap(), count: 0, at: None }],
            highlighted_objective: Some(guid(JA).unwrap()),
            ..Default::default()
        };
        let facts = vec![("x_confronted".to_owned(), 1), ("x_to_km".to_owned(), 1)];
        let s = swap().apply(&ops, &edit, &facts).unwrap();

        // Untouched ops keep their guards; rewritten ones lose them.
        assert_eq!(s.ops[0], ops[0]);
        assert_eq!(s.ops[3], ops[3]);
        assert_eq!(s.ops[1], GuardedOp { op: GraftOp::Insert { path: format!("{T}/questBlock[c133524a]") }, before: None, after: None });
        assert_eq!(s.ops[2].op.path(), format!("{T}/questThread[c133524a]/questThread/questBlock[0000aaaa]"));
        // Both donor nodes get rekeyed (the deeper path first; same depth keeps op order).
        assert_eq!(
            s.rekeys,
            vec![(format!("{T}/questBlock[f5290e77]"), guid(B).unwrap()), (format!("{T}/questThread[f5290e77]"), guid(B).unwrap())]
        );
        // Journal: the A entry becomes the B entry in place (order kept); the shared one stays.
        assert_eq!(s.journal.append[0], entry("b47bfb415fc1e746ba2e4b78da1d792a"));
        assert_eq!(s.journal.append[1], entry(JB));
        assert_eq!(s.journal.status[0].key, entry(JB).key());
        assert_eq!(s.journal.counters[0].guid, guid(JB).unwrap());
        assert_eq!(s.journal.highlighted_objective, Some(guid(JB).unwrap()));
        assert_eq!(s.facts, vec![("x_confronted".to_owned(), 1), ("x_to_radovid".to_owned(), 1)]);
    }

    #[test]
    fn an_unused_item_is_refused() {
        let ops = vec![g(GraftOp::Insert { path: format!("{T}/questBlock[f5290e77]") })];
        let err = swap().apply(&ops, &JournalEdit::default(), &[("x_to_km".into(), 1)]).unwrap_err();
        assert!(err.contains("journal") && !err.contains("block f5290e77"), "{err}");
    }

    #[test]
    fn a_resource_step_is_never_rewritten() {
        let mut s = swap();
        s.block.clear();
        s.fact.clear();
        s.journal[0].to[0] = JB.into();
        let edit = JournalEdit { append: vec![entry(JA)], ..Default::default() };
        assert!(s.apply(&[], &edit, &[]).unwrap_err().contains("across journal files"));
    }

    #[test]
    fn bad_swap_files_are_refused() {
        assert!(from_toml("[swap]\n").is_err());
        assert!(from_toml(&format!("[swap]\n[[swap.block]]\nfrom = \"{A}\"\nto = \"{A}\"\n")).is_err());
        assert!(from_toml(&format!("[swap]\n[[swap.journal]]\nfrom = [\"{P}\"]\nto = [\"{P}\", \"{JB}\"]\n")).is_err());
        assert!(from_toml("[swap]\n[[swap.fact]]\nfrom = \"a\"\nto = \"a\"\n").is_err());
        assert!(from_toml("[swap]\nextra = 1\n[[swap.fact]]\nfrom = \"a\"\nto = \"b\"\n").is_err());
    }
}
