//! Consequence check of the auto-complete walk: a quest may be marked done automatically only when it has no
//! player decision with consequences.
//!
//! **The rule** (`check`, run by `crate::complete::complete` on every finished walk unless the quest is a decision
//! quest completed with the player's own answers):
//! 1. A *decision* is a block the walk passed inside the quest's region that plays a dialogue (`CQuestSceneBlock`,
//!    `CQuestInteractionDialogBlock`) with two or more connected output sockets, with no forced pick. (Conditions,
//!    random blocks and pauses are the game's own tests, not the player's choices.)
//! 2. The *branch* of a socket = everything reachable forward from it inside the region (connections, phase
//!    links, facts that wake a reader), plus the facts the scene itself sets on the way to that output. A fact
//!    or a journal outcome *differs* when some branches have it and others do not (what every branch shares, such
//!    as a common tail, cancels out). Journal outcomes are the root or objective entries a branch sets to Success
//!    or Failed.
//! 3. The quest is **refused** when
//!    * a differing fact is *read outside the quest* (see [`FactReaders`]): by a pause or condition block of a graph
//!      file that is not one of the quest's own files, by a flow condition of a scene the quest does not play, by a
//!      spawn set the quest does not switch (a name mention), or by a game script (`FactsQuerySum("id")`, ...); or
//!    * a decision's branches end with different journal outcomes that matter: an entry that is Failed in one
//!      branch and not in another, or any difference in the root entry (Success against Failed); or
//!    * a dialogue's own in-scene choice lines (inside the `.w2scene`, which the walker does not pick) set a fact
//!      that is read outside the quest (`Walk::maybe_facts`, these are not written by the walk at all).
//! 4. Everything else stays allowed: cosmetic dialogue, routes that converge, differences only the quest itself
//!    reads. A decision that passes the rule but whose branches still differ in facts or journal entries is kept in
//!    [`Report::kept`] (the generated TOML's `auto_picks`, so the UI can show what the walk chose).
//!
//! Not considered (documented gaps): rewards, spawn-set differences between branches, facts read by `.journal`
//! files, facts a script builds by string concatenation, other conditions than facts (inventory, time).
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::complete::{self, Completion};
use crate::quest::{guid_hex, normalize, BlockKind, Cond, Guid, Library, PathStep, SpawnAction};
use crate::walk::Event;

/// Who reads which fact across the install (lower-case fact ids), built once per library.
#[derive(Debug, Default)]
pub struct FactReaders {
    /// Quest-graph pause and condition blocks (demo files aside).
    graph: HashMap<String, Vec<usize>>,
    /// Flow conditions of dialogue scenes: fact -> scene files.
    scenes: HashMap<String, BTreeSet<String>>,
    /// `Facts*("id")` calls of the game's scripts: fact -> script files.
    scripts: HashMap<String, BTreeSet<String>>,
    /// Spawn sets: identifier-like tokens of the file -> file names (a mention, not a proven read).
    comms: HashMap<String, BTreeSet<String>>,
}

/// What belongs to the quest itself: its graph files, the scenes and spawn sets it plays.
#[derive(Debug, Default)]
struct Own {
    files: HashSet<String>,
    scenes: HashSet<String>,
    comms: HashSet<String>,
}

fn leaves(c: &Cond, out: &mut Vec<String>) {
    match c {
        Cond::Fact { id, .. } => out.push(id.to_ascii_lowercase()),
        Cond::Logic { inner, .. } => inner.iter().for_each(|i| leaves(i, out)),
        _ => {}
    }
}

/// The library's [`FactReaders`], built on first use (scenes, spawn sets and scripts are read from the install).
pub fn readers(lib: &Library) -> Arc<FactReaders> {
    if let Some(r) = lib.fact_readers.get() {
        return r.clone();
    }
    let r = Arc::new(FactReaders::build(lib));
    lib.fact_readers.get_or_init(|| r).clone()
}

