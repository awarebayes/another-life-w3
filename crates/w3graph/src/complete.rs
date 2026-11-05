//! Auto-complete walk: finish one quest from the state a template save holds it in (not discovered,
//! or active) to its Success, with no human and no donor, so that the save has it done for real: the
//! journal root at Success and the quest's graph ended (gate G: a quest whose graph still runs is
//! reopened by the game).
//!
//! **The quest.** A `CJournalQuest` of the install (`crate::catalog`). Its *home* is the graph file that
//! holds it (the journal's `questPhase`, else the topmost file holding the quest's journal blocks,
//! world hubs such as `bugfixing_phase` aside); its *instances* are the phase blocks that run that file
//! in the template (`questThread[GUID]` in the save); its *region* is every block under the instances
//! (embedded graphs and imported files, recursively). A quest embedded in a phase of a shared file is
//! that phase.
//!
//! **The policy** (deterministic: the same template and graphs give the same walk, and every pick is
//! listed in [`Completion::picks`]):
//! 1. Starts = the template's waits inside the instances from which the root's Success is reachable,
//!    the furthest first (the earliest stage of the quest first, as play goes); each start is walked to
//!    its end before the next (`Walker::run`), a start inside a phase that already ended does not fire.
//! 2. Inside the quest the walk is `through`: a wait counts as done when one of its outputs leads to
//!    the goal; with several outputs, the one nearest the goal is taken (a *pick*); a wait with no output
//!    towards the goal stays waiting (failure listeners, despawns after the player leaves: they end with
//!    the quest's phase). An instant block that must choose (a condition the walker cannot evaluate,
//!    a random block) takes the output nearest the goal, else its first output (a pick too). Fact
//!    conditions are evaluated as always.
//! 3. The goal is the root entry's Success (an edge into a root journal block with input `Success`);
//!    once it fired, the goal becomes the end of the quest's phase (an output of the instance that is not
//!    a failure). Edges into the root's Failure are never taken towards either goal.
//! 4. Outside the quest the walk is not `through`: what the quest's end activates in the hub runs to its
//!    next wait, and the template's own fact listeners the walk's facts make true fire (gate J2), as in
//!    any synthesized walk. Nothing else of another quest is walked.
//! 5. Refused, with the reason ([`Refusal`]): the quest is already done or failed in the template; its
//!    region holds a block of a decision-catalogue file (those stay in the decision questbook); its phase
//!    is not running in the template (not reachable there); no Success in its graph or no route to it;
//!    an unresolved point; the walk ends without Success or with the quest's phase still running; the walk passes
//!    a player decision with consequences (`crate::consequence`: branches that differ in facts read outside the
//!    quest, or in journal outcomes), unless `Options::check_consequences` is off.
//!
//! HYPOTHESES (checked offline against saves where a player completed the quest, `w3al side-check`; not
//! yet in game): that assuming every wait on the way is what play does, that the nearest-to-Success output
//! is a legal play-through, and that the scopes this leaves are what the game keeps.
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use w3save::journal;
use w3save::Document;

use crate::catalog::JournalQuest;
use crate::frontier::{self, Frontier};
use crate::quest::{guid_hex, normalize, BlockKind, Guid, Library};
use crate::walk::{AutoGoal, AutoPick, Event, Policy, Scores, Walk, Walker};

/// The graph files where the decisions of `docs/plan/decision-catalogue.md` (sections 4, 5 and 7) are taken
/// (file names, lower case). A quest whose region holds one of them is refused: its completion is a
/// decision the player makes in the decision questbook.
pub const DECISION_FILES: &[&str] = &[
    "q105_evil_spirit.w2phase",
    "q109_wrapup.w2phase",
    "sq101_keira.w2phase",
    "sq102_letho.w2phase",
    "q302_mafia.w2phase",
    "q202_giant.w2phase",
    "q206_berserkers_attack.w2phase",
    "sq301_dawn_estate.w2phase",
    "q309_casablanca.w2phase",
    "sq202_yen.w2phase",
    "q402_ciri.w2phase",
    "q110_kaer_morhen.w2phase",
    "q110_vizima.w2phase",
    "q310_pregeels_new.w2phase",
    "mq3035_meet&bait.w2phase",
    "mq3035_wrap_up.w2phase",
    "mq3035_novigrad_roche.w2phase",
    "q210_precanaris.w2phase",
    "q103_daughter.w2phase",
    "q107_swamps.w2phase",
    "sq312_kill.w2phase",
    "q403_first_square.w2phase",
];

/// The file name part of a graph path, lower case (`quests/part_1/x.w2phase` -> `x.w2phase`).
pub fn file_name(path: &str) -> String {
    let n = normalize(path);
    n.rsplit('/').next().unwrap_or(&n).to_owned()
}

/// Graph files that hold journal blocks of many quests and are never a quest's home (world hubs).
pub fn is_shared_file(f: &str) -> bool {
    ["bugfixing", "persistent_content", "patch_signal", "traveling_merchants", "/demo_files/", "character_journal", "witcher3_structure", "living_world"].iter().any(|s| f.contains(s))
}

/// Why a quest is not completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Refusal {
    /// The journal root is already Success in the template (nothing to do).
    AlreadyDone,
    /// The journal root is Failed in the template.
    Failed,
    /// The quest's region holds a decision-catalogue file.
    Decision,
    /// The quest's graph is not in the install's quest graphs (no journal block, no phase file).
    NoGraph,
    /// The quest's phase is not running in the template.
    NotAvailable,
    /// The quest's phase runs twice in the template.
    SeveralInstances,
    /// No route from the template's waits to the root's Success.
    NoSuccess,
    /// The walk has an unresolved point.
    Unresolved,
    /// The walk reached Success but the quest's phase still runs afterwards.
    NotFinished,
    /// The walk could not be written into the save.
    Write,
    /// The walk passes a player decision whose branches differ in something the game reads elsewhere
    /// (`crate::consequence`).
    Consequence,
}

impl Refusal {
    pub fn label(self) -> &'static str {
        match self {
            Refusal::AlreadyDone => "already done",
            Refusal::Failed => "failed in the template",
            Refusal::Decision => "decision quest (catalogue)",
            Refusal::NoGraph => "no quest graph",
            Refusal::NotAvailable => "not running in the template",
            Refusal::SeveralInstances => "several instances",
            Refusal::NoSuccess => "no route to Success",
            Refusal::Unresolved => "unresolved point",
            Refusal::NotFinished => "phase still running after Success",
            Refusal::Write => "write failed",
            Refusal::Consequence => "has a decision with consequences",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Refused {
    pub kind: Refusal,
    pub detail: String,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.label(), self.detail)
    }
}

impl std::error::Error for Refused {}

fn refused(kind: Refusal, detail: impl Into<String>) -> Refused {
    Refused { kind, detail: detail.into() }
}

