//! Questbook graph effects: `[effects.<key>.graph]`, a quest's quest-graph (`questSystem`) and
//! spawn-set (`community`) delta, and how it becomes `w3save::graph` ops. Design and evidence:
//! docs/findings/quest-graph-design.md (gates G, P, S, F, R passed in game) and
//! docs/findings/export-wiring.md (how export applies them).
//!
//! The table is exactly what `w3al graph-diff --toml` prints and what `w3al journal-set
//! --graph-from-toml` reads; both the CLI and export parse it here, so there is ONE parser:
//!
//! ```toml
//! [effects.done.graph]
//! source = "save diff A -> B (w3al graph-diff)"   # optional; the effect's own `source` is the required one
//! donor  = "ManualSave_8559a_7ea4d800_4038706"     # stem of the save subtrees are copied from
//! ops = [
//!   { op = "remove", path = "questSystem/.../questBlock[62a65bce]", before = "#d5942b6b" },
//!   { op = "insert", path = "questSystem/.../questBlock[7fd9eeae]", after = "#3f4cebad" },
//!   { op = "replace", path = "...", before = "#91b604b6", after = "#07621930" },
//!   { op = "swap-remove", path = "community/stubs/a#8", before = "#dda3a33e" },
//!   { op = "recount", path = "community/stubs/numStubs", of = "a" },
//! ]
//! ```
//!
//! Guards: `before` = [`guard_hash`] of the template's node, `after` = of the donor's node; an empty
//! string (what `graph-diff` prints for an absent node) is not checked. The subtree bytes are not in
//! the TOML: applying a `replace` or `insert` needs the donor save itself.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use w3save::graph::{self, GraftOp, COUNT_RULES};
use w3save::Document;

/// Top-level blocks a questbook graph op may start with: the two whose transplant is proven in game
/// (gates G, P, S, F, R). Other blocks (`universe`, map pins, ...) are refused by lint and export.
pub const GRAPH_BLOCKS: [&str; 2] = ["questSystem", "community"];

/// The op kinds, as written in TOML.
pub const OP_KINDS: [&str; 5] = ["replace", "remove", "swap-remove", "insert", "recount"];

/// `[effects.X.graph]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphEffect {
    /// Where the delta came from (`graph-diff` writes the two save stems). Informational.
    #[serde(default)]
    pub source: Option<String>,
    /// File stem of the donor save (`<donor>.sav`, found in the export's donor folders). Required when
    /// an op copies from it (`replace`, `insert`); a delta of removals and recounts needs no donor file.
    #[serde(default)]
    pub donor: Option<String>,
    pub ops: Vec<GraphOpEffect>,
}

/// One op of a graph effect. Unused guard fields stay empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphOpEffect {
    pub op: String,
    pub path: String,
    #[serde(default)]
    pub before: String,
    #[serde(default)]
    pub after: String,
    #[serde(default)]
    pub of: String,
}

/// One parsed op with its guards (`None` = not checked).
#[derive(Debug, Clone, PartialEq)]
pub struct GuardedOp {
    pub op: GraftOp,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl GuardedOp {
    /// True for ops that copy a subtree from the donor.
    pub fn copies(&self) -> bool {
        matches!(self.op, GraftOp::Replace { .. } | GraftOp::Insert { .. })
    }
}

impl GraphEffect {
    /// True when an op copies from the donor (`replace`, `insert`), so the donor save is needed.
    pub fn needs_donor(&self) -> bool {
        self.ops.iter().any(|o| matches!(o.op.as_str(), "replace" | "insert"))
    }

    /// The ops, validated (see [`check_op`]) and with the donor rules applied. Errors are lint messages.
    pub fn to_ops(&self) -> Result<Vec<GuardedOp>, String> {
        if self.ops.is_empty() {
            return Err("graph: `ops` is empty".into());
        }
        if let Some(d) = &self.donor {
            check_donor_stem(d)?;
        } else if self.needs_donor() {
            return Err("graph: `replace`/`insert` ops copy from a donor save, but `donor` is missing".into());
        }
        self.ops.iter().enumerate().map(|(i, o)| op_from_spec(o).map_err(|m| format!("graph op {}: {m}", i + 1))).collect()
    }
}

/// A donor stem: what `w3al-core` will look for as `<stem>.sav`. Same alphabet as an output stem
/// (letters, digits, `_`, `-`; no dots, spaces or separators), at most 100 characters.
pub fn check_donor_stem(stem: &str) -> Result<(), String> {
    let ok = !stem.is_empty() && stem.len() <= 100 && stem.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(format!("graph: donor {stem:?} is not a save stem (letters, digits, `_`, `-`; no extension or folders)"))
    }
}

