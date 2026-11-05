//! Fixture-gated (skipped when `W3AL_FIXTURES` is unset): the quest-graph tools on real saves
//! (docs/findings/quest-graph-design.md).
//!
//! * every 5.x save: the count rules of `graph::COUNT_RULES` hold, and every `ExternalDialog` guid is
//!   the GUID of a `questBlock` in the same save's frontier (the scene-player list mirrors waiting scenes);
//! * consecutive numbered saves (lukwraith): replaying the keyed diff of `questSystem` and `community`
//!   on A gives B's blocks byte for byte (the 21 -> 22 journal result, for the graph);
//! * lukwraith 21 -> 22 (mq0005): the quest's own delta is four scopes plus the scene-player list; the
//!   scoped transplant makes those nodes equal to 22's and leaves every other graph scope as 21's;
//! * three more quests (Precious Cargo 24 -> 25, Missing in Action 25 -> 26, Beast of White Orchard
//!   36 -> 37): the scoped replay of their changed scopes is byte-exact for those scopes.
//!
//! Name remapping (docs/findings/name-remap.md):
//! * every opaque value of `questSystem` and `community` in every fixture decodes: the identity remap
//!   gives the same bytes, a scrambled map and its inverse give the same bytes back, every index
//!   position holds a valid NM index;
//! * remap mode on every consecutive pair equals B (byte-exact where B's names extend A's; by
//!   `resolved_fingerprint` for 06 -> 07, whose names are not a prefix);
//! * two donors that branch from one save (lukwraith 05 -> 06 and 05 -> 07, conflicting extra names)
//!   both go onto 05; each transplanted scope equals its donor's;
//! * whole `questSystem` + `community` from other playthroughs onto a lukwraith save.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use w3save::graph::{self, Change, GraftOp, NameMode, COUNT_RULES};
use w3save::{Container, Document, Kind, Node, Value};

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

fn fixtures() -> Option<Vec<PathBuf>> {
    let dir = std::env::var_os("W3AL_FIXTURES")?;
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    Some(files)
}

fn load(p: &Path) -> Document {
    let c = Container::read(&std::fs::read(p).unwrap()).unwrap();
    Document::parse(&c.payload, c.data_start).unwrap()
}

fn is_5x(d: &Document) -> bool {
    matches!(d.map.header_ints, [66 | 67, 29, 164])
}

/// Saves in `NN. title` folders, grouped by playthrough folder, in folder-number order.
fn numbered(files: &[PathBuf]) -> BTreeMap<PathBuf, Vec<(u32, PathBuf)>> {
    let mut sets: BTreeMap<PathBuf, Vec<(u32, PathBuf)>> = BTreeMap::new();
    for f in files {
        let Some(folder) = f.parent() else { continue };
        let name = folder.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let Some((num, _)) = name.split_once(". ") else { continue };
        let Ok(n) = num.parse::<u32>() else { continue };
        sets.entry(folder.parent().unwrap().to_owned()).or_default().push((n, f.clone()));
    }
    for v in sets.values_mut() {
        v.sort();
    }
    sets
}

fn find(files: &[PathBuf], suffix: &str) -> Option<PathBuf> {
    files.iter().find(|f| f.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.ends_with(suffix))).cloned()
}

fn uint(d: &Document, n: &Node) -> Option<i64> {
    match &n.kind {
        Kind::Var(v) => match v.value {
            Value::U32(x) => Some(i64::from(x)),
            Value::I32(x) => Some(i64::from(x)),
            _ => None,
        },
        _ => {
            let _ = d;
            None
        }
    }
}