/// The library's [`QuestIndex`], built on first use and kept with the library.
pub fn index(lib: &Library) -> Result<std::sync::Arc<QuestIndex>, crate::quest::Error> {
    if let Some(i) = lib.quest_index.get() {
        return Ok(i.clone());
    }
    let i = std::sync::Arc::new(QuestIndex::new(lib)?);
    Ok(lib.quest_index.get_or_init(|| i).clone())
}

/// The install's quests with their journal blocks, built once and shared by many walks.
#[derive(Debug)]
pub struct QuestIndex {
    pub quests: Vec<JournalQuest>,
    /// Quest GUID -> its journal blocks.
    pub blocks: HashMap<Guid, Vec<usize>>,
    /// File -> the quests whose root entry (depth 2) a block of that file writes.
    pub roots_in_file: HashMap<String, HashSet<Guid>>,
}

impl QuestIndex {
    pub fn new(lib: &Library) -> Result<QuestIndex, crate::quest::Error> {
        let quests = crate::catalog::journal_quests(lib)?;
        let blocks = crate::catalog::journal_blocks_by_quest(lib);
        let mut roots_in_file: HashMap<String, HashSet<Guid>> = HashMap::new();
        for (q, bs) in &blocks {
            for b in bs {
                if let BlockKind::Journal { path, .. } = &lib.blocks[*b].kind {
                    if path.len() == 2 {
                        roots_in_file.entry(lib.blocks[*b].file.clone()).or_default().insert(*q);
                    }
                }
            }
        }
        Ok(QuestIndex { quests, blocks, roots_in_file })
    }

    /// The quest a key names: `GROUP/QUEST` or `QUEST` (32 hex digits each), or the first 8 hex digits of the quest GUID.
    pub fn find(&self, key: &str) -> Option<&JournalQuest> {
        let k = key.trim().to_ascii_lowercase();
        let quest = k.rsplit('/').next().unwrap_or(&k);
        let group = k.split_once('/').map(|x| x.0);
        let hits: Vec<&JournalQuest> = self
            .quests
            .iter()
            .filter(|q| {
                let g = guid_hex(&q.guid);
                (g == quest || (quest.len() == 8 && g.starts_with(quest))) && group.is_none_or(|gr| guid_hex(&q.group) == gr)
            })
            .collect();
        match hits[..] {
            [one] => Some(one),
            _ => None,
        }
    }
}

/// Where a quest lives in the graphs.
#[derive(Debug, Clone)]
pub enum Home {
    /// Its own file, run by phase blocks that import it.
    File(String),
    /// An embedded phase of a shared file.
    Phase(usize),
}

/// The quest's home (see the module doc).
pub fn home(lib: &Library, idx: &QuestIndex, q: &JournalQuest) -> Option<Home> {
    if let Some(p) = &q.phase_file {
        if !lib.root_blocks(p).is_empty() {
            return Some(Home::File(p.clone()));
        }
    }
    let jb = idx.blocks.get(&q.guid)?;
    let mut count: HashMap<&str, usize> = HashMap::new();
    for b in jb {
        let f = lib.blocks[*b].file.as_str();
        if !is_shared_file(f) {
            *count.entry(f).or_default() += 1;
        }
    }
    let files: HashSet<&str> = count.keys().copied().collect();
    // Topmost: no importer of the file sits in another candidate file.
    let mut tops: Vec<(&str, usize)> =
        count.iter().filter(|(f, _)| !lib.importers_of(f).iter().any(|i| files.contains(lib.blocks[*i].file.as_str()) && lib.blocks[*i].file != **f)).map(|(f, n)| (*f, *n)).collect();
    tops.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let top = tops.first()?.0;
    // A file that writes other quests' roots too: the quest is the deepest embedded phase holding all of its blocks there.
    let others = idx.roots_in_file.get(top).is_some_and(|s| s.iter().any(|g| *g != q.guid));
    if !others {
        return Some(Home::File(top.to_owned()));
    }
    let chains: Vec<Vec<usize>> = jb
        .iter()
        .filter(|b| lib.blocks[**b].file == top)
        .map(|b| {
            let mut v = Vec::new();
            let mut at = lib.blocks[*b].owner;
            while let Some(o) = at {
                v.push(o);
                at = lib.blocks[o].owner;
            }
            v.reverse();
            v
        })
        .collect();
    let first = chains.first()?;
    let mut lca = None;
    for (d, p) in first.iter().enumerate() {
        if chains.iter().all(|c| c.get(d) == Some(p)) {
            lca = Some(*p);
        } else {
            break;
        }
    }
    lca.map(Home::Phase)
}

/// Every block under these phase blocks (embedded graphs and imported files, recursively).
pub fn region(lib: &Library, instances: &[usize]) -> HashSet<usize> {
    let mut out: HashSet<usize> = HashSet::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut todo: Vec<usize> = instances.to_vec();
    while let Some(p) = todo.pop() {
        if !seen.insert(p) {
            continue;
        }
        for &i in lib.inner_blocks(p) {
            if out.insert(i) && matches!(lib.blocks[i].kind, BlockKind::Phase { .. }) {
                todo.push(i);
            }
        }
    }
    out
}

fn is_root(path: &[crate::quest::PathStep], root: &[Guid]) -> bool {
    path.len() == root.len() && path.iter().zip(root).all(|(p, g)| p.guid == *g)
}

fn cond_facts(c: &crate::quest::Cond, out: &mut Vec<String>) {
    match c {
        crate::quest::Cond::Fact { id, cmp, value } if !(cmp == "CF_Equal" && *value == 0) && !(cmp == "CF_Less" && *value <= 1) => out.push(id.clone()),
        crate::quest::Cond::Logic { inner, .. } => inner.iter().for_each(|i| cond_facts(i, out)),
        _ => {}
    }
}

/// Fact id -> the blocks of the region that wait on it or test it (pause and condition blocks): a fact block
/// leads to them (the quest's parallel branches meet through facts, not connections: an investigation sets
/// `mh206_lair_found`, the combat branch waits for it).
pub(crate) fn fact_readers(lib: &Library, nodes: &HashSet<usize>) -> HashMap<String, Vec<usize>> {
    let mut m: HashMap<String, Vec<usize>> = HashMap::new();
    let mut ordered: Vec<usize> = nodes.iter().copied().collect();
    ordered.sort_unstable();
    for b in ordered {
        let mut ids = Vec::new();
        match &lib.blocks[b].kind {
            BlockKind::Pause { conds } => conds.iter().for_each(|c| cond_facts(c, &mut ids)),
            BlockKind::Condition { cond: Some(c) } => cond_facts(c, &mut ids),
            _ => {}
        }
        for id in ids {
            m.entry(id).or_default().push(b);
        }
    }
    m
}

