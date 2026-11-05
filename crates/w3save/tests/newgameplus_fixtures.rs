//! Fixture-gated: the New Game+ state of real saves and `newgameplus::turn_off`. Skipped (passes) when
//! `W3AL_FIXTURES` is unset. Fixtures are copies of real saves and are never committed.
use std::path::{Path, PathBuf};

use w3save::newgameplus::{self, State};
use w3save::{Container, Document};

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

fn parse(bytes: &[u8]) -> Option<(Container, Document)> {
    let c = Container::read(bytes).ok()?;
    let d = Document::parse(&c.payload, c.data_start).ok()?;
    Some((c, d))
}

/// On every 5.x fixture: the two facts and the `active` flag agree (a New Game+ save has all three, a normal save
/// none), and `turn_off` leaves a save that re-reads clean, differs from the original in exactly the two facts and
/// the one bool, still round-trips and passes check-save. A normal save is left byte-identical.
#[test]
fn turn_off_on_every_fixture() {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    let (mut ngp, mut normal) = (0, 0);
    for p in files {
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let Some((mut c, mut d)) = parse(&bytes) else { continue };
        let Ok(before) = newgameplus::read(&d) else { continue };
        let what = p.display().to_string();
        if before.is_new_game_plus() {
            ngp += 1;
            assert!(before.level.is_some() && before.active, "{what}: a New Game+ save has the level fact and active: {before:?}");
            let facts_before = d.facts_table().unwrap();
            let ch = newgameplus::turn_off(&mut d).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(ch.removed_facts, vec!["NewGamePlus", "FinalNewGamePlusLevel"], "{what}");
            assert!(ch.after.is_clean() && ch.after.enable == before.enable && ch.after.ngp_items == before.ngp_items, "{what}: {:?}", ch.after);
            c.payload = d.to_bytes();
            let out = c.write().unwrap();
            let (c2, d2) = parse(&out).unwrap_or_else(|| panic!("{what}: the edited save does not parse"));
            assert_eq!(d2.to_bytes(), c2.payload, "{what}: typed re-serialize");
            assert_eq!(d2.table(), d2.map.variables, "{what}: variable table recomputed from the tree");
            let after: State = newgameplus::read(&d2).unwrap();
            assert!(after.is_clean() && !after.is_new_game_plus(), "{what}: {after:?}");
            // exactly the two facts differ
            let mut diff = facts_before.differing(&d2.facts_table().unwrap());
            diff.sort();
            assert_eq!(diff, vec!["FinalNewGamePlusLevel", "NewGamePlus"], "{what}");
            // the rest of the tree is the same: the top-level nodes other than `facts` and `NewGamePlus`
            let (orig, _) = parse(&bytes).unwrap();
            let orig_doc = Document::parse(&orig.payload, orig.data_start).unwrap();
            for (x, y) in orig_doc.top_nodes().zip(d2.top_nodes()) {
                let name = w3save::graph::node_name(&orig_doc, x);
                if matches!(x.kind, w3save::Kind::Manu(_)) || name == "facts" || name == newgameplus::BLOCK {
                    continue;
                }
                assert_eq!(x.to_bytes(), y.to_bytes(), "{what}: top-level {name} changed");
            }
            let info = w3save::inspect(&out).unwrap();
            assert!(info.issues.iter().all(|i| i.severity != w3save::Severity::Error), "{what}: check-save {:?}", info.issues);
        } else if before.is_clean() {
            normal += 1;
            let payload = d.to_bytes();
            let ch = newgameplus::turn_off(&mut d).unwrap();
            assert!(ch.removed_facts.is_empty(), "{what}");
            assert_eq!(d.to_bytes(), payload, "{what}: a save that is not a New Game+ must stay byte-identical");
        } else {
            // e.g. Blood and Wine's NG+ menu save: Enable true, active false, no fact: not a New Game+ either
            assert!(!before.is_new_game_plus(), "{what}: {before:?}");
        }
    }
    eprintln!("turn_off: {ngp} New Game+ saves, {normal} normal saves");
}