/// Count-rule and scene-list violations of one save, as messages.
fn rule_violations(d: &Document) -> (usize, Vec<String>) {
    let mut checked = 0usize;
    let mut bad = Vec::new();
    let mut block_guids: BTreeSet<[u8; 16]> = BTreeSet::new();
    let mut dialog_guids: Vec<(String, [u8; 16])> = Vec::new();
    d.visit(&mut |n| {
        let name = graph::node_name(d, n);
        let kids = n.children();
        for (parent, var, of) in COUNT_RULES {
            if name != parent {
                continue;
            }
            let Some(c) = kids.iter().find(|k| matches!(k.kind, Kind::Var(_)) && graph::node_name(d, k) == var) else { continue };
            let want = kids.iter().filter(|k| matches!(k.kind, Kind::Block(_)) && graph::node_name(d, k) == of).count() as i64;
            checked += 1;
            if uint(d, c) != Some(want) {
                bad.push(format!("{parent}/{var} = {:?} but {want} {of}", uint(d, c)));
            }
        }
        let guid = |field: &str| {
            kids.iter().find_map(|k| match &k.kind {
                Kind::Var(v) if d.name(v.name) == Some(field) => match v.value {
                    Value::Guid(g) => Some(g),
                    _ => None,
                },
                _ => None,
            })
        };
        if name == "questBlock" {
            if let Some(g) = guid("GUID") {
                block_guids.insert(g);
            }
        }
        if name == "ExternalDialog" {
            if let Some(g) = guid("guid") {
                dialog_guids.push((graph::identity(d, n).unwrap_or_default(), g));
            }
        }
    });
    for (tag, g) in dialog_guids {
        checked += 1;
        if !block_guids.contains(&g) {
            bad.push(format!("ExternalDialog {tag} guid is no questBlock of the frontier"));
        }
    }
    (checked, bad)
}