/// Successor edges of a block inside the region: `(target, input)`.
pub(crate) fn successors(lib: &Library, nodes: &HashSet<usize>, readers: &HashMap<String, Vec<usize>>, b: usize) -> Vec<(usize, String)> {
    let blk = &lib.blocks[b];
    let mut v: Vec<(usize, String)> = blk.outs.iter().map(|c| (c.target, c.input.clone())).collect();
    if let BlockKind::Fact { id, value } = &blk.kind {
        if *value > 0 {
            v.extend(readers.get(id).into_iter().flatten().filter(|r| **r != b).map(|r| (*r, "\u{1}fact".to_owned())));
        }
    }
    match &blk.kind {
        BlockKind::Phase { .. } => {
            for &i in lib.inner_blocks(b) {
                if matches!(lib.blocks[i].kind, BlockKind::PhaseInput { .. }) {
                    v.push((i, String::new()));
                }
            }
        }
        BlockKind::PhaseOutput { socket } => {
            let owners: Vec<usize> = match blk.owner {
                Some(o) => vec![o],
                None => lib.importers_of(&blk.file).iter().copied().filter(|i| nodes.contains(i)).collect(),
            };
            for o in owners {
                v.extend(lib.blocks[o].outs.iter().filter(|c| &c.socket == socket).map(|c| (c.target, c.input.clone())));
            }
        }
        _ => {}
    }
    v
}

/// Breadth-first distances to `terminal` over the region (reverse edges), never through a `wall`.
fn distances(lib: &Library, nodes: &HashSet<usize>, terminal: HashSet<(usize, String)>, wall: HashSet<(usize, String)>) -> Scores {
    let mut s = Scores { dist: HashMap::new(), terminal, wall };
    let readers = fact_readers(lib, nodes);
    let mut preds: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    let mut ordered: Vec<usize> = nodes.iter().copied().collect();
    ordered.sort_unstable();
    for &b in &ordered {
        for (t, input) in successors(lib, nodes, &readers, b) {
            match s.via(t, &input) {
                Some(0) if !s.dist.contains_key(&b) => {
                    s.dist.insert(b, 1);
                    queue.push_back(b);
                }
                None if Scores::is_blocked(&s.wall, t, &input) => {}
                _ => {
                    if nodes.contains(&t) {
                        preds.entry(t).or_default().push(b);
                    }
                }
            }
        }
    }
    while let Some(t) = queue.pop_front() {
        let d = s.dist[&t];
        for &p in preds.get(&t).map_or(&[][..], Vec::as_slice) {
            if let std::collections::hash_map::Entry::Vacant(v) = s.dist.entry(p) {
                v.insert(d + 1);
                queue.push_back(p);
            }
        }
    }
    s
}

impl Scores {
    fn is_blocked(wall: &HashSet<(usize, String)>, t: usize, input: &str) -> bool {
        wall.contains(&(t, input.to_owned())) || wall.contains(&(t, String::new()))
    }
}

/// The two goals over a region: the root's Success, then the end of the instances' graphs.
pub fn goals(lib: &Library, nodes: &HashSet<usize>, instances: &[usize], root: &[Guid]) -> (Scores, Scores) {
    goals_avoiding(lib, nodes, instances, root, &HashSet::new())
}

/// [`goals`] with `avoid` blocks as walls (forced stops: the policy routes around them).
pub fn goals_avoiding(lib: &Library, nodes: &HashSet<usize>, instances: &[usize], root: &[Guid], avoid: &HashSet<usize>) -> (Scores, Scores) {
    let mut success_t = HashSet::new();
    let mut fail: HashSet<(usize, String)> = avoid.iter().map(|b| (*b, String::new())).collect();
    for &b in nodes {
        if let BlockKind::Journal { quest: true, path } = &lib.blocks[b].kind {
            if is_root(path, root) {
                success_t.insert((b, "Success".to_owned()));
                for f in ["Failure", "Fail", "Failed"] {
                    fail.insert((b, f.to_owned()));
                }
            }
        }
    }
    let mut exit_t = HashSet::new();
    let mut exit_wall = fail.clone();
    for &p in instances {
        for &i in lib.inner_blocks(p) {
            if let BlockKind::PhaseOutput { socket } = &lib.blocks[i].kind {
                if socket.to_ascii_lowercase().contains("fail") {
                    exit_wall.insert((i, String::new()));
                } else {
                    exit_t.insert((i, String::new()));
                }
            }
        }
    }
    (distances(lib, nodes, success_t, fail), distances(lib, nodes, exit_t, exit_wall))
}

/// Options of a completion walk.
#[derive(Debug, Clone)]
pub struct Options {
    /// Graph file names (lower case, as [`file_name`] gives them) that refuse a quest whose region holds one.
    pub deny_files: HashSet<String>,
    pub max_steps: usize,
    /// Forced outputs, `block GUID -> socket`: the player's answer at a decision inside (or on the way to) the
    /// quest. They win over the policy's own pick and over the gate's way; each one the walk took is listed in
    /// [`Completion::picks`]. Used for a decision quest completed late on a later template
    /// (`[effects.*.synth] complete` with `pick`), with `deny_files` emptied for it.
    pub picks: HashMap<Guid, String>,
    /// Forced choice lines inside scenes, `scene block GUID -> section names` (as `--scene-pick`).
    pub scene_picks: HashMap<Guid, Vec<String>>,
    /// Waits that stay waiting (the player did not take that way: "far from Triss" = left her during the fight);
    /// the policy routes around them. They end with their phase like any parked wait.
    pub stops: HashSet<Guid>,
    /// Refuse a quest whose walk passes a player decision with consequences (`crate::consequence`). On by default;
    /// off for a decision quest completed with the player's own answers.
    pub check_consequences: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            deny_files: DECISION_FILES.iter().map(|s| (*s).to_owned()).collect(),
            max_steps: 20_000,
            picks: HashMap::new(),
            scene_picks: HashMap::new(),
            stops: HashSet::new(),
            check_consequences: true,
        }
    }
}

impl Options {
    /// Add `GUID=socket` picks, `GUID=section` scene picks and `GUID` stops (8 or 32 hex digits, as `synth-walk`
    /// takes them), resolved against the graphs and the save's frontier; each socket must exist.
    pub fn with_picks(mut self, lib: &Library, tf: &Frontier, picks: &[String], scene_picks: &[String], stops: &[String]) -> Result<Options, String> {
        for s in stops {
            let i = crate::synth::resolve_block(lib, tf, s).map_err(|e| format!("stop {s:?}: {e}"))?;
            self.stops.insert(lib.blocks[i].guid);
        }
        for p in picks {
            let (k, s) = p.split_once('=').ok_or_else(|| format!("pick {p:?}: want GUID=socket"))?;
            let i = crate::synth::resolve_block(lib, tf, k).map_err(|e| format!("pick {p:?}: {e}"))?;
            if !lib.blocks[i].sockets().contains(&s) {
                return Err(format!("pick: block {} has outputs {:?}, not {s:?}", lib.describe(i), lib.blocks[i].sockets()));
            }
            self.picks.insert(lib.blocks[i].guid, s.to_owned());
        }
        for p in scene_picks {
            let (k, s) = p.split_once('=').ok_or_else(|| format!("scene pick {p:?}: want GUID=section"))?;
            let i = crate::synth::resolve_block(lib, tf, k).map_err(|e| format!("scene pick {p:?}: {e}"))?;
            if !matches!(lib.blocks[i].kind, BlockKind::Scene { .. } | BlockKind::Interaction { .. }) {
                return Err(format!("scene pick: block {} plays no scene", lib.describe(i)));
            }
            self.scene_picks.entry(lib.blocks[i].guid).or_default().push(s.to_owned());
        }
        Ok(self)
    }
}