impl FactReaders {
    fn build(lib: &Library) -> FactReaders {
        let mut r = FactReaders::default();
        for (i, b) in lib.blocks.iter().enumerate() {
            if crate::quest::is_demo_file(&b.file) {
                continue;
            }
            let mut ids = Vec::new();
            match &b.kind {
                BlockKind::Pause { conds } => conds.iter().for_each(|c| leaves(c, &mut ids)),
                BlockKind::Condition { cond: Some(c) } => leaves(c, &mut ids),
                _ => {}
            }
            for id in ids {
                let v = r.graph.entry(id).or_default();
                if v.last() != Some(&i) {
                    v.push(i);
                }
            }
        }
        for name in lib.resource_names(".w2scene") {
            let Ok(data) = lib.read_resource(&name) else { continue };
            let Ok(scene) = crate::scene::parse(&data, &name) else { continue };
            let mut ids = Vec::new();
            for n in &scene.nodes {
                if let Some(c) = &n.cond {
                    leaves(c, &mut ids);
                }
            }
            for id in ids {
                r.scenes.entry(id).or_default().insert(name.clone());
            }
        }
        for name in lib.resource_names(".w2comm") {
            let Ok(data) = lib.read_resource(&name) else { continue };
            let mut tok = String::new();
            let flush = |tok: &mut String, r: &mut FactReaders| {
                if tok.len() >= 6 && tok.contains('_') {
                    r.comms.entry(std::mem::take(tok)).or_default().insert(name.clone());
                }
                tok.clear();
            };
            for &b in &data {
                let c = b.to_ascii_lowercase();
                if c.is_ascii_alphanumeric() || c == b'_' {
                    tok.push(c as char);
                } else {
                    flush(&mut tok, &mut r);
                }
            }
            flush(&mut tok, &mut r);
        }
        if let Some(install) = &lib.install {
            let mut dirs = vec![install.join("content"), install.join("dlc")];
            while let Some(d) = dirs.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        if p.file_name().is_some_and(|n| n != "bundles") {
                            dirs.push(p);
                        }
                    } else if p.extension().is_some_and(|x| x == "ws") {
                        if let Ok(bytes) = std::fs::read(&p) {
                            let name = normalize(&p.strip_prefix(install).unwrap_or(&p).to_string_lossy());
                            scan_script(&String::from_utf8_lossy(&bytes), &name, &mut r.scripts);
                        }
                    }
                }
            }
        }
        r
    }

    /// Where `fact` is read outside the quest (`own`): at most a few names per source, empty when nowhere.
    fn outside(&self, lib: &Library, fact: &str, own: &Own) -> Vec<String> {
        self.outside_n(lib, fact, own, 3)
    }

    fn outside_n(&self, lib: &Library, fact: &str, own: &Own, n: usize) -> Vec<String> {
        let id = fact.to_ascii_lowercase();
        let mut out: Vec<String> = Vec::new();
        let mut files: BTreeSet<String> = BTreeSet::new();
        for b in self.graph.get(&id).into_iter().flatten() {
            let f = &lib.blocks[*b].file;
            if !own.files.contains(f) {
                files.insert(f.clone());
            }
        }
        out.extend(files.into_iter().take(n).map(|f| format!("graph {}", complete::file_name(&f))));
        out.extend(self.scenes.get(&id).into_iter().flatten().filter(|s| !own.scenes.contains(*s)).take(n).map(|s| format!("scene {}", complete::file_name(s))));
        out.extend(self.scripts.get(&id).into_iter().flatten().take(n).map(|s| format!("script {s}")));
        out.extend(self.comms.get(&id).into_iter().flatten().filter(|s| !own.comms.contains(*s)).take(n).map(|s| format!("spawn set {}", complete::file_name(s))));
        out
    }
}

/// Every place of the install that reads `fact` (research: `w3al fact-readers`), `n` names per source at most.
pub fn readers_of(lib: &Library, fact: &str, n: usize) -> Vec<String> {
    readers(lib).outside_n(lib, fact, &Own::default(), n)
}

/// Record the first string literal argument of every `Facts...(` call of a script.
fn scan_script(text: &str, file: &str, out: &mut HashMap<String, BTreeSet<String>>) {
    let bytes = text.as_bytes();
    for (at, _) in text.match_indices("Facts") {
        let mut i = at + 5;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        // Reads only (`FactsQuerySum`, `FactsQuery`, `FactsDoesExist`, `FactsQueryLatestValue`), not `FactsAdd` / `FactsRemove`.
        if !(text[at + 5..i].starts_with("Query") || text[at + 5..i].starts_with("DoesExist")) {
            continue;
        }
        let skip_ws = |mut j: usize| {
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            j
        };
        i = skip_ws(i);
        if bytes.get(i) != Some(&b'(') {
            continue;
        }
        i = skip_ws(i + 1);
        if bytes.get(i) != Some(&b'"') {
            continue;
        }
        let Some(len) = text[i + 1..].find('"') else { continue };
        let id = &text[i + 1..i + 1 + len];
        if !id.is_empty() && id.len() < 128 && !id.contains('\n') {
            out.entry(id.to_ascii_lowercase()).or_default().insert(file.to_owned());
        }
    }
}