#[test]
fn count_rules_and_scene_list_hold_on_every_5x_fixture() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (mut saves, mut checked, mut failures) = (0usize, 0usize, Vec::new());
    for f in &files {
        let d = load(f);
        if !is_5x(&d) || graph::top_block(&d, "questSystem").is_none() {
            continue;
        }
        saves += 1;
        let (n, bad) = rule_violations(&d);
        checked += n;
        failures.extend(bad.into_iter().map(|b| format!("{}: {b}", f.display())));
    }
    eprintln!("graph rules: {saves} saves, {checked} counts/scene guids checked, {} failures", failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const BLOCKS: [&str; 2] = ["questSystem", "community"];

fn all_changes(a: &Document, b: &Document) -> Vec<Change> {
    BLOCKS.iter().flat_map(|n| graph::diff_top(a, b, n).unwrap().unwrap_or_default()).collect()
}

fn block_bytes(d: &Document, name: &str) -> Vec<u8> {
    graph::top_block(d, name).unwrap().to_bytes()
}

#[test]
fn consecutive_graph_diffs_replay_byte_exact() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (mut pairs, mut exact, mut refused, mut failures) = (0usize, 0usize, Vec::new(), Vec::new());
    for list in numbered(&files).values() {
        for w in list.windows(2) {
            let ((na, fa), (nb, fb)) = (&w[0], &w[1]);
            let (mut a, b) = (load(fa), load(fb));
            if !is_5x(&a) || !is_5x(&b) {
                continue;
            }
            pairs += 1;
            let ops = graph::ops_from_diff(&all_changes(&a, &b), &[]);
            if !graph::names_prefix(&a, &b) {
                // The transplant must refuse, not guess.
                assert!(graph::transplant(&mut a.clone(), &b, &ops).is_err());
                refused.push(format!("{na:02}->{nb:02}"));
                continue;
            }
            match graph::transplant(&mut a, &b, &ops) {
                Ok(_) if BLOCKS.iter().all(|n| block_bytes(&a, n) == block_bytes(&b, n)) => exact += 1,
                Ok(_) => failures.push(format!("{na:02}->{nb:02}: replay differs from B")),
                Err(e) => failures.push(format!("{na:02}->{nb:02}: {e}")),
            }
        }
    }
    eprintln!("graph replay: {pairs} pairs, {exact} byte-exact, refused (NM table not a prefix): {refused:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const Q: &str = "questSystem/quest[witcher3_quest.w2quest]/questThread/questThread[7b125918]/questThread";

/// The mq0005 scopes found by diffing two independent real completions (lukwraith 21 -> 22 and the
/// user's own play from gate J1): the quest's phase, the two waits that react to its end, and the scene list.
fn mq0005_scopes() -> Vec<String> {
    vec![
        "questSystem/questExternalScenePlayers/CQuestExternalScenePlayer".into(),
        format!("{Q}/questThread[f0b52e98]/questThread/questThread[b4e73851]/questThread/questThread[38424c39]"),
        format!("{Q}/questThread[988006f0]/questThread/questThread[55d4b37b]/questThread/questThread[83ca614d]"),
        format!("{Q}/questThread[988006f0]/questThread/questThread[61545c9b]/questThread/questThread[a43cea18]/questThread/questThread[089c7277]/questThread/questThread[408d8753]"),
    ]
}

fn subtree(d: &Document, path: &str) -> Option<String> {
    graph::locate(d, path).unwrap().map(|l| graph::fingerprint(d, graph::node_at(d, &l)))
}

#[test]
fn lukwraith_21_to_22_mq0005_scoped_transplant() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (Some(fa), Some(fb)) = (find(&files, "_3f4cf08.sav"), find(&files, "_4038706.sav")) else {
        eprintln!("lukwraith saves 21/22 not under W3AL_FIXTURES; skipping");
        return;
    };
    let (a, b) = (load(&fa), load(&fb));
    let changes = graph::diff_top(&a, &b, "questSystem").unwrap().unwrap();
    // The numbers of docs/findings/quest-graph-design.md: 34 changes, 12 of them in the mq0005 scopes.
    assert_eq!(changes.len(), 34);
    let scopes = mq0005_scopes();
    let ops = graph::ops_from_diff(&changes, &scopes);
    assert_eq!(ops.len(), 16, "{ops:#?}");
    // The scene that drives Napp's dialogue (ExternalDialog mq0005_arsonist, guid 300f579b) waits in thread 3446e000.
    assert!(ops.iter().any(|o| matches!(o, GraftOp::Remove { path } if path.ends_with("questThread[3446e000]"))));
    assert!(ops.iter().any(|o| matches!(o, GraftOp::Remove { path } if path.ends_with("ExternalDialog[mq0005_arsonist]"))));

    let mut t = a.clone();
    let r = graph::transplant(&mut t, &b, &ops).unwrap();
    assert_eq!((r.removed, r.inserted, r.recounted.len()), (7, 5, 4));
    for s in &scopes {
        assert_eq!(subtree(&t, s), subtree(&b, s), "{s} differs from 22");
    }
    // Every other change of 21 -> 22 is still open (the transplant did not touch other quests).
    let rest = graph::diff_top(&t, &b, "questSystem").unwrap().unwrap();
    assert!(!rest.is_empty());
    assert!(rest.iter().all(|c| !scopes.iter().any(|s| c.path().starts_with(s.as_str()))));
    let untouched = graph::diff_top(&a, &t, "questSystem").unwrap().unwrap();
    assert!(untouched.iter().all(|c| scopes.iter().any(|s| c.path().starts_with(s.as_str()))), "{untouched:#?}");
    // Other blocks are byte-identical to 21 (names: 22's extra names were appended).
    for name in ["community", "universe", "facts", "CJournalManager"] {
        assert_eq!(block_bytes(&t, name), block_bytes(&a, name), "{name}");
    }
    assert_eq!(t.names.as_ref().unwrap(), b.names.as_ref().unwrap());

    // Community: the arsonist's spawn set and stub, removed the way the game does it (swap-remove).
    let community = [
        GraftOp::Remove { path: "community/communities/c[mq0005_arsonist.w2comm]".into() },
        GraftOp::Recount { path: "community/communities/numCommunities".into(), of: "c".into() },
        GraftOp::SwapRemove { path: "community/stubs/a#8".into() },
        GraftOp::Recount { path: "community/stubs/numStubs".into(), of: "a".into() },
    ];
    let stub_tail = subtree(&a, "community/stubs/a#19").unwrap();
    graph::transplant(&mut t, &b, &community).unwrap();
    assert_eq!(subtree(&t, "community/stubs/a#8").unwrap(), stub_tail, "the last stub takes the removed slot");
    assert!(subtree(&t, "community/stubs/a#19").is_none());
    assert!(subtree(&t, "community/communities/c[mq0005_arsonist.w2comm]").is_none());
    let (_, bad) = rule_violations(&t);
    assert!(bad.is_empty(), "{bad:?}");
}

/// The changed scopes of a diff: for each change, the deepest `questThread[GUID]` step of its path that
/// is not itself the changed node (else the parent path).
fn scopes_of(changes: &[Change]) -> BTreeSet<String> {
    let mut scopes = BTreeSet::new();
    for c in changes {
        let p = c.path();
        let cut = p.rmatch_indices("/questThread[").map(|(i, _)| i).find(|&i| p[i + 1..].contains('/'));
        scopes.insert(cut.map_or_else(|| p.rsplit_once('/').map_or(p, |x| x.0).to_owned(), |i| {
            let end = p[i + 1..].find('/').map_or(p.len(), |e| i + 1 + e);
            p[..end].to_owned()
        }));
    }
    scopes
}

/// For each pair, every changed graph scope (deepest wrapper `questThread[..]` above a change) replays
/// byte-exact on its own: the delta of one scope never needs a change elsewhere to verify.
#[test]
fn other_quests_replay_per_scope() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    // (before, after, event) from docs/findings/quest-state.md.
    let pairs = [
        ("_412dab3.sav", "_4d2b311.sav", "24->25 mq0002 Precious Cargo completed"),
        ("_4d2b311.sav", "_4d76f6a.sav", "25->26 Missing in Action"),
        ("_46a305c.sav", "_3bb2d59.sav", "36->37 q002 Beast of White Orchard completed"),
    ];
    let mut done = 0usize;
    for (sa, sb, what) in pairs {
        let (Some(fa), Some(fb)) = (find(&files, sa), find(&files, sb)) else { continue };
        let (a, b) = (load(&fa), load(&fb));
        let changes = graph::diff_top(&a, &b, "questSystem").unwrap().unwrap();
        let scopes = scopes_of(&changes);
        for s in &scopes {
            let ops = graph::ops_from_diff(&changes, std::slice::from_ref(s));
            let mut t = a.clone();
            graph::transplant(&mut t, &b, &ops).unwrap_or_else(|e| panic!("{what}: {s}: {e}"));
            assert_eq!(subtree(&t, s), subtree(&b, s), "{what}: {s}");
        }
        eprintln!("{what}: {} graph changes in {} scopes, each replays on its own", changes.len(), scopes.len());
        done += 1;
    }
    if done == 0 {
        eprintln!("lukwraith pairs not under W3AL_FIXTURES; skipping");
    }
}