/// A finished quest, ready for `crate::synth::apply`.
#[derive(Debug)]
pub struct Completion {
    pub walk: Walk,
    /// The quest's phase blocks running in the template.
    pub instances: Vec<usize>,
    /// The template's waits the walk fired, in order, with their saved input.
    pub starts: Vec<(usize, String)>,
    /// Every output chosen by the policy.
    pub picks: Vec<AutoPick>,
    /// The blocks of the quest (under its instances).
    pub region: HashSet<usize>,
    pub notes: Vec<String>,
    /// Decisions the walk passed whose branches differ in facts or journal entries that nothing outside the quest
    /// reads (`crate::consequence`), as `(block, socket the walk took)`. Kept in the generated TOML (`auto_picks`).
    pub decisions: Vec<(usize, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quest::{Block, Cond, Conn, PathStep};
    use crate::walk::Walker;

    const G: Guid = [0xaa; 16];
    const Q: Guid = [0xbb; 16];

    fn block(i: u8, class: &str, kind: BlockKind, outs: Vec<(&str, usize, &str)>, owner: Option<usize>) -> Block {
        let mut guid = [0u8; 16];
        guid[0] = i;
        Block {
            guid,
            class: class.into(),
            file: "t.w2phase".into(),
            export: i as usize,
            label: String::new(),
            outs: outs.into_iter().map(|(s, t, i)| Conn { socket: s.into(), target: t, input: i.into() }).collect(),
            owner,
            graph: if owner.is_some() { 9 } else { 0 },
            kind,
        }
    }

    fn root_journal() -> BlockKind {
        BlockKind::Journal { quest: true, path: vec![PathStep { guid: G, resource: String::new() }, PathStep { guid: Q, resource: String::new() }] }
    }

    fn lib(blocks: Vec<Block>) -> Library {
        let mut l = Library::default();
        l.blocks = blocks;
        l.reindex();
        l
    }

    fn walk(l: &Library, starts: &[(usize, String)]) -> Walk {
        let mut nodes = region(l, &[0]);
        nodes.insert(0);
        let (success, exit) = goals(l, &nodes, &[0], &[G, Q]);
        let policy = Policy {
            through: true,
            running: [l.blocks[0].guid].into(),
            auto: Some(AutoGoal { instances: [l.blocks[0].guid].into(), root: vec![G, Q], success, exit }),
            ..Default::default()
        };
        let base = |_: &str| 0;
        let mut w = Walker::new(l, &base, policy);
        w.run(starts);
        w.into_walk()
    }

    #[test]
    fn the_policy_takes_the_way_to_success_parks_failure_listeners_and_records_picks() {
        // 0 quest phase (embedded 9) -> 9.Out -> 7 fact `after` (hub).
        // inside: 2 scene: Fight -> 3 root Failure -> 6 output Failed; Talk -> 4 condition (unknown): True -> 5 root
        // Success -> 10 random: R1 -> 11 fact `r1` -> 9 output Out, R2 -> 3 Failure. 8 pause (unknown) -> 3 Failure.
        let l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![("Out", 7, "In"), ("Failed", 12, "In")], None),
            block(1, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "In".into() }, vec![("", 2, "In")], Some(0)),
            block(2, "CQuestSceneBlock", BlockKind::Scene { file: String::new() }, vec![("Fight", 3, "Failure"), ("Talk", 4, "In")], Some(0)),
            block(3, "CJournalQuestBlock", root_journal(), vec![("Out", 6, "In")], Some(0)),
            block(4, "CQuestConditionBlock", BlockKind::Condition { cond: Some(Cond::Other("CQuestActorCondition".into())) }, vec![("False", 3, "Failure"), ("True", 5, "Success")], Some(0)),
            block(5, "CJournalQuestBlock", root_journal(), vec![("Out", 10, "In")], Some(0)),
            block(6, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Failed".into() }, vec![], Some(0)),
            block(7, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "after".into(), value: 1 }, vec![], None),
            block(8, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("CQuestInsideTriggerCondition".into())] }, vec![("", 3, "Failure")], Some(0)),
            block(9, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(0)),
            block(10, "CQuestRandomBlock", BlockKind::Instant, vec![("R2", 3, "Failure"), ("R1", 11, "In")], Some(0)),
            block(11, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "r1".into(), value: 1 }, vec![("Out", 9, "In")], Some(0)),
            block(12, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "failed".into(), value: 1 }, vec![], None),
        ]);
        let w = walk(&l, &[(8, "In".into()), (2, "In".into())]);
        assert!(w.auto_success, "{:?}", w.events);
        let picks: Vec<(usize, &str)> = w.auto_picks.iter().map(|p| (p.block, p.socket.as_str())).collect();
        assert_eq!(picks, vec![(2, "Talk"), (4, "True"), (10, "R1")], "{:?}", w.auto_picks);
        assert!(w.auto_picks[0].why.starts_with("success in"), "{:?}", w.auto_picks);
        assert!(w.auto_picks[2].why.starts_with("end in"), "after Success the goal is the phase's end: {:?}", w.auto_picks);
        assert_eq!(w.auto_parked, vec![8], "the failure listener stays waiting");
        assert_eq!(w.exited, vec![0]);
        assert_eq!(w.facts.get("r1"), Some(&1));
        assert_eq!(w.facts.get("after"), Some(&1), "the hub runs on after the quest's end");
        assert_eq!(w.facts.get("failed"), None);
        assert!(!w.events.iter().any(|e| matches!(e, Event::Journal { status, .. } if status == "JS_Failed")));
        // Deterministic: the same walk again gives the same picks.
        assert_eq!(walk(&l, &[(8, "In".into()), (2, "In".into())]).auto_picks, w.auto_picks);
    }

    #[test]
    fn forced_picks_and_stops_override_the_nearest_way() {
        // 0 quest phase: 1 input -> 2 scene: Short -> 3 pause (unknown) -> 6 fact short -> 7 root Success;
        // Long -> 4 pause (unknown) -> 8 fact long -> 9 fact long2 -> 7. 7 -> 10 output Out.
        let l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![], None),
            block(1, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "In".into() }, vec![("", 2, "In")], Some(0)),
            block(2, "CQuestSceneBlock", BlockKind::Scene { file: String::new() }, vec![("Short", 3, "In"), ("Long", 4, "In")], Some(0)),
            block(3, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("x".into())] }, vec![("", 6, "In")], Some(0)),
            block(4, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("x".into())] }, vec![("", 8, "In")], Some(0)),
            block(5, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "unused".into(), value: 1 }, vec![], Some(0)),
            block(6, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "short".into(), value: 1 }, vec![("Out", 7, "Success")], Some(0)),
            block(7, "CJournalQuestBlock", root_journal(), vec![("Out", 10, "In")], Some(0)),
            block(8, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "long".into(), value: 1 }, vec![("Out", 11, "In")], Some(0)),
            block(9, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Unused".into() }, vec![], Some(0)),
            block(10, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(0)),
            block(11, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "long2".into(), value: 1 }, vec![("Out", 7, "Success")], Some(0)),
        ]);
        let run = |picks: &[(usize, &str)], stops: &[usize]| {
            let mut nodes = region(&l, &[0]);
            nodes.insert(0);
            let avoid: HashSet<usize> = stops.iter().copied().collect();
            let (success, exit) = goals_avoiding(&l, &nodes, &[0], &[G, Q], &avoid);
            let policy = Policy {
                through: true,
                running: [l.blocks[0].guid].into(),
                picks: picks.iter().map(|(b, s)| (l.blocks[*b].guid, (*s).to_owned())).collect(),
                stop: stops.iter().map(|b| l.blocks[*b].guid).collect(),
                auto: Some(AutoGoal { instances: [l.blocks[0].guid].into(), root: vec![G, Q], success, exit }),
                ..Default::default()
            };
            let base = |_: &str| 0;
            let mut w = Walker::new(&l, &base, policy);
            w.run(&[(2, "In".into())]);
            w.into_walk()
        };
        let free = run(&[], &[]);
        assert!(free.auto_success && free.facts.contains_key("short") && !free.facts.contains_key("long"), "nearest way: {:?}", free.facts);
        let forced = run(&[(2, "Long")], &[]);
        assert!(forced.auto_success && forced.facts.contains_key("long2") && !forced.facts.contains_key("short"), "forced pick: {:?}", forced.facts);
        assert!(forced.auto_picks.is_empty(), "a forced pick is not the policy's: {:?}", forced.auto_picks);
        let stopped = run(&[], &[3]);
        assert!(stopped.auto_success && stopped.facts.contains_key("long") && !stopped.facts.contains_key("short"), "stop routes around: {:?}", stopped.facts);
        assert_eq!(stopped.auto_picks.iter().map(|p| (p.block, p.socket.as_str())).collect::<Vec<_>>(), vec![(2, "Long")]);
    }

    #[test]
    fn parallel_branches_meet_through_facts_and_all_run_before_the_phase_ends() {
        // 0 quest phase: 1 pause (unknown, "investigate") -> 2 fact lair_found; 3 pause [lair_found >= 1] -> 4 root Success
        // -> 5 output Out. 0.Out -> 6 fact after. Starts in the "wrong" order: the fight first.
        let l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![("Out", 6, "In")], None),
            block(1, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("CQuestInsideTriggerCondition".into())] }, vec![("", 2, "In")], Some(0)),
            block(2, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "lair_found".into(), value: 1 }, vec![], Some(0)),
            block(3, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Fact { id: "lair_found".into(), cmp: "CF_GreaterEqual".into(), value: 1 }] }, vec![("", 4, "Success")], Some(0)),
            block(4, "CJournalQuestBlock", root_journal(), vec![("Out", 5, "In")], Some(0)),
            block(5, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(0)),
            block(6, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "after".into(), value: 1 }, vec![], None),
        ]);
        let mut nodes = region(&l, &[0]);
        nodes.insert(0);
        let (success, _) = goals(&l, &nodes, &[0], &[G, Q]);
        assert_eq!(success.dist.get(&1), Some(&3), "the investigation reaches Success through its fact: {:?}", success.dist);
        let w = walk(&l, &[(3, "In".into()), (1, "In".into())]);
        assert!(w.auto_success);
        assert_eq!(w.facts.get("lair_found"), Some(&1), "the other branch was played before the phase ended: {:?}", w.events);
        assert_eq!(w.facts.get("after"), Some(&1));
        assert_eq!(w.exited, vec![0]);
    }

    #[test]
    fn outside_the_quest_through_does_not_apply() {
        // The quest's end activates a hub wait (7, unknown condition, one output to a fact): it waits.
        let l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![("Out", 4, "In")], None),
            block(1, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("x".into())] }, vec![("", 2, "Success")], Some(0)),
            block(2, "CJournalQuestBlock", root_journal(), vec![("Out", 3, "In")], Some(0)),
            block(3, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(0)),
            block(4, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("x".into())] }, vec![("", 5, "In")], None),
            block(5, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "next_quest".into(), value: 1 }, vec![], None),
        ]);
        let w = walk(&l, &[(1, "In".into())]);
        assert!(w.auto_success);
        assert_eq!(w.facts.get("next_quest"), None);
        assert_eq!(w.waits.iter().map(|x| x.block).collect::<Vec<_>>(), vec![4]);
        assert!(!w.events.iter().any(|e| matches!(e, Event::Unresolved { .. })));
    }

    #[test]
    fn suspended_scope_arrays_reencode_and_drop_an_entry() {
        use w3save::token::{Kind, Node, Part, Raw};
        let raw = |b: Vec<u8>| Node { index: 0, kind: Kind::Raw(Raw { tag: "?".into(), why: "test", parts: vec![Part::Bytes(b)] }) };
        let g1 = [1u8; 16];
        let g2 = [2u8; 16];
        let parts = vec![
            Part::Bytes(vec![2, 0, 0, 0, 0, 0x7d, 0x07, 3, 3]),
            Part::Node(raw(g1.to_vec())),
            Part::Bytes(vec![0x7e, 0x07, 0x2e, 0x4b]),
            Part::Node(raw(vec![9, 9, 9])),
            Part::Bytes(vec![0, 0, 0, 0x7d, 0x07, 3, 3]),
            Part::Node(raw(g2.to_vec())),
            Part::Bytes(vec![0x7e, 0x07, 0x2e, 0x4b]),
            Part::Node(raw(vec![8])),
            Part::Bytes(vec![0, 0]),
        ];
        let mut s = frontier::suspended_entries(&parts).expect("parses");
        assert_eq!(s.entries.iter().map(|e| e.guid).collect::<Vec<_>>(), vec![g1, g2]);
        assert_eq!(s.to_parts(), parts, "re-encodes byte for byte");
        s.entries.retain(|e| e.guid != g1);
        let one = s.to_parts();
        assert_eq!(one[0], Part::Bytes(vec![1, 0, 0, 0, 0, 0x7d, 0x07, 3, 3]));
        assert_eq!(frontier::suspended_entries(&one).unwrap().entries.len(), 1);
        s.entries.clear();
        assert_eq!(s.to_parts(), vec![Part::Bytes(vec![0, 0, 0, 0])]);
    }
}