impl Own {
    fn of(lib: &Library, region: &HashSet<usize>) -> Own {
        let mut own = Own::default();
        for &b in region {
            let blk = &lib.blocks[b];
            own.files.insert(blk.file.clone());
            match &blk.kind {
                BlockKind::Scene { file } | BlockKind::Interaction { file, .. } if !file.is_empty() => {
                    own.scenes.insert(normalize(file));
                }
                BlockKind::StoryPhase { actions } => {
                    for a in actions {
                        match a {
                            SpawnAction::Activate { spawnset, .. } | SpawnAction::Deactivate { spawnset } => {
                                own.comms.insert(normalize(spawnset));
                            }
                            SpawnAction::Other(_) => {}
                        }
                    }
                }
                _ => {}
            }
        }
        own
    }
}

/// What one branch of a decision does: facts set, and journal entries set to Success or Failed
/// (`path`, status, whether the entry is the quest's root).
#[derive(Debug, Default, Clone)]
struct Effects {
    facts: BTreeSet<String>,
    journal: BTreeSet<(String, &'static str, bool)>,
}

fn path_key(path: &[PathStep]) -> String {
    path.iter().map(|s| guid_hex(&s.guid)[..8].to_owned()).collect::<Vec<_>>().join("/")
}

fn scene_file(lib: &Library, b: usize) -> Option<&str> {
    match &lib.blocks[b].kind {
        BlockKind::Scene { file } | BlockKind::Interaction { file, .. } if !file.is_empty() && lib.has_resources(".w2scene") => Some(file),
        _ => None,
    }
}

/// Every fact a scene sets anywhere.
fn all_scene_facts(lib: &Library, file: &str, out: &mut BTreeSet<String>) {
    if let Ok(s) = lib.scene(file) {
        out.extend(s.nodes.iter().flat_map(|n| n.facts.iter()).map(|f| f.id().to_ascii_lowercase()));
    }
}

/// The branch of `socket` of the decision `b`.
fn branch(lib: &Library, nodes: &HashSet<usize>, fr: &HashMap<String, Vec<usize>>, b: usize, socket: &str) -> Effects {
    let mut e = Effects::default();
    if let Some(file) = scene_file(lib, b) {
        let input = lib.input_sockets(b).into_iter().next().unwrap_or_default();
        match lib.scene(file).map_err(|x| x.to_string()).and_then(|s| s.outcome(&input, socket, &[], &|_| None).map_err(|x| x.to_string())) {
            Ok(o) => {
                e.facts.extend(o.certain.iter().map(|f| f.id().to_ascii_lowercase()));
                e.facts.extend(o.maybe.iter().map(|(f, _)| f.id().to_ascii_lowercase()));
            }
            Err(_) => all_scene_facts(lib, file, &mut e.facts),
        }
    }
    let mut seen: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<(usize, String)> = lib.blocks[b].outs.iter().filter(|c| c.socket == socket).map(|c| (c.target, c.input.clone())).collect();
    while let Some((t, input)) = queue.pop_front() {
        if !nodes.contains(&t) {
            continue;
        }
        if let BlockKind::Journal { path, .. } = &lib.blocks[t].kind {
            let status = match input.as_str() {
                "Success" => Some("Success"),
                "Fail" | "Failed" | "Failure" => Some("Failed"),
                _ => None,
            };
            if let Some(s) = status {
                e.journal.insert((path_key(path), s, path.len() == 2));
            }
        }
        if !seen.insert(t) {
            continue;
        }
        match &lib.blocks[t].kind {
            BlockKind::Fact { id, .. } => {
                e.facts.insert(id.to_ascii_lowercase());
            }
            BlockKind::Scene { file } | BlockKind::Interaction { file, .. } if !file.is_empty() && lib.has_resources(".w2scene") => all_scene_facts(lib, file, &mut e.facts),
            _ => {}
        }
        queue.extend(complete::successors(lib, nodes, fr, t));
    }
    e
}

/// What [`check`] found.
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// Why the quest is refused (empty = allowed).
    pub blocking: Vec<String>,
    /// Decisions the walk passed whose branches differ but only inside the quest: `(block, socket taken)`.
    pub kept: Vec<(usize, String)>,
}