// ---------------------------------------------------------------------------------------------
// Name remapping

fn names(d: &Document) -> Vec<String> {
    d.names.clone().expect("NM table")
}

fn bad_index(i: u16) -> w3save::Error {
    w3save::Error::Unverified(format!("index {i} outside the NM table"))
}

/// Every opaque value (and `SS` section) of `questSystem` and `community` in every fixture, by type,
/// and that each top block decodes completely: the identity remap reproduces its bytes (the layouts
/// tile every value exactly), every decoded index position holds a valid NM index, and a scrambled
/// table (i -> n + 1 - i) and back gives the same bytes (the remapped block is decodable on its own).
#[test]
fn every_opaque_value_in_the_graph_blocks_decodes() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (mut saves, mut failures) = (0usize, Vec::new());
    let (mut inventory, mut inputs) = (BTreeMap::<String, usize>::new(), BTreeMap::<String, usize>::new());
    for f in &files {
        let d = load(f);
        if !is_5x(&d) {
            continue;
        }
        saves += 1;
        let table = names(&d);
        let n = table.len() as u16;
        let flipped: Vec<String> = table.iter().rev().cloned().collect();
        for blk in BLOCKS {
            let Some(top) = graph::top_block(&d, blk) else { continue };
            top.visit(&mut |x| match &x.kind {
                Kind::Var(v) if matches!(v.value, Value::Opaque(_)) => {
                    let ty = d.name(v.ty).unwrap_or("?");
                    *inventory.entry(format!("{blk:<11} {ty}")).or_default() += 1;
                    if ty == "array:2,0,CName" {
                        let b = x.to_bytes();
                        for k in (10..b.len()).step_by(2) {
                            let i = u16::from_le_bytes([b[k], b[k + 1]]);
                            let first = d.name(i).unwrap_or("<not a name>").split(' ').next().unwrap_or_default().to_owned();
                            *inputs.entry(first).or_default() += 1;
                        }
                    }
                }
                Kind::Sized(_) => *inventory.entry(format!("{blk:<11} SS section")).or_default() += 1,
                _ => {}
            });
            let at = format!("{}: {blk}", f.display());
            match graph::remap_node(&table, top, &mut |i| if (1..=n).contains(&i) { Ok(i) } else { Err(bad_index(i)) }) {
                Ok(same) if same.to_bytes() == top.to_bytes() => {}
                Ok(_) => failures.push(format!("{at}: identity remap changed bytes")),
                Err(e) => failures.push(format!("{at}: {e}")),
            }
            let back = graph::remap_node(&table, top, &mut |i| Ok(n + 1 - i))
                .and_then(|s| graph::remap_node(&flipped, &s, &mut |i| Ok(n + 1 - i)));
            match back {
                Ok(same) if same.to_bytes() == top.to_bytes() => {}
                Ok(_) => failures.push(format!("{at}: scrambled and back differs")),
                Err(e) => failures.push(format!("{at}: scrambled: {e}")),
            }
        }
    }
    eprintln!("opaque values of questSystem/community in {saves} saves (all decode: {}):", failures.is_empty());
    for (k, v) in &inventory {
        eprintln!("  {v:>8}  {k}");
    }
    eprintln!("ActivatedInputs element names by first word: {inputs:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // Every ActivatedInputs element names an input pin ("In", "In 1", ..).
    assert!(inputs.is_empty() || inputs.keys().eq(["In"]), "{inputs:?}");
}