/// `#` + 8 lowercase hex digits, the shape [`graph::short_hash`] prints.
fn check_hash(field: &str, h: &str) -> Result<Option<String>, String> {
    if h.is_empty() {
        return Ok(None);
    }
    let ok = h.len() == 9 && h.starts_with('#') && h[1..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if ok {
        Ok(Some(h.to_owned()))
    } else {
        Err(format!("`{field}` {h:?} is not a guard hash (`#` + 8 lowercase hex digits, as `graph-diff --toml` prints)"))
    }
}

/// The path rules: non-empty steps separated by `/`, starting with a block of [`GRAPH_BLOCKS`].
pub fn check_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    if path.split('/').any(|s| s.trim().is_empty()) {
        return Err(format!("path {path:?} has an empty step"));
    }
    let first = path.split('/').next().unwrap_or_default();
    if !GRAPH_BLOCKS.contains(&first) {
        return Err(format!("path {path:?} does not start with one of {GRAPH_BLOCKS:?} (the blocks proven in game)"));
    }
    if !path.contains('/') {
        return Err(format!("path {path:?} names a whole top-level block; ops work on nodes inside it"));
    }
    Ok(())
}

/// The block name of a path step: `questThread[7b125918]` -> `questThread`, `a#3` -> `a`.
fn step_name(s: &str) -> &str {
    s.split(['[', '#', '{']).next().unwrap_or(s)
}