/// A completed quest compared with a save where a player completed it (validation).
#[derive(Debug, Default, Clone)]
pub struct QuestCheck {
    /// Root status in the synthesized save and in the played one.
    pub root: (Option<String>, Option<String>),
    /// Journal entries of the quest that are new or changed against the template: hit / miss / extra.
    pub journal: (usize, usize, usize),
    /// The quest's facts (set by its graph or its scenes) that changed: hit / miss / extra.
    pub facts: (usize, usize, usize),
    /// The quest's phase is gone from `questSystem`: in the synthesized save, in the played one.
    pub scope_ended: (bool, bool),
    /// Spawn sets the walk switched, and set-level differences from the played save in them.
    pub community: (usize, usize),
    pub lines: Vec<String>,
}

impl QuestCheck {
    /// Root Success in both, the phase ended in both, no journal or fact miss, no set-level community difference.
    pub fn hit(&self) -> bool {
        self.root.0.as_deref() == Some("JS_Success")
            && self.root.0 == self.root.1
            && self.scope_ended.0 == self.scope_ended.1
            && self.journal.1 == 0
            && self.facts.1 == 0
            && self.community.1 == 0
    }
}

/// Compare a completion written into `synth` (from `template`) with `played`, a later save of the same
/// line where the quest is done.
pub fn check(lib: &Library, q: &JournalQuest, c: &Completion, template: &Document, synth: &Document, played: &Document) -> Result<QuestCheck, crate::quest::Error> {
    let mut out = QuestCheck::default();
    let root = q.root_key();
    let (tj, sj, pj) = (journal::read(template)?, journal::read(synth)?, journal::read(played)?);
    let status = |j: &journal::Journal, k: &[Guid]| j.find(k).first().map(|i| j.entries[*i].status.clone());
    out.root = (status(&sj, &root), status(&pj, &root));
    let changed = |j: &journal::Journal| -> HashMap<Vec<Guid>, String> {
        j.entries
            .iter()
            .filter(|e| e.key().starts_with(&root))
            .filter(|e| status(&tj, &e.key()).as_deref() != Some(e.status.as_str()))
            .map(|e| (e.key(), e.status.clone()))
            .collect()
    };
    let (sc, pc) = (changed(&sj), changed(&pj));
    let label = |k: &[Guid]| k.iter().map(|g| guid_hex(g)[..8].to_owned()).collect::<Vec<_>>().join("/");
    for (k, s) in &pc {
        match sc.get(k) {
            Some(x) if x == s => out.journal.0 += 1,
            other => {
                out.journal.1 += 1;
                out.lines.push(format!("journal MISS {s} {} (synth {:?})", label(k), other.cloned().or_else(|| status(&sj, k))));
            }
        }
    }
    for (k, s) in &sc {
        if !pc.contains_key(k) {
            out.journal.2 += 1;
            out.lines.push(format!("journal EXTRA {s} {} (played {:?})", label(k), status(&pj, k)));
        }
    }
    // Facts of the quest: ids its region's fact blocks set, plus what the walk set (scene facts).
    let mut ids: HashSet<String> = c.region.iter().filter_map(|b| match &lib.blocks[*b].kind {
        BlockKind::Fact { id, .. } => Some(id.clone()),
        _ => None,
    }).collect();
    ids.extend(c.walk.facts.keys().cloned());
    let (tfacts, sfacts, pfacts) = (frontier::fact_sums(template)?, frontier::fact_sums(synth)?, frontier::fact_sums(played)?);
    for id in ids.iter().filter(|i| !frontier::is_noise_fact(i)) {
        let (t, s, p) = (tfacts.get(id), sfacts.get(id), pfacts.get(id));
        let synth_changed = s != t;
        let played_changed = p != t;
        match (synth_changed, played_changed) {
            (true, true) => out.facts.0 += 1,
            (false, true) => {
                out.facts.1 += 1;
                out.lines.push(format!("fact MISS {id} (template {t:?}, played {p:?})"));
            }
            (true, false) => {
                out.facts.2 += 1;
                out.lines.push(format!("fact EXTRA {id} (template {t:?}, synth {s:?})"));
            }
            (false, false) => {}
        }
    }
    let gone = |d: &Document| -> Result<bool, crate::quest::Error> {
        let f = frontier::read(d)?;
        Ok(!c.instances.iter().any(|i| f.scopes.iter().chain(&f.suspended).any(|s| s.0 == lib.blocks[*i].guid)))
    };
    out.scope_ended = (gone(synth)?, gone(played)?);
    if out.scope_ended.0 != out.scope_ended.1 {
        out.lines.push(format!("phase ended: synth {} played {}", out.scope_ended.0, out.scope_ended.1));
    }
    let sets = crate::community::touched_sets(&crate::synth::spawn_actions(&c.walk));
    let diffs: Vec<String> = if sets.is_empty() { Vec::new() } else { crate::community::compare(synth, played, &sets) }
        .into_iter()
        .filter(|l| l.starts_with("set ") || l.starts_with("phase "))
        .collect();
    out.community = (sets.len(), diffs.len());
    out.lines.extend(diffs.into_iter().map(|d| format!("community {d}")));
    Ok(out)
}