/// Var/SS nodes holding opaque data, by keyed path (step + occurrence), under `n`.
fn opaque_nodes<'a>(d: &Document, n: &'a Node, path: &str, out: &mut BTreeMap<String, &'a Node>) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for c in n.children() {
        let s = graph::step(d, c);
        let k = seen.entry(s.clone()).or_default();
        let p = format!("{path}/{s}#{k}");
        *k += 1;
        match &c.kind {
            Kind::Var(v) if matches!(v.value, Value::Opaque(_)) => {
                out.insert(p, c);
            }
            Kind::Sized(_) => {
                out.insert(p, c);
            }
            Kind::Block(_) => opaque_nodes(d, c, &p, out),
            _ => {}
        }
    }
}

/// Independent evidence that the decoder finds EVERY name index: where two saves with different NM
/// tables hold the same value (same content by `resolved_fingerprint`), translating one save's bytes
/// into the other's table must give exactly the bytes the GAME wrote in the other save. A missed index
/// position would keep the first save's number there. Pairs: the first and last save of each fixture
/// folder against those of every other folder, plus lukwraith 06 / 07.
#[test]
fn translated_values_equal_the_games_own_bytes_across_name_tables() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let root = PathBuf::from(std::env::var_os("W3AL_FIXTURES").unwrap());
    let mut by_folder: BTreeMap<PathBuf, Vec<&PathBuf>> = BTreeMap::new();
    for f in &files {
        let top = f.strip_prefix(&root).ok().and_then(|r| r.components().next()).map(|c| root.join(c));
        by_folder.entry(top.unwrap_or_default()).or_default().push(f);
    }
    let mut docs: Vec<(PathBuf, Document)> = Vec::new();
    for (folder, list) in &by_folder {
        for f in [list[0], list[list.len() - 1]] {
            let d = load(f);
            if is_5x(&d) {
                docs.push((folder.clone(), d));
            }
        }
    }
    let mut extra = Vec::new();
    if let (Some(f6), Some(f7)) = (find(&files, "_4544c15.sav"), find(&files, "_b2dd10.sav")) {
        extra.push((load(&f6), load(&f7)));
    }
    let mut pairs: Vec<(&Document, &Document)> = Vec::new();
    for (i, (ga, a)) in docs.iter().enumerate() {
        for (gb, b) in &docs[i + 1..] {
            if ga != gb && a.names != b.names {
                pairs.push((a, b));
            }
        }
    }
    pairs.extend(extra.iter().map(|(a, b)| (a, b)));
    let (mut checked, mut translated, mut failures) = (BTreeMap::<String, usize>::new(), BTreeMap::<String, usize>::new(), Vec::new());
    for (a, b) in &pairs {
        let ta = names(a);
        for blk in BLOCKS {
            let (Some(xa), Some(xb)) = (graph::top_block(a, blk), graph::top_block(b, blk)) else { continue };
            let (mut ma, mut mb) = (BTreeMap::new(), BTreeMap::new());
            opaque_nodes(a, xa, blk, &mut ma);
            opaque_nodes(b, xb, blk, &mut mb);
            for (p, na) in &ma {
                let Some(nb) = mb.get(p) else { continue };
                if graph::resolved_fingerprint(a, na).unwrap() != graph::resolved_fingerprint(b, nb).unwrap() {
                    continue;
                }
                let ty = match &na.kind {
                    Kind::Var(v) => a.name(v.ty).unwrap_or("?").to_owned(),
                    _ => "SS section".into(),
                };
                let mapped = graph::remap_node(&ta, na, &mut |i| b.name_index(&ta[i as usize - 1]).ok_or_else(|| bad_index(i))).unwrap();
                // The value bytes only (a Var's own name and type are not opaque).
                let body = |n: &Node| n.to_bytes()[6..].to_vec();
                *checked.entry(ty.clone()).or_default() += 1;
                if body(na) != body(nb) {
                    *translated.entry(ty.clone()).or_default() += 1;
                }
                if body(&mapped) != body(nb) {
                    failures.push(format!("{p} ({ty})"));
                }
            }
        }
    }
    eprintln!("{} save pairs with different NM tables; equal values checked, by type: {checked:?}", pairs.len());
    eprintln!("  of those, values whose bytes differ between the two saves (indices translated): {translated:?}");
    assert!(failures.is_empty(), "{} values differ from the game's bytes after translation:\n{}", failures.len(), failures.join("\n"));
    if !pairs.is_empty() {
        assert!(!translated.is_empty(), "no equal value needed translation; the check proved nothing");
    }
}

