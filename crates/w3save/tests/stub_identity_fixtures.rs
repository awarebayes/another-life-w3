//! Fixture-gated (skipped when `W3AL_FIXTURES` is unset): NPC stubs addressed by `idTag` instead of
//! by list position (`graph::SELECTORS`, `graph::DiffOptions::stub_identity`;
//! docs/plan/decision-chaining.md).
//!
//! * every save: stub idTags are unique, and so are `universe > .. > entityData` idTags; a selector path
//!   (`community/stubs/a{idTag=..}`) finds the same node as its positional path (`a#k`);
//! * across every save under the fixture folder: an idTag never names two different spawn-set entries
//!   (spawn set file + `entityEntry`), so it is a stable identity within a playthrough;
//! * consecutive numbered saves (lukwraith): the stub-identity diff of `questSystem` + `community`
//!   replays on A to B's blocks (same stubs by idTag, same content), byte for byte where B's order is
//!   what the game's swap-remove + append gives;
//! * session V1/V2 (the user's act-2 donors under `C:\w3-fixtures`): the Keira decision (donor from
//!   T_keira) and the Letho quest closure captured from T_keira to the Letho-invited donor (pack line,
//!   remapped names) compose on T_keira; the Letho delta captured on T_letho alone would leave the
//!   quest's start triggers waiting on T_keira.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use w3save::graph::{self, DiffOptions, GraftOp, NameMode};
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

fn fixtures() -> Option<Vec<PathBuf>> {
    let dir = std::env::var_os("W3AL_FIXTURES")?;
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    Some(files)
}

fn try_load(p: &Path) -> Option<Document> {
    let c = Container::read(&std::fs::read(p).ok()?).ok()?;
    Document::parse(&c.payload, c.data_start).ok()
}

fn find(files: &[PathBuf], stem: &str) -> Option<PathBuf> {
    files.iter().find(|f| f.file_stem().and_then(|s| s.to_str()) == Some(stem)).cloned()
}

/// idTags of `universe > world[..] > layerStorage > entityData`.
fn entity_data_tags(d: &Document) -> Vec<String> {
    let Some(u) = graph::top_block(d, "universe") else { return Vec::new() };
    graph::render(d, u)
        .into_iter()
        .filter(|l| l.path.ends_with("/layerStorage/entityData/idTag"))
        .map(|l| l.text.rsplit(' ').next().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn stub_idtags_are_unique_and_selector_paths_resolve() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (mut checked, mut stubs, mut located) = (0usize, 0usize, 0usize);
    for f in &files {
        let Some(d) = try_load(f) else { continue };
        let list = graph::stubs(&d);
        if list.is_empty() {
            continue;
        }
        checked += 1;
        let tags: Vec<&str> = list.iter().map(|s| s.id_tag.as_deref().expect("every stub has an idTag")).collect();
        assert_eq!(tags.iter().collect::<HashSet<_>>().len(), tags.len(), "{}: duplicate stub idTag", f.display());
        let ent = entity_data_tags(&d);
        assert_eq!(ent.iter().collect::<HashSet<_>>().len(), ent.len(), "{}: duplicate entityData idTag", f.display());
        stubs += list.len();
        // Selector and positional paths name the same node (first 3 saves only, it is quadratic).
        if checked <= 3 {
            for s in &list {
                let by_tag = graph::locate(&d, &format!("community/stubs/a{{idTag={}}}", s.id_tag.as_deref().unwrap())).unwrap();
                let k = if s.index == 0 { String::new() } else { format!("#{}", s.index) };
                let by_pos = graph::locate(&d, &format!("community/stubs/a{k}")).unwrap();
                assert!(by_tag.is_some() && by_tag == by_pos, "{}: a#{} vs its idTag", f.display(), s.index);
                located += 1;
            }
        }
    }
    eprintln!("stub identity: {checked} saves, {stubs} stubs, all idTags unique per save; {located} selector paths checked");
}

#[test]
fn an_idtag_never_names_two_spawn_set_entries() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let mut seen: HashMap<String, (String, String, PathBuf)> = HashMap::new();
    let (mut rows, mut saves_read) = (0usize, 0usize);
    let mut appearance_changes = 0usize;
    let mut last_app: HashMap<String, String> = HashMap::new();
    for f in &files {
        let Some(d) = try_load(f) else { continue };
        saves_read += 1;
        for s in graph::stubs(&d) {
            let tag = s.id_tag.clone().unwrap();
            rows += 1;
            let entry = (s.community.clone(), s.entries[2].clone());
            match seen.get(&tag) {
                Some((c, e, at)) => assert!(
                    (c, e) == (&entry.0, &entry.1),
                    "idTag {tag}: {c} / {e} in {} but {} / {} in {}",
                    at.display(),
                    entry.0,
                    entry.1,
                    f.display()
                ),
                None => {
                    seen.insert(tag.clone(), (entry.0, entry.1, f.clone()));
                }
            }
            if last_app.insert(tag, s.appearance.clone()).is_some_and(|a| a != s.appearance) {
                appearance_changes += 1;
            }
        }
    }
    eprintln!(
        "{saves_read} saves, {rows} stub rows, {} distinct idTags, none names two spawn-set entries; appearance is mutable ({appearance_changes} changes)",
        seen.len()
    );
}

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