/// The template's status of the quest's root entry (`JS_Success`, ...), `None` when the log does not have it.
pub fn root_status(template: &Document, q: &JournalQuest) -> Result<Option<String>, crate::quest::Error> {
    let j = journal::read(template)?;
    Ok(j.find(&q.root_key()).first().map(|i| j.entries[*i].status.clone()))
}

/// The quest's instances in the template: home importers (or the embedded phase) running there.
pub fn instances(lib: &Library, tf: &Frontier, home: &Home) -> Vec<usize> {
    let running = tf.scope_guids();
    let cands: Vec<usize> = match home {
        Home::File(f) => lib.importers_of(f).to_vec(),
        Home::Phase(p) => vec![*p],
    };
    cands.into_iter().filter(|i| running.contains(&lib.blocks[*i].guid)).collect()
}

/// A quest whose phase does not run yet: a wait of the template upstream of one of its importers in the
/// same graph (the hub's gate, e.g. "player reached the area"), with the outputs on the way to the importer.
pub struct Gate {
    pub importer: usize,
    /// The template's waits to fire, with their saved input.
    pub starts: Vec<(usize, String)>,
    /// `block GUID -> socket` along the way from each gate to the importer.
    pub picks: HashMap<Guid, String>,
}

/// Search backwards from each importer (in its own graph, at most `depth` blocks) for blocks waiting in the
/// template. The first importer with gates wins.
pub fn find_gate(lib: &Library, tf: &Frontier, importers: &[usize], depth: usize) -> Option<Gate> {
    for &imp in importers {
        let blk = &lib.blocks[imp];
        let graph: Vec<usize> = lib.blocks.iter().enumerate().filter(|(_, b)| b.file == blk.file && b.graph == blk.graph).map(|(i, _)| i).collect();
        let mut preds: HashMap<usize, Vec<(usize, String)>> = HashMap::new();
        for &b in &graph {
            for c in &lib.blocks[b].outs {
                preds.entry(c.target).or_default().push((b, c.socket.clone()));
            }
        }
        // next[b] = (socket, successor) on a shortest way from b to the importer.
        let mut next: HashMap<usize, (String, usize)> = HashMap::new();
        let mut queue: VecDeque<(usize, usize)> = VecDeque::from([(imp, 0)]);
        let mut gates: Vec<(usize, String)> = Vec::new();
        while let Some((t, d)) = queue.pop_front() {
            if d >= depth {
                continue;
            }
            for (p, socket) in preds.get(&t).cloned().unwrap_or_default() {
                if p == imp || next.contains_key(&p) {
                    continue;
                }
                next.insert(p, (socket, t));
                match tf.find(&lib.blocks[p].guid) {
                    Some(w) if lib.blocks[p].is_wait() => gates.push((p, w.input.clone())),
                    _ => queue.push_back((p, d + 1)),
                }
            }
        }
        if gates.is_empty() {
            continue;
        }
        let mut picks = HashMap::new();
        for (g, _) in &gates {
            let mut at = *g;
            let mut guard = 0;
            while let Some((socket, succ)) = next.get(&at) {
                picks.insert(lib.blocks[at].guid, socket.clone());
                at = *succ;
                guard += 1;
                if at == imp || guard > depth {
                    break;
                }
            }
        }
        return Some(Gate { importer: imp, starts: gates, picks });
    }
    None
}