#[test]
fn two_donors_from_two_branches_onto_one_template() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (Some(f5), Some(f6), Some(f7)) = (find(&files, "_44f1395.sav"), find(&files, "_4544c15.sav"), find(&files, "_b2dd10.sav")) else {
        eprintln!("lukwraith saves 05/06/07 not under W3AL_FIXTURES; skipping");
        return;
    };
    let (t, d1, d2) = (load(&f5), load(&f6), load(&f7));
    // Both donors were played from 05 (its names are a prefix of both), but not from each other, and
    // their first extra name differs: the same index means different names in 06 and 07.
    let (n5, n6, n7) = (names(&t).len(), names(&d1), names(&d2));
    assert!(graph::names_prefix(&t, &d1) && graph::names_prefix(&t, &d2));
    assert!(!graph::names_prefix(&d1, &d2) && !graph::names_prefix(&d2, &d1));
    assert_ne!(n6[n5], n7[n5]);
    // 05 -> 06 only moves NPCs: 17 `community` stubs change. 05 -> 07 is a quest step: 33 `questSystem`
    // scopes, 20 stubs, and a spawn set with its 3 stubs (a#20..a#22) removed. Delta 1 = all of 06's;
    // delta 2 = 07's quest graph plus the stubs 06 did not touch (a#10, a#16..a#19; a#18/a#19 use names
    // that only 07 has). 07's spawn-set removal stays out: it would drop 06's a#20/a#21.
    let c1 = all_changes(&t, &d1);
    assert!(c1.iter().all(|c| c.path().starts_with("community/stubs/a")), "05 -> 06 is not only stub changes");
    let c2 = all_changes(&t, &d2);
    let s1 = scopes_of(&c1);
    let stub = |s: &str| s.split('/').take(3).collect::<Vec<_>>().join("/");
    let stubs2: BTreeSet<String> = scopes_of(&c2).iter().filter(|s| s.starts_with("community/stubs/a")).map(|s| stub(s)).filter(|s| !s1.contains(s)).collect();
    assert!(!stubs2.is_empty());
    let mut scopes2: Vec<String> = vec!["questSystem".into()];
    scopes2.extend(stubs2.iter().cloned());
    let (ops1, ops2) = (graph::ops_from_diff(&c1, &[]), graph::ops_from_diff(&c2, &scopes2));
    let check = |r: &Document, order: &str| {
        let fp = |doc: &Document, path: &str| graph::locate(doc, path).unwrap().map(|l| graph::resolved_fingerprint(doc, graph::node_at(doc, &l)).unwrap());
        for (d, scopes) in [(&d1, s1.iter().collect::<Vec<_>>()), (&d2, scopes2.iter().collect())] {
            for s in scopes {
                assert_eq!(fp(r, s), fp(d, s), "{order}: {s} differs from its donor's");
            }
        }
        // Nothing else moved: the spawn sets and the stub list are still 05's.
        for p in ["community/communities", "community/stubs/numStubs", "community/stubs/a#22"] {
            assert_eq!(fp(r, p), fp(&t, p), "{order}: {p}");
        }
        let (n, bad) = rule_violations(r);
        assert!(bad.is_empty(), "{order}: {bad:?}");
        n
    };

    // 06 then 07: 06 extends 05 (its one extra name is appended, identity); 07 then needs a real remap.
    let mut a = t.clone();
    let r1 = graph::transplant_with(&mut a, &d1, &ops1, NameMode::Remap).unwrap();
    assert_eq!((r1.names_added, r1.names_remapped), (1, 0));
    assert!(graph::transplant(&mut a.clone(), &d2, &ops2).is_err(), "prefix mode must refuse the second donor");
    let r2 = graph::transplant_with(&mut a, &d2, &ops2, NameMode::Remap).unwrap();
    assert!(r2.names_remapped > 0, "07's delta uses none of its extra names; the test proves nothing");
    let checks = check(&a, "06 then 07");
    let table = names(&a);
    eprintln!("names appended for 07 after 06: {:?}", &table[table.len() - r2.names_added..]);
    // 07 then 06: the other way round.
    let mut b = t.clone();
    let r3 = graph::transplant_with(&mut b, &d2, &ops2, NameMode::Remap).unwrap();
    let r4 = graph::transplant_with(&mut b, &d1, &ops1, NameMode::Remap).unwrap();
    check(&b, "07 then 06");
    for blk in BLOCKS {
        let fp = |doc: &Document| graph::resolved_fingerprint(doc, graph::top_block(doc, blk).unwrap()).unwrap();
        assert_eq!(fp(&a), fp(&b), "{blk}: the order of the two donors matters");
    }
    eprintln!(
        "two donors on 05: 06 ({} ops in {} scopes) then 07 ({} ops: questSystem + stubs {stubs2:?}; {} names appended, {} donor indices moved); \
         07 then 06 ({} + {} names appended, {} moved); {checks} count/scene checks hold",
        ops1.len(),
        s1.len(),
        ops2.len(),
        r2.names_added,
        r2.names_remapped,
        r3.names_added,
        r4.names_added,
        r4.names_remapped
    );
}

