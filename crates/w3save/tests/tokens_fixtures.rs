//! Fixture-gated: typed parse -> serialize must reproduce the decoded payload byte for byte on
//! every save under `W3AL_FIXTURES` (PHASE0.md step 7), and the variable table recomputed
//! from the tree must equal the one on disk. Skipped (passes) when the variable is unset.
//! Fixtures are copies of real saves and are never committed.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use w3save::{Container, Document, TokenStats};

fn saves(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            saves(&p, out);
        } else if p.extension().is_some_and(|x| x == "sav") {
            out.push(p);
        }
    }
}

#[test]
fn typed_tree_reproduces_every_fixture_payload() {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    let mut failures = Vec::new();
    let mut total = TokenStats::default();
    let mut with_facts = 0usize;
    let mut checked = 0usize;
    for f in &files {
        let outcome = (|| -> Result<(TokenStats, bool), String> {
            let bytes = std::fs::read(f).map_err(|e| e.to_string())?;
            let c = Container::read(&bytes).map_err(|e| e.to_string())?;
            let doc = Document::parse(&c.payload, c.data_start).map_err(|e| e.to_string())?;
            if doc.to_bytes() != c.payload {
                return Err("typed parse -> serialize differs from the decoded payload".into());
            }
            if doc.table() != doc.map.variables {
                return Err("table recomputed from the tree differs from the on-disk table".into());
            }
            // The game numbers its table in post-order (children before parent); journal appends rely on it.
            if !doc.indices_are_post_order() {
                return Err("table indices are not the post-order ranks of the nodes".into());
            }
            Ok((doc.token_stats(), doc.facts_node().is_some()))
        })();
        match outcome {
            Ok((st, facts)) => {
                checked += 1;
                with_facts += facts as usize;
                total.total += st.total;
                total.typed += st.typed;
                total.raw += st.raw;
                for (t, s) in st.by_tag {
                    let e = total.by_tag.entry(t).or_default();
                    e.total += s.total;
                    e.typed += s.typed;
                    e.raw += s.raw;
                    e.value_decoded += s.value_decoded;
                }
                for (k, v) in st.raw_reasons {
                    *total.raw_reasons.entry(k).or_default() += v;
                }
            }
            Err(e) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!(
        "{checked} of {} saves round-trip through typed nodes; {with_facts} have a recognised facts SS; \
         {} nodes, {:.2}% typed",
        files.len(),
        total.total,
        total.typed_percent()
    );
    let tags: BTreeMap<_, _> = total.by_tag.iter().collect();
    for (t, s) in tags {
        eprintln!("  {t:<5} total {:>9} typed {:>9} raw {:>8} value-decoded {:>9}", s.total, s.typed, s.raw, s.value_decoded);
    }
    for (k, v) in &total.raw_reasons {
        eprintln!("  raw: {k} x {v}");
    }
    assert!(failures.is_empty(), "{} of {} saves failed:\n{}", failures.len(), files.len(), failures.join("\n"));
}