/// Apply the rule of the module doc to a finished completion. `forced` are the player's own picks (those blocks
/// are not decisions the walk made).
pub fn check(lib: &Library, c: &Completion, forced: &HashMap<Guid, String>) -> Report {
    let mut rep = Report::default();
    let nodes = &c.region;
    let mut took: HashMap<usize, String> = HashMap::new();
    for e in &c.walk.events {
        if let Event::Assumed { block, socket, .. } = e {
            took.insert(*block, socket.clone());
        }
    }
    let mut cands: Vec<usize> = took
        .keys()
        .copied()
        .filter(|b| nodes.contains(b) && matches!(lib.blocks[*b].kind, BlockKind::Scene { .. } | BlockKind::Interaction { .. }))
        .filter(|b| lib.blocks[*b].sockets().len() >= 2 && !forced.contains_key(&lib.blocks[*b].guid))
        .collect();
    cands.sort_unstable();
    let own = Own::of(lib, nodes);
    let fr = complete::fact_readers(lib, nodes);
    let rd = readers(lib);
    let read_by = |f: &str| rd.outside(lib, f, &own);
    for b in cands {
        let sockets: Vec<String> = lib.blocks[b].sockets().into_iter().map(str::to_owned).collect();
        // Only the ways of *completing* the quest are options: an output that fails or abandons the quest, or never
        // reaches its Success, is not a choice for a quest the player marks completed.
        let branches: Vec<Effects> = sockets
            .iter()
            .map(|s| branch(lib, nodes, &fr, b, s))
            .filter(|e| e.journal.iter().any(|(_, s, root)| *root && *s == "Success") && !e.journal.iter().any(|(_, s, root)| *root && *s == "Failed"))
            .collect();
        if branches.len() < 2 {
            continue;
        }
        let union_f: BTreeSet<&String> = branches.iter().flat_map(|e| e.facts.iter()).collect();
        let diff_f: Vec<&String> = union_f.into_iter().filter(|f| !branches.iter().all(|e| e.facts.contains(*f))).collect();
        let union_j: BTreeSet<&(String, &'static str, bool)> = branches.iter().flat_map(|e| e.journal.iter()).collect();
        let diff_j: Vec<&(String, &'static str, bool)> = union_j.into_iter().filter(|j| !branches.iter().all(|e| e.journal.contains(*j))).collect();
        let label = lib.describe(b);
        let mut blocked = false;
        for f in &diff_f {
            let r = read_by(f);
            if !r.is_empty() {
                blocked = true;
                rep.blocking.push(format!("{label}: its outputs {sockets:?} differ in fact {f}, read outside the quest by {}", r.join(", ")));
            }
        }
        if let Some((p, _, _)) = diff_j.iter().find(|(_, s, _)| *s == "Failed") {
            blocked = true;
            rep.blocking.push(format!("{label}: its outputs {sockets:?} end with different journal outcomes (Failed {p} on some, not on others)"));
        }
        if !blocked && (!diff_f.is_empty() || !diff_j.is_empty()) {
            if let Some(s) = took.get(&b) {
                rep.kept.push((b, s.clone()));
            }
        }
    }
    // In-scene choices the walker does not pick: the facts of their lines are not written at all.
    let mut seen_maybe: HashSet<&str> = HashSet::new();
    for e in &c.walk.events {
        let Event::Maybe { block, id, via } = e else { continue };
        if !nodes.contains(block) || !seen_maybe.insert(id.as_str()) {
            continue;
        }
        let r = read_by(id);
        if !r.is_empty() {
            rep.blocking.push(format!("{}: a choice inside its dialogue ({via}) sets fact {id}, read outside the quest by {}", lib.describe(*block), r.join(", ")));
        }
    }
    rep.kept.sort_unstable();
    rep
}

/// The `block GUID=socket` form of a kept decision (the form `--pick` takes).
pub fn pick_key(lib: &Library, block: usize, socket: &str) -> String {
    format!("{}={socket}", guid_hex(&lib.blocks[block].guid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quest::{Block, Conn, PathStep};
    use crate::walk::Walk;

    fn block(i: u8, file: &str, kind: BlockKind, outs: Vec<(&str, usize, &str)>) -> Block {
        let mut guid = [0u8; 16];
        guid[0] = i;
        Block {
            guid,
            class: "CQuestBlock".into(),
            file: file.into(),
            export: i as usize,
            label: String::new(),
            outs: outs.into_iter().map(|(s, t, i)| Conn { socket: s.into(), target: t, input: i.into() }).collect(),
            owner: None,
            graph: 0,
            kind,
        }
    }

    fn fact(id: &str) -> BlockKind {
        BlockKind::Fact { id: id.into(), value: 1 }
    }

    fn done(lib: &Library, took: &[(usize, &str)]) -> Completion {
        let walk = Walk {
            events: took.iter().map(|(b, s)| Event::Assumed { block: *b, socket: (*s).into(), why: "auto" }).collect(),
            ..Default::default()
        };
        Completion { walk, instances: vec![], starts: vec![], picks: vec![], region: (0..lib.blocks.len()).filter(|i| lib.blocks[*i].file == "quest.w2phase").collect(), notes: vec![], decisions: vec![] }
    }

    /// 0 dialogue: A -> 1 fact `a_only` -> 3 root Success; B -> 2 fact `b_only` -> 3 root Success. File `other.w2phase` has a pause on `a_only`.
    fn lib(reader: Option<&str>) -> Library {
        let root = BlockKind::Journal { quest: true, path: vec![PathStep { guid: [1; 16], resource: String::new() }, PathStep { guid: [2; 16], resource: String::new() }] };
        let mut blocks = vec![
            block(0, "quest.w2phase", BlockKind::Scene { file: String::new() }, vec![("A", 1, "In"), ("B", 2, "In")]),
            block(1, "quest.w2phase", fact("a_only"), vec![("Out", 3, "Success")]),
            block(2, "quest.w2phase", fact("b_only"), vec![("Out", 3, "Success")]),
            block(3, "quest.w2phase", root, vec![]),
        ];
        if let Some(r) = reader {
            blocks.push(block(4, "other.w2phase", BlockKind::Pause { conds: vec![Cond::Fact { id: r.into(), cmp: "CF_GreaterEqual".into(), value: 1 }] }, vec![]));
        }
        let mut l = Library::default();
        l.blocks = blocks;
        l.reindex();
        l
    }

    #[test]
    fn a_decision_whose_fact_is_read_by_another_graph_is_refused() {
        let l = lib(Some("a_only"));
        let r = check(&l, &done(&l, &[(0, "A")]), &HashMap::new());
        assert_eq!(r.blocking.len(), 1, "{r:?}");
        assert!(r.blocking[0].contains("a_only") && r.blocking[0].contains("other.w2phase"), "{r:?}");
    }

    #[test]
    fn a_decision_the_quest_alone_reads_is_kept_and_a_converging_one_is_trivial() {
        let l = lib(Some("unrelated"));
        let r = check(&l, &done(&l, &[(0, "B")]), &HashMap::new());
        assert!(r.blocking.is_empty(), "{r:?}");
        assert_eq!(r.kept, vec![(0, "B".to_owned())]);
        // Both outputs lead to the same fact: nothing differs.
        let mut l = lib(None);
        l.blocks[0].outs = vec![Conn { socket: "A".into(), target: 1, input: "In".into() }, Conn { socket: "B".into(), target: 1, input: "In".into() }];
        let r = check(&l, &done(&l, &[(0, "A")]), &HashMap::new());
        assert!(r.blocking.is_empty() && r.kept.is_empty(), "{r:?}");
        // A forced pick is the player's answer, not a decision of the walk.
        let l = lib(Some("a_only"));
        let forced: HashMap<Guid, String> = [(l.blocks[0].guid, "A".to_owned())].into();
        assert!(check(&l, &done(&l, &[(0, "A")]), &forced).blocking.is_empty());
    }

    #[test]
    fn an_objective_failed_on_one_branch_is_a_journal_difference_but_failing_the_quest_is_not_a_choice() {
        let mut l = lib(None);
        // B also fails an objective (path of three steps) while the root still succeeds.
        let obj = BlockKind::Journal { quest: false, path: vec![PathStep { guid: [1; 16], resource: String::new() }, PathStep { guid: [2; 16], resource: String::new() }, PathStep { guid: [3; 16], resource: String::new() }] };
        l.blocks.push(block(4, "quest.w2phase", obj, vec![]));
        l.blocks[2].outs = vec![Conn { socket: "Out".into(), target: 3, input: "Success".into() }, Conn { socket: "Out".into(), target: 4, input: "Failure".into() }];
        l.reindex();
        let r = check(&l, &done(&l, &[(0, "A")]), &HashMap::new());
        assert!(r.blocking.iter().any(|m| m.contains("journal outcomes")), "{r:?}");
        // B instead fails the whole quest: only A completes it, so there is nothing to decide.
        let mut l = lib(None);
        l.blocks[2].outs = vec![Conn { socket: "Out".into(), target: 3, input: "Failure".into() }];
        let r = check(&l, &done(&l, &[(0, "A")]), &HashMap::new());
        assert!(r.blocking.is_empty() && r.kept.is_empty(), "{r:?}");
    }

    #[test]
    fn scripts_name_the_facts_they_read() {
        let mut m = HashMap::new();
        scan_script("if ( FactsQuerySum ( \"sq104_niellen_dead\" ) > 0 ) {}\nFactsAdd(\"x_add\");\nFactsDoesExist(name);", "game/x.ws", &mut m);
        assert!(m.contains_key("sq104_niellen_dead") && m.len() == 1, "reads only: {m:?}");
    }
}