/// Another playthrough as donor: the whole `questSystem` and `community` of the last save of every other
/// fixture folder onto lukwraith 21 (offline only; nothing says the game would run such a mix).
#[test]
fn whole_graph_blocks_from_other_playthroughs() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let Some(ft) = find(&files, "_3f4cf08.sav") else {
        eprintln!("lukwraith save 21 not under W3AL_FIXTURES; skipping");
        return;
    };
    let t = load(&ft);
    let lukwraith = ft.ancestors().nth(3).map(Path::to_path_buf).unwrap_or_default();
    let mut last: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for f in files.iter().filter(|f| !f.starts_with(&lukwraith)) {
        last.insert(f.parent().unwrap().to_owned(), f.clone());
    }
    let ops = [GraftOp::Replace { path: "questSystem".into() }, GraftOp::Replace { path: "community".into() }];
    let mut done = Vec::new();
    for f in last.values() {
        let d = load(f);
        if !is_5x(&d) || graph::names_prefix(&t, &d) || BLOCKS.iter().any(|b| graph::top_block(&d, b).is_none()) {
            continue;
        }
        let mut r = t.clone();
        let rep = graph::transplant_with(&mut r, &d, &ops, NameMode::Remap).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        for blk in BLOCKS {
            let fp = |doc: &Document| graph::resolved_fingerprint(doc, graph::top_block(doc, blk).unwrap()).unwrap();
            assert_eq!(fp(&r), fp(&d), "{}: {blk}", f.display());
        }
        for blk in ["facts", "CJournalManager", "universe"] {
            assert_eq!(block_bytes(&r, blk), block_bytes(&t, blk), "{blk}");
        }
        let (_, bad) = rule_violations(&r);
        assert!(bad.is_empty(), "{}: {bad:?}", f.display());
        done.push(format!("{} ({} names appended, {} donor indices moved)", f.file_name().unwrap().to_string_lossy(), rep.names_added, rep.names_remapped));
    }
    eprintln!("whole graph blocks from other playthroughs onto lukwraith 21: {done:?}");
}