/// Walk the quest to its Success and end (see the module doc). The walk is ready for `crate::synth::apply`.
pub fn complete(lib: &Library, idx: &QuestIndex, template: &Document, tf: &Frontier, q: &JournalQuest, opts: &Options) -> Result<Completion, Refused> {
    match root_status(template, q).map_err(|e| refused(Refusal::Write, e.to_string()))?.as_deref() {
        Some("JS_Success") => return Err(refused(Refusal::AlreadyDone, "the journal root is Success in the template")),
        Some("JS_Failed") => return Err(refused(Refusal::Failed, "the journal root is Failed in the template")),
        _ => {}
    }
    let home = home(lib, idx, q).ok_or_else(|| refused(Refusal::NoGraph, format!("no graph file writes {}", q.file)))?;
    let all_instances: Vec<usize> = match &home {
        Home::File(f) => lib.importers_of(f).to_vec(),
        Home::Phase(p) => vec![*p],
    };
    // Decision check on every possible region (not only the running one): a decision quest is refused on every template.
    for b in region(lib, &all_instances) {
        let f = file_name(&lib.blocks[b].file);
        if opts.deny_files.contains(&f) {
            return Err(refused(Refusal::Decision, format!("its graph holds {f} (docs/plan/decision-catalogue.md)")));
        }
    }
    let mut inst = instances(lib, tf, &home);
    let mut gate = None;
    // A quest of another world: its phase is suspended in the template (the player is elsewhere). It is walked
    // from its phase entry and its suspended entry is dropped (HYPOTHESIS: what the game keeps of a finished
    // quest of an unloaded world is nothing; in-game gate pending).
    let suspended = if inst.is_empty() { all_instances.iter().copied().find(|i| tf.suspended.iter().any(|(g, _)| *g == lib.blocks[*i].guid)) } else { None };
    if let Some(s) = suspended {
        inst = vec![s];
    }
    if inst.is_empty() {
        // Not started: a hub wait upstream of an importer is the quest's start (fired along the way to it).
        match find_gate(lib, tf, &all_instances, 24) {
            Some(g) => {
                inst = vec![g.importer];
                gate = Some(g);
            }
            None => {
                let what = match &home {
                    Home::File(f) => format!(
                        "{f} is not run by any phase running in the template and no wait of the template leads to one (importers: {})",
                        all_instances.iter().map(|i| lib.describe(*i)).collect::<Vec<_>>().join("; ")
                    ),
                    Home::Phase(p) => format!("phase {} is not running in the template", lib.describe(*p)),
                };
                return Err(refused(Refusal::NotAvailable, what));
            }
        }
    }
    let inst_guids: HashSet<Guid> = inst.iter().map(|i| lib.blocks[*i].guid).collect();
    // The template's waits inside the quest.
    let inside: Vec<(usize, String)> = tf
        .blocks
        .iter()
        .filter(|b| b.scope.iter().any(|s| inst_guids.contains(s)))
        .filter_map(|b| lib.get(&b.guid).map(|i| (i, b.input.clone())))
        .filter(|(i, _)| lib.blocks[*i].is_wait())
        .collect();
    if inst.len() > 1 {
        let with_waits: Vec<usize> = inst.iter().copied().filter(|p| tf.blocks.iter().any(|b| b.scope.contains(&lib.blocks[*p].guid))).collect();
        if with_waits.len() != 1 {
            return Err(refused(Refusal::SeveralInstances, inst.iter().map(|i| lib.describe(*i)).collect::<Vec<_>>().join("; ")));
        }
        inst = with_waits;
    }
    let inst_guids: HashSet<Guid> = inst.iter().map(|i| lib.blocks[*i].guid).collect();
    let mut nodes = region(lib, &inst);
    nodes.extend(inst.iter().copied());
    let root = q.root_key();
    let avoid: HashSet<usize> = opts.stops.iter().filter_map(|g| lib.get(g)).collect();
    let (success, exit) = goals_avoiding(lib, &nodes, &inst, &root, &avoid);
    if success.terminal.is_empty() {
        return Err(refused(Refusal::NoGraph, "no block of the quest's graph sets its root entry to Success"));
    }
    let starts: Vec<(usize, String)> = match &gate {
        None if suspended.is_some() => {
            let s = suspended.unwrap_or_default();
            if !success.dist.contains_key(&s) {
                return Err(refused(Refusal::NoSuccess, "no route from the quest's phase entry to its Success"));
            }
            let input = lib.input_sockets(s).into_iter().next().unwrap_or_else(|| "In".to_owned());
            vec![(s, input)]
        }
        Some(g) => {
            if !success.dist.contains_key(&g.importer) {
                return Err(refused(Refusal::NoSuccess, "no route from the quest's phase entry to its Success"));
            }
            g.starts.clone()
        }
        None => {
            let mut starts: Vec<(usize, String, u32)> = inside
                .into_iter()
                .filter(|(i, _)| inst_guids.iter().any(|g| tf.find(&lib.blocks[*i].guid).is_some_and(|b| b.scope.contains(g))))
                .filter_map(|(i, input)| success.dist.get(&i).map(|d| (i, input, *d)))
                .collect();
            // Furthest from Success first (the earliest stage), frontier order on ties (stable sort).
            starts.sort_by_key(|s| std::cmp::Reverse(s.2));
            if starts.is_empty() {
                return Err(refused(Refusal::NoSuccess, "none of the template's waits inside the quest leads to its Success"));
            }
            starts.into_iter().map(|(i, s, _)| (i, s)).collect()
        }
    };

    let sums = frontier::fact_sums(template).map_err(|e| refused(Refusal::Write, e.to_string()))?;
    let base = |id: &str| sums.get(id).copied().unwrap_or(0);
    let waiting = tf.blocks.iter().map(|b| (b.guid, b.inputs.clone())).collect();
    let mut forced = gate.as_ref().map(|g| g.picks.clone()).unwrap_or_default();
    forced.extend(opts.picks.iter().map(|(g, s)| (*g, s.clone())));
    let policy = Policy {
        through: true,
        max_steps: opts.max_steps,
        running: tf.scope_guids(),
        waiting,
        picks: forced,
        scene_picks: opts.scene_picks.clone(),
        stop: opts.stops.clone(),
        auto: Some(AutoGoal { instances: inst_guids.clone(), root: root.clone(), success, exit }),
    };
    let frontier_blocks: Vec<(usize, String)> = tf.blocks.iter().filter_map(|b| lib.get(&b.guid).map(|i| (i, b.input.clone()))).collect();
    let mut w = Walker::new(lib, &base, policy);
    w.run(&starts);
    w.fire_listeners(&frontier_blocks);
    let mut walk = w.into_walk();

    let unresolved: Vec<String> = walk
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Unresolved { block, why } => Some(format!("{}: {why}", lib.describe(*block))),
            _ => None,
        })
        .collect();
    if !unresolved.is_empty() {
        return Err(refused(Refusal::Unresolved, unresolved.join("; ")));
    }
    if !walk.auto_success {
        let parked: Vec<String> = walk.auto_parked.iter().take(4).map(|b| lib.describe(*b)).collect();
        let last: Vec<String> = walk
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Assumed { block, socket, .. } => Some(format!("{}->{socket:?}", lib.blocks[*block].short())),
                Event::Wait { block, .. } => Some(format!("wait {}", lib.blocks[*block].short())),
                _ => None,
            })
            .collect();
        let tail = &last[last.len().saturating_sub(8)..];
        return Err(refused(
            Refusal::NoSuccess,
            format!(
                "the walk ended without the root's Success (starts: {}; left waiting: {}; last steps: {})",
                starts.iter().map(|(b, _)| lib.blocks[*b].short()).collect::<Vec<_>>().join(" "),
                parked.join("; "),
                tail.join(" ")
            ),
        ));
    }
    let mut notes = Vec::new();
    if suspended.is_some() {
        // The whole suspended scope leaves the save: what the walk left waiting inside it goes with it.
        let inside = |w: &crate::walk::WaitAt| w.scope.iter().any(|s| inst_guids.contains(s));
        let n = walk.waits.iter().filter(|w| inside(w)).count();
        walk.waits.retain(|w| !inside(w));
        let running = |g: &Guid| tf.scope_guids().contains(g);
        walk.entered.retain(|e| !nodes.contains(e) && !lib.scope_chain(*e, &running).iter().any(|s| inst_guids.contains(s)));
        for i in &inst {
            if !walk.exited.contains(i) {
                walk.exited.push(*i);
            }
        }
        if n > 0 {
            notes.push(format!("{n} wait(s) left inside the suspended quest are dropped with it"));
        }
    }
    let ended = inst.iter().all(|i| walk.exited.contains(i));
    if !ended {
        let still: Vec<&crate::walk::WaitAt> = walk.waits.iter().filter(|w| w.scope.iter().any(|s| inst_guids.contains(s))).collect();
        let names = || still.iter().take(4).map(|w| lib.describe(w.block)).collect::<Vec<_>>().join("; ");
        let auto = |s: &Scores| still.iter().any(|w| s.dist.contains_key(&w.block));
        let goal_exit = { let g = goals_avoiding(lib, &nodes, &inst, &root, &avoid).1; auto(&g) };
        if suspended.is_some() || goal_exit {
            return Err(refused(Refusal::NotFinished, format!("waits left inside: {}", names())));
        }
        // No wait left can end the phase (map pins, a merchant's gameplay hook, ...): it keeps running after the
        // Success, as in the game.
        notes.push(format!("the quest's phase keeps running after its Success (no route to its end): {}", names()));
    }
    if let Some(s) = suspended {
        walk.drop_suspended.push(lib.blocks[s].guid);
        // Walked from its entry: facts the save already holds (the stages played before) are not added again.
        let had: Vec<String> = walk.facts.keys().filter(|k| base(k) != 0).cloned().collect();
        for k in had {
            walk.facts.remove(&k);
        }
        notes.push(format!("suspended in the template (another world): walked from its entry, its suspended scope {} dropped", lib.describe(s)));
    }
    let mut picks: Vec<AutoPick> = Vec::new();
    if let Some(g) = &gate {
        let mut gp: Vec<AutoPick> = g
            .picks
            .iter()
            .filter(|(guid, _)| !opts.picks.contains_key(*guid))
            .filter_map(|(guid, s)| lib.get(guid).map(|b| AutoPick { block: b, socket: s.clone(), why: "way to the quest's start".into() }))
            .collect();
        gp.sort_by_key(|p| p.block);
        picks.extend(gp);
    }
    // The forced picks the walk reached (the player's answer).
    let reached: HashSet<usize> = walk
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Assumed { block, .. } | Event::Fired { block, .. } => Some(*block),
            _ => None,
        })
        .collect();
    let mut fp: Vec<AutoPick> =
        opts.picks.iter().filter_map(|(guid, s)| lib.get(guid).filter(|b| reached.contains(b)).map(|b| AutoPick { block: b, socket: s.clone(), why: "forced (the player's answer)".into() })).collect();
    fp.sort_by_key(|p| p.block);
    picks.extend(fp);
    picks.extend(walk.auto_picks.iter().cloned());
    let mut c = Completion { walk, instances: inst, starts, picks, region: nodes, notes, decisions: Vec::new() };
    if opts.check_consequences {
        let report = crate::consequence::check(lib, &c, &opts.picks);
        if !report.blocking.is_empty() {
            return Err(refused(Refusal::Consequence, report.blocking.join("; ")));
        }
        c.decisions = report.kept;
    }
    Ok(c)
}