const BLOCKS: [&str; 2] = ["questSystem", "community"];
const BY_ID: DiffOptions = DiffOptions { stub_identity: true };

#[test]
fn stub_identity_diffs_replay_on_consecutive_saves() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let (mut pairs, mut exact, mut same_set, mut failures) = (0usize, 0usize, 0usize, Vec::new());
    let (mut positional_ops, mut identity_ops) = (0usize, 0usize);
    for list in numbered(&files).values() {
        for w in list.windows(2) {
            let ((na, fa), (nb, fb)) = (&w[0], &w[1]);
            let (Some(mut a), Some(b)) = (try_load(fa), try_load(fb)) else { continue };
            if !graph::names_prefix(&a, &b) || graph::stubs(&a).is_empty() {
                continue;
            }
            pairs += 1;
            let changes: Vec<_> = BLOCKS.iter().flat_map(|n| graph::diff_top_with(&a, &b, n, BY_ID).unwrap().unwrap_or_default()).collect();
            let ops = graph::ops_from_diff(&changes, &[]);
            let stub_ops = |ops: &[GraftOp]| ops.iter().filter(|o| o.path().starts_with("community/stubs/a")).count();
            identity_ops += stub_ops(&ops);
            let positional: Vec<_> = BLOCKS.iter().flat_map(|n| graph::diff_top(&a, &b, n).unwrap().unwrap_or_default()).collect();
            positional_ops += stub_ops(&graph::ops_from_diff(&positional, &[]));
            if let Err(e) = graph::transplant(&mut a, &b, &ops) {
                failures.push(format!("{na:02}->{nb:02}: {e}"));
                continue;
            }
            if BLOCKS.iter().all(|n| graph::top_block(&a, n).unwrap().to_bytes() == graph::top_block(&b, n).unwrap().to_bytes()) {
                exact += 1;
                continue;
            }
            // Not byte-exact: then only the stub order may differ (the diff ignores it).
            let left: Vec<_> = BLOCKS.iter().flat_map(|n| graph::diff_top_with(&a, &b, n, BY_ID).unwrap().unwrap_or_default()).collect();
            if left.is_empty() {
                same_set += 1;
            } else {
                failures.push(format!("{na:02}->{nb:02}: {} differences left, first {:?}", left.len(), left[0]));
            }
        }
    }
    eprintln!(
        "stub-identity replay: {pairs} pairs, {exact} byte-exact, {same_set} equal up to stub order; stub ops: {identity_ops} by idTag vs {positional_ops} by position"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const Q: &str = "questSystem/quest[witcher3_quest.w2quest]/questThread/questThread[7b125918]/questThread";
const SP: &str = "questSystem/questExternalScenePlayers/CQuestExternalScenePlayer";

/// The scopes of the Keira decision (to Kaer Morhen), T_keira -> donor (session V1).
fn keira_scopes() -> Vec<String> {
    let t117 = format!("{Q}/questThread[f0b52e98]/questThread/questThread[117e4006]/questThread");
    let t20b = format!("{Q}/questThread[988006f0]/questThread/questThread[55d4b37b]/questThread/questThread[3560bc4e]/questThread/questThread[20babdd8]");
    vec![
        format!("{SP}/ExternalDialog[keira]"),
        format!("{SP}/tagsCount"),
        format!("{t117}/questBlock[7ffa6436]"),
        format!("{t117}/numBlocks"),
        format!("{t20b}/questThread/questBlock[973a4357]"),
        format!("{t20b}/questThread/questThread[973a4357]"),
        format!("{Q}/questThread[988006f0]/questThread/questThread[37028f15]/questThread/questThread[143c1d62]/questThread/questThread[a536c75d]"),
        "community/communities/c[keira_metz.w2comm]".into(),
        "community/stubs/a{idTag=0121659951849b055c00000000db070000}".into(),
    ]
}

const B8F: &str = "questThread[f0b52e98]/questThread/questThread[117e4006]/questThread/questThread[b8f04d50]";
const T685: &str = "questThread[988006f0]/questThread/questThread[37028f15]/questThread/questThread[279a1a03]/questThread/questThread[6850c728]";

/// The Letho quest (sq102) to "invited to Kaer Morhen", captured from T_keira (before the quest started)
/// to the user's donor: its phase scope, the long wait for Kaer Morhen, two scene-list entries, spawn sets
/// and stubs (Letho's own stub has another idTag on T_keira than on T_letho: respawned at quest start).
fn letho_closure_scopes() -> Vec<String> {
    let b531 = format!("{Q}/questThread[988006f0]/questThread/questThread[55d4b37b]/questThread/questThread[b531a276]/questThread");
    let mut v = vec![
        format!("{Q}/{B8F}"),
        format!("{Q}/{T685}"),
        format!("{b531}/questBlock[b48b318d]"),
        format!("{b531}/questBlock[980d81e0]"),
        format!("{b531}/questThread[980d81e0]"),
        format!("{b531}/numThreads"),
        format!("{SP}/ExternalDialog[letho]"),
        format!("{SP}/ExternalDialog[sq102_dolores]"),
        format!("{SP}/tagsCount"),
        "community/communities/numCommunities".into(),
        "community/stubs/numStubs".into(),
    ];
    for c in ["letho", "sq102_dolores", "sq102_dolores_evictor", "sq102_farm_headhunters", "sq102_bandits", "sq102_bandit_lookout"] {
        v.push(format!("community/communities/c[{c}.w2comm]"));
    }
    for t in ["0128b9921cd052c3e600000000cf010000", "01217fb0c8ddf5311300000000e0070000", "01c50e1ef729a41ffa00000000e1070000"] {
        v.push(format!("community/stubs/a{{idTag={t}}}"));
    }
    v
}

fn ops(a: &Document, b: &Document, scopes: &[String]) -> Vec<GraftOp> {
    let changes: Vec<_> = BLOCKS.iter().flat_map(|n| graph::diff_top_with(a, b, n, BY_ID).unwrap().unwrap_or_default()).collect();
    graph::ops_from_diff(&changes, scopes)
}

fn resolved(d: &Document, path: &str) -> Option<String> {
    graph::locate(d, path).unwrap().map(|l| graph::resolved_fingerprint(d, graph::node_at(d, &l)).unwrap())
}

#[test]
fn keira_decision_and_letho_closure_compose_on_t_keira() {
    let Some(files) = fixtures() else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let get = |s: &str| find(&files, s).and_then(|p| try_load(&p));
    let (Some(tk), Some(km), Some(tl), Some(li)) = (
        get("ManualSave_568ef_7ea51000_923cb5"),
        get("ManualSave_568ef_7ea51000_94abd5"),
        get("ManualSave_6b9be_7ea51000_9bb30d"),
        get("ManualSave_52584_7ea51000_a4841f"),
    ) else {
        eprintln!("session V1/V2 saves not under W3AL_FIXTURES; skipping");
        return;
    };
    let triggers = [
        format!("{Q}/{T685}/questThread/questBlock[72a761d5]"),
        format!("{Q}/{B8F}/questThread/questThread[15601b24]"),
        format!("{Q}/{B8F}/questThread/questThread[8fdb44ab]"),
    ];
    // The quest's start triggers wait on T_keira, are gone on T_letho (quest running) and in the donor.
    for t in &triggers {
        assert!(graph::locate(&tk, t).unwrap().is_some(), "{t} on T_keira");
        assert!(graph::locate(&tl, t).unwrap().is_none() && graph::locate(&li, t).unwrap().is_none(), "{t} after the start");
    }
    // A delta captured on T_letho does not touch them: on T_keira they would stay and restart the quest.
    let from_tl = ops(&tl, &li, &letho_closure_scopes());
    assert!(triggers.iter().all(|t| from_tl.iter().all(|o| o.path() != t)));

    let mut work = tk.clone();
    let keira = ops(&tk, &km, &keira_scopes());
    graph::transplant(&mut work, &km, &keira).unwrap();
    let letho = ops(&tk, &li, &letho_closure_scopes());
    assert!(!graph::names_prefix(&work, &li), "T_keira and the Letho donor are separate re-saves");
    graph::transplant_with(&mut work, &li, &letho, NameMode::Remap).unwrap();

    for t in &triggers {
        assert!(graph::locate(&work, t).unwrap().is_none(), "{t} left waiting");
    }
    for (donor, list) in [(&km, &keira), (&li, &letho)] {
        for o in list.iter().filter(|o| matches!(o, GraftOp::Insert { .. } | GraftOp::Replace { .. })) {
            assert_eq!(resolved(&work, o.path()), resolved(donor, o.path()), "{}", o.path());
        }
    }
    // Nothing outside the two deltas moved: the rest of the graph is T_keira's.
    let mut scopes = keira_scopes();
    scopes.extend(letho_closure_scopes());
    let inside = |p: &str| scopes.iter().any(|s| p == s || p.starts_with(&format!("{s}/")));
    let stray: Vec<String> = BLOCKS
        .iter()
        .flat_map(|n| graph::diff_top_with(&tk, &work, n, BY_ID).unwrap().unwrap_or_default())
        .map(|c| c.path().to_owned())
        .filter(|p| !inside(p) && !p.ends_with("/numBlocks") && !p.ends_with("/numThreads"))
        .collect();
    assert!(stray.is_empty(), "changes outside the deltas: {stray:?}");
    eprintln!("Keira ({} ops, prefix) + Letho closure ({} ops, remap) on T_keira: verified", keira.len(), letho.len());
}