#[test]
fn consecutive_graph_diffs_replay_in_remap_mode() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (mut pairs, mut exact, mut by_names, mut failures) = (0usize, 0usize, Vec::new(), Vec::new());
    for list in numbered(&files).values() {
        for w in list.windows(2) {
            let ((na, fa), (nb, fb)) = (&w[0], &w[1]);
            let (mut a, b) = (load(fa), load(fb));
            if !is_5x(&a) || !is_5x(&b) {
                continue;
            }
            pairs += 1;
            let prefix = graph::names_prefix(&a, &b);
            let ops = graph::ops_from_diff(&all_changes(&a, &b), &[]);
            let r = match graph::transplant_with(&mut a, &b, &ops, NameMode::Remap) {
                Ok(r) => r,
                Err(e) => {
                    failures.push(format!("{na:02}->{nb:02}: {e}"));
                    continue;
                }
            };
            for n in BLOCKS {
                let (x, y) = (graph::top_block(&a, n).unwrap(), graph::top_block(&b, n).unwrap());
                if graph::resolved_fingerprint(&a, x).unwrap() != graph::resolved_fingerprint(&b, y).unwrap() {
                    failures.push(format!("{na:02}->{nb:02}: {n} differs from B"));
                }
            }
            let (_, bad) = rule_violations(&a);
            failures.extend(bad.into_iter().map(|m| format!("{na:02}->{nb:02}: {m}")));
            if prefix {
                if BLOCKS.iter().all(|n| block_bytes(&a, n) == block_bytes(&b, n)) {
                    exact += 1;
                } else {
                    failures.push(format!("{na:02}->{nb:02}: names are a prefix but the bytes differ from B"));
                }
            } else {
                let plain = BLOCKS
                    .iter()
                    .filter(|n| graph::fingerprint(&a, graph::top_block(&a, n).unwrap()) == graph::fingerprint(&b, graph::top_block(&b, n).unwrap()))
                    .count();
                by_names.push(format!(
                    "{na:02}->{nb:02} ({} ops, {} names appended, {} donor indices moved, graph::fingerprint equal on {plain}/2 blocks)",
                    ops.len(),
                    r.names_added,
                    r.names_remapped
                ));
            }
        }
    }
    eprintln!("graph replay (remap mode): {pairs} pairs, {exact} byte-exact, equal by resolved fingerprint (names not a prefix): {by_names:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