/// `recount` must name a rule of [`COUNT_RULES`]: the path ends in `<parent>/<count var>` and `of` is
/// that rule's counted child.
fn check_recount(path: &str, of: &str) -> Result<(), String> {
    let mut it = path.rsplit('/');
    let (var, parent) = (it.next().unwrap_or_default(), it.next().map(step_name).unwrap_or_default());
    match COUNT_RULES.iter().find(|r| r.1 == var && r.0 == parent) {
        Some(r) if r.2 == of => Ok(()),
        Some(r) => Err(format!("recount of {parent}/{var} counts `{}` children, not {of:?}", r.2)),
        None => Err(format!(
            "recount path must end in a count var of graph::COUNT_RULES ({}), not {parent}/{var}",
            COUNT_RULES.iter().map(|r| format!("{}/{}", r.0, r.1)).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Validate one op and turn it into a [`GuardedOp`]: known kind, path rules, guard shapes, guards
/// only where they mean something (`before` on the template's node: replace / remove / swap-remove;
/// `after` on the donor's node: replace / insert; `of` only on recount).
pub fn op_from_spec(o: &GraphOpEffect) -> Result<GuardedOp, String> {
    check_path(&o.path)?;
    let path = o.path.clone();
    let (op, before_ok, after_ok) = match o.op.as_str() {
        "replace" => (GraftOp::Replace { path }, true, true),
        "remove" => (GraftOp::Remove { path }, true, false),
        "swap-remove" => (GraftOp::SwapRemove { path }, true, false),
        "insert" => (GraftOp::Insert { path }, false, true),
        "recount" => {
            check_recount(&o.path, &o.of)?;
            (GraftOp::Recount { path, of: o.of.clone() }, false, false)
        }
        k => return Err(format!("unknown op {k:?} (expected one of {OP_KINDS:?})")),
    };
    if !o.of.is_empty() && o.op != "recount" {
        return Err(format!("`of` only belongs to recount, not {}", o.op));
    }
    if !o.before.is_empty() && !before_ok {
        return Err(format!("`before` has no meaning for {} (the template has no such node yet)", o.op));
    }
    if !o.after.is_empty() && !after_ok {
        return Err(format!("`after` has no meaning for {} (nothing is copied)", o.op));
    }
    Ok(GuardedOp { op, before: check_hash("before", &o.before)?, after: check_hash("after", &o.after)? })
}

/// `KIND:PATH` (`recount:PATH=of`), the CLI's `--graph-op`. No guards.
pub fn parse_op(s: &str) -> Result<GuardedOp, String> {
    let (kind, rest) = s.split_once(':').ok_or("--graph-op is KIND:PATH")?;
    let (path, of) = match kind {
        "recount" => rest.rsplit_once('=').map(|(p, f)| (p.to_owned(), f.to_owned())).ok_or("recount is recount:PATH=of")?,
        _ => (rest.to_owned(), String::new()),
    };
    op_from_spec(&GraphOpEffect { op: kind.to_owned(), path, of, ..Default::default() })
}

#[derive(Deserialize)]
struct GraphOnly {
    graph: Option<GraphEffect>,
}

#[derive(Deserialize)]
struct GraphFile {
    effects: BTreeMap<String, GraphOnly>,
}

/// Every `[effects.*.graph]` table of a TOML text (a `graph-diff --toml` snippet or a quest file;
/// other effect keys are ignored here), in key order: `(effect key, table)`.
pub fn tables_from_toml(text: &str) -> Result<Vec<(String, GraphEffect)>, String> {
    let f: GraphFile = toml::from_str(text).map_err(|e| e.to_string())?;
    Ok(f.effects.into_iter().filter_map(|(k, e)| e.graph.map(|g| (k, g))).collect())
}

/// Short hash of the subtree at `path` (name-resolved, comparable across saves), `None` when absent.
/// This is the guard value of graph TOML ops.
pub fn guard_hash(doc: &Document, path: &str) -> w3save::Result<Option<String>> {
    Ok(graph::locate(doc, path)?.map(|l| graph::short_hash(graph::fingerprint(doc, graph::node_at(doc, &l)).as_bytes())))
}

/// Check the `before` guards against `template` and the `after` guards against `donor` (`None`: the
/// ops copy nothing, so there are no `after` guards). A replace whose template node already equals
/// the donor's (`after`) passes too: an earlier effect from the same donor did it.
pub fn check_guards(ops: &[GuardedOp], template: &Document, donor: Option<&Document>) -> Result<(), String> {
    let hash = |d: &Document, p: &str| guard_hash(d, p).map_err(|e| format!("{p}: {e}"));
    for g in ops {
        let p = g.op.path();
        if let Some(want) = &g.after {
            let d = donor.ok_or_else(|| format!("{p}: an `after` guard needs the donor save"))?;
            let got = hash(d, p)?;
            if got.as_deref() != Some(want.as_str()) {
                return Err(format!("donor node {p} is not the one the delta was captured from (hash {got:?}, want {want})"));
            }
        }
        if let Some(want) = &g.before {
            let got = hash(template, p)?;
            let already = matches!(g.op, GraftOp::Replace { .. }) && g.after.is_some() && got == g.after;
            if got.as_deref() != Some(want.as_str()) && !already {
                return Err(format!("template node {p} is not the one the delta was captured from (hash {got:?}, want {want})"));
            }
        }
    }
    Ok(())
}

/// True when `a` and `b` address the same node or one lies under the other.
pub fn paths_overlap(a: &str, b: &str) -> bool {
    let under = |x: &str, y: &str| x.len() > y.len() && x.starts_with(y) && x.as_bytes()[y.len()] == b'/';
    a == b || under(a, b) || under(b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCOPE: &str = "questSystem/quest[witcher3_quest.w2quest]/questThread/questThread[7b125918]/questThread";

    fn op(kind: &str, path: &str) -> GraphOpEffect {
        GraphOpEffect { op: kind.into(), path: path.into(), ..Default::default() }
    }

    #[test]
    fn parses_what_graph_diff_prints() {
        let text = format!(
            r##"[effects.done.graph]
source = "save diff A -> B (w3al graph-diff)"
donor = "ManualSave_8559a_7ea4d800_4038706"
ops = [
  {{ op = "remove", path = "{SCOPE}/questBlock[62a65bce]", before = "#d5942b6b" }},
  {{ op = "insert", path = "{SCOPE}/questBlock[7fd9eeae]", after = "#3f4cebad" }},
  {{ op = "replace", path = "questSystem/questExternalScenePlayers/CQuestExternalScenePlayer/ExternalDialog[mq0005_arsonist]", before = "#91b604b6", after = "" }},
  {{ op = "recount", path = "{SCOPE}/numBlocks", of = "questBlock" }},
  {{ op = "swap-remove", path = "community/stubs/a#8", before = "#dda3a33e" }},
  {{ op = "recount", path = "community/stubs/numStubs", of = "a" }},
]

[effects.other]
source = "x"
facts = {{ a = 1 }}
"##
        );
        let tables = tables_from_toml(&text).unwrap();
        assert_eq!(tables.len(), 1);
        let (key, g) = &tables[0];
        assert_eq!(key, "done");
        let ops = g.to_ops().unwrap();
        assert_eq!(ops.len(), 6);
        assert!(g.needs_donor());
        assert_eq!(ops[0].before.as_deref(), Some("#d5942b6b"));
        assert_eq!(ops[2].after, None, "an empty guard is not checked");
        assert_eq!(ops[3].op, GraftOp::Recount { path: format!("{SCOPE}/numBlocks"), of: "questBlock".into() });
    }

    #[test]
    fn rejects_bad_ops() {
        let bad = [
            (op("move", "questSystem/x"), "unknown op"),
            (op("remove", ""), "empty path"),
            (op("remove", "questSystem//x"), "empty step"),
            (op("remove", "universe/x"), "does not start"),
            (op("remove", "questSystem"), "whole top-level"),
            (GraphOpEffect { before: "d5942b6b".into(), ..op("remove", "questSystem/x") }, "guard hash"),
            (GraphOpEffect { before: "#D5942B6B".into(), ..op("remove", "questSystem/x") }, "guard hash"),
            (GraphOpEffect { after: "#d5942b6b".into(), ..op("remove", "questSystem/x") }, "no meaning"),
            (GraphOpEffect { before: "#d5942b6b".into(), ..op("insert", "questSystem/x") }, "no meaning"),
            (GraphOpEffect { of: "a".into(), ..op("remove", "questSystem/x") }, "only belongs"),
            (GraphOpEffect { of: "questBlock".into(), ..op("recount", "community/stubs/numStubs") }, "counts `a`"),
            (GraphOpEffect { of: "a".into(), ..op("recount", "community/stubs/size") }, "COUNT_RULES"),
            (GraphOpEffect { of: "questBlock".into(), ..op("recount", "community/numBlocks") }, "COUNT_RULES"),
        ];
        for (o, want) in bad {
            let err = op_from_spec(&o).unwrap_err();
            assert!(err.contains(want), "{o:?}: {err}");
        }
        // recount inside a keyed scope: the parent step's name is what counts
        assert!(op_from_spec(&GraphOpEffect { of: "questThread".into(), ..op("recount", &format!("{SCOPE}/questThread[38424c39]/questThread/numThreads")) }).is_ok());
    }

    #[test]
    fn donor_rules() {
        let copy = GraphEffect { ops: vec![op("insert", "questSystem/x")], ..Default::default() };
        assert!(copy.to_ops().unwrap_err().contains("donor` is missing"));
        let removal = GraphEffect { ops: vec![op("remove", "community/communities/c[x.w2comm]")], ..Default::default() };
        assert!(removal.to_ops().is_ok(), "removals need no donor file");
        for bad in ["", "a.sav", "dir/a", "a b", "..", "x\\y"] {
            let g = GraphEffect { donor: Some(bad.into()), ..copy.clone() };
            assert!(g.to_ops().is_err(), "{bad:?}");
        }
        assert!(GraphEffect { donor: Some("ManualSave_8559a_7ea51000_3ad835".into()), ..copy }.to_ops().is_ok());
        assert!(GraphEffect::default().to_ops().unwrap_err().contains("empty"));
    }

    #[test]
    fn cli_op_syntax() {
        assert_eq!(parse_op("remove:community/stubs/a#8").unwrap().op, GraftOp::Remove { path: "community/stubs/a#8".into() });
        assert_eq!(
            parse_op("recount:community/stubs/numStubs=a").unwrap().op,
            GraftOp::Recount { path: "community/stubs/numStubs".into(), of: "a".into() }
        );
        assert!(parse_op("remove").is_err());
        assert!(parse_op("recount:community/stubs/numStubs").is_err());
        // A stub addressed by idTag (graph::SELECTORS).
        let p = "community/stubs/a{idTag=0121659951849b055c00000000db070000}";
        assert_eq!(parse_op(&format!("swap-remove:{p}")).unwrap().op, GraftOp::SwapRemove { path: p.into() });
        assert_eq!(step_name("a{idTag=01}"), "a");
    }

    #[test]
    fn overlap() {
        assert!(paths_overlap("questSystem/a", "questSystem/a"));
        assert!(paths_overlap("questSystem/a", "questSystem/a/b"));
        assert!(paths_overlap("questSystem/a/b", "questSystem/a"));
        assert!(!paths_overlap("questSystem/a", "questSystem/ab"));
        assert!(!paths_overlap("community/stubs/a#3", "community/stubs/a#30"));
        assert!(!paths_overlap("questSystem/a/b", "questSystem/a/c"));
    }
}
