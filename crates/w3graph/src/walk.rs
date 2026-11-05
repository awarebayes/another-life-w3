//! The decision walker: from a set of activations (block + input socket) it follows the graph
//! through every instant block (facts, journal, rewards, scripts, spawn-set phases, layers,
//! checkpoints, phase inputs / outputs, condition blocks it can evaluate) and stops at blocks that
//! wait for the world (scenes, interactions, pause conditions, scripted actions, ...). What it
//! passes is what the game writes when the same thing happens in play; where it stops is the new
//! frontier.
//!
//! Three knobs make a walk a branch:
//! * `picks`: the output socket to take at a decision scene (or any multi-output wait);
//! * `through`: a wait with one output is assumed satisfied (the player did it): the walker goes
//!   on. Without it the walk stops at the first wait, which is what fact edits alone would do;
//! * `stop`: waits to park at even in `through` mode (the end of the decision window).
//!
//! Facts are evaluated against the template's table plus what the walk itself set, so a pause
//! block whose fact condition is already true falls through (the game does that on load,
//! gate J2), and `fire_listeners` fires the template's own waiting pause blocks whose fact
//! conditions the walk made true.
//!
//! HYPOTHESES (checked offline against played donors in `frontier::compare`, not in game):
//! that a phase output ends every wait inside the phase (and every activation still queued there,
//! W-1, and every sub-phase entered there, W-3); that `CQuestCutControlBlock` only forwards `Out`;
//! that a multi-condition pause block's output sockets are its conditions in order; that
//! `CQuestScriptBlock` is instant (some script functions are latent); that a `CQuestAndBlock` fires
//! once every connected input has been activated, the template's saved inputs counting (Letho
//! invited donor `e9e84a64`, pack 51 `a73fe883`).
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::quest::{BlockKind, Cond, Guid, Library, PathStep, SpawnAction};
use crate::scene::FactOp;

/// How to resolve what the walker cannot decide by itself.
#[derive(Debug, Default, Clone)]
pub struct Policy {
    /// Output socket to take at a block (scene choice, condition block, random, ...).
    pub picks: HashMap<Guid, String>,
    /// Assume single-output waits satisfied and go on.
    pub through: bool,
    /// Waits to stop at regardless of `through`.
    pub stop: HashSet<Guid>,
    /// Choice lines to take inside a scene (by the name of the section the line leads to), per
    /// scene block: resolves facts that depend on an in-scene choice (`scene::Scene::outcome`).
    pub scene_picks: HashMap<Guid, Vec<String>>,
    /// Scopes (phase-block GUIDs) known to be running in the template: picks the importer of a
    /// file that several phase blocks import.
    pub running: HashSet<Guid>,
    /// Blocks waiting in the template with their saved input names: an `AndBlock` the template
    /// already holds counts those inputs as activated.
    pub waiting: HashMap<Guid, Vec<String>>,
    pub max_steps: usize,
    /// Auto-complete mode (`crate::complete`): inside the quest's scope the walker decides by itself
    /// which output to take (towards the quest's Success, then towards the quest's end) and records every
    /// pick in [`Walk::auto_picks`]; outside it `through` does not apply (the walk only runs to the next wait).
    pub auto: Option<AutoGoal>,
}

/// Distances to a goal over a quest's graph region, for the auto-complete policy. A *node* is a block;
/// an *edge* is an output connection (plus phase entry / phase output links). An edge into a
/// `terminal` target (with that input, or any input when the input is empty) reaches the goal; an edge
/// into a `wall` is never taken towards it (the quest's Failure).
#[derive(Debug, Default, Clone)]
pub struct Scores {
    /// Block -> number of blocks to activate before the goal (1 = one of its outputs reaches it).
    pub dist: HashMap<usize, u32>,
    pub terminal: HashSet<(usize, String)>,
    pub wall: HashSet<(usize, String)>,
}

impl Scores {
    fn hit(set: &HashSet<(usize, String)>, t: usize, input: &str) -> bool {
        set.contains(&(t, input.to_owned())) || set.contains(&(t, String::new()))
    }

    /// Distance to the goal through one connection: 0 = the target is the goal, `None` = never.
    pub fn via(&self, t: usize, input: &str) -> Option<u32> {
        if Self::hit(&self.wall, t, input) {
            None
        } else if Self::hit(&self.terminal, t, input) {
            Some(0)
        } else {
            self.dist.get(&t).copied()
        }
    }

    /// Distance to the goal through an output socket of `b` (the best of its connections).
    pub fn socket(&self, lib: &Library, b: usize, socket: &str) -> Option<u32> {
        lib.blocks[b].outs.iter().filter(|c| c.socket == socket).filter_map(|c| self.via(c.target, &c.input)).min()
    }
}

/// What the auto-complete mode walks towards.
#[derive(Debug, Default, Clone)]
pub struct AutoGoal {
    /// The quest's running phase blocks (its instances in the template): a block whose scope chain holds
    /// one of them is inside the quest.
    pub instances: HashSet<Guid>,
    /// The save's journal key of the quest's root entry (`[group, quest]`).
    pub root: Vec<Guid>,
    /// Towards the root entry's Success.
    pub success: Scores,
    /// After Success: towards the end of the quest's phase (an output that is not a failure).
    pub exit: Scores,
}

/// One decision the auto-complete mode took (reproducible: the same template and graphs give the same picks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoPick {
    pub block: usize,
    pub socket: String,
    /// Why: `success in N`, `end in N`, `first output`.
    pub why: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// An instant block fired through this input socket.
    Fired { block: usize, input: String },
    /// A wait assumed satisfied (`through` / `picks`), leaving through `socket`.
    Assumed { block: usize, socket: String, why: &'static str },
    /// A fact set by the graph: the value added (the block's `value`).
    Fact { block: usize, id: String, value: i64 },
    /// A journal path activated with `status` (`JS_Active` / `JS_Success` / `JS_Failed`).
    Journal { block: usize, quest: bool, path: Vec<PathStep>, status: String },
    Reward { block: usize, name: String },
    Spawn { block: usize, action: SpawnAction },
    Script { block: usize, function: String },
    /// `RemoveFactQuest` / `ResetFactQuest`: the fact is gone from the table.
    FactRemoved { block: usize, id: String },
    /// A phase block's graph started (`questThread[guid]` appears).
    Enter { phase: usize, socket: String },
    /// A phase block's output fired: its scope and every wait inside it end.
    Exit { phase: usize, socket: String },
    /// The walk stopped here: a new waiting block.
    Wait { block: usize, input: String },
    /// Something the walker could not decide (no pick, unknown condition, no importer, ...).
    Unresolved { block: usize, why: String },
    /// A `CQuestCutControlBlock` socket other than `Out`, or a scene's `Cut` input: not followed.
    Skipped { block: usize, input: String },
    /// A fact a scene sets on some of the paths to the output taken (an in-scene choice without a
    /// scene pick): not set; a condition on it is unresolved. `via` = the section that sets it.
    Maybe { block: usize, id: String, via: String },
    /// Something worth knowing that does not stop the walk (a scene that could not be read, ...).
    Note { block: usize, text: String },
}

/// A new waiting block with its scope chain (outermost first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitAt {
    pub block: usize,
    pub input: String,
    pub scope: Vec<Guid>,
    pub n_conds: usize,
    /// `CQuestAndBlock`: the input sockets activated so far (the save's `ActivatedInputs`), in the
    /// block's socket order. Empty for every other class.
    pub and_inputs: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct Walk {
    pub events: Vec<Event>,
    /// Fact id -> value added by the walk (sum of the fact blocks fired).
    pub facts: BTreeMap<String, i64>,
    /// Facts removed by script blocks (`RemoveFactQuest`).
    pub removed_facts: Vec<String>,
    /// Waiting blocks after the walk, cancelled ones removed.
    pub waits: Vec<WaitAt>,
    /// Phase blocks whose graphs ended (scopes to remove, with everything inside).
    pub exited: Vec<usize>,
    /// Phase blocks exited and then entered again (a `Restart` loop): their old waits are gone, the
    /// scope itself stays with a new input name.
    pub reentered: Vec<usize>,
    /// Phase blocks whose graphs started and did not end (scopes to add).
    pub entered: Vec<usize>,
    /// Blocks that fired or were assumed (gone from the frontier if they were waiting there).
    pub consumed: HashSet<usize>,
    /// Facts a scene may have set depending on an unpicked choice: `(fact, scene file: section)`.
    pub maybe_facts: Vec<(String, String)>,
    pub steps: usize,
    /// Auto-complete mode: every output the walker chose by itself, in order.
    pub auto_picks: Vec<AutoPick>,
    /// Auto-complete mode: waits inside the quest that were left waiting (no route to the goal).
    pub auto_parked: Vec<usize>,
    /// Auto-complete mode: the quest's root entry was set to Success.
    pub auto_success: bool,
    /// Suspended scopes (`frontier::Frontier::suspended`) to drop: a quest walked to its end while its
    /// world is not loaded (`crate::complete`).
    pub drop_suspended: Vec<Guid>,
}

/// The template's facts: `sum(id)` (0 when absent).
pub type FactSum<'a> = dyn Fn(&str) -> i64 + 'a;

pub struct Walker<'a> {
    pub lib: &'a Library,
    pub base: &'a FactSum<'a>,
    pub policy: Policy,
    pub walk: Walk,
    visited: HashSet<(usize, String)>,
    exited_set: HashSet<usize>,
    /// Phase -> the input socket it was last entered through (W-6: a re-entry through another
    /// socket walks the phase's content afresh).
    entered_via: HashMap<usize, String>,
    pending_waits: Vec<WaitAt>,
    /// `CQuestAndBlock`: the input sockets activated so far, in activation order.
    and_inputs: HashMap<usize, Vec<String>>,
    /// Evaluate conditions against the template's facts alone (to tell listeners the walk made
    /// true from ones that were already true and did not fire in the template).
    ignore_overlay: bool,
    /// Auto-complete: outputs of the quest's own phase reached while other starts are still to walk
    /// (every branch of the quest is played before the quest's phase ends).
    deferred_exits: Vec<(usize, String)>,
    defer_exits: bool,
}

fn has_fact(c: &Cond) -> bool {
    match c {
        Cond::Fact { .. } => true,
        Cond::Logic { inner, .. } => inner.iter().any(has_fact),
        _ => false,
    }
}

fn compare(cmp: &str, sum: i64, value: i64) -> bool {
    match cmp {
        "CF_Equal" => sum == value,
        "CF_NotEqual" => sum != value,
        "CF_Less" => sum < value,
        "CF_LessEqual" => sum <= value,
        "CF_Greater" => sum > value,
        "CF_GreaterEqual" => sum >= value,
        _ => false,
    }
}

impl<'a> Walker<'a> {
    pub fn new(lib: &'a Library, base: &'a FactSum<'a>, policy: Policy) -> Self {
        Walker {
            lib,
            base,
            policy,
            walk: Walk::default(),
            visited: HashSet::new(),
            exited_set: HashSet::new(),
            entered_via: HashMap::new(),
            pending_waits: Vec::new(),
            and_inputs: HashMap::new(),
            ignore_overlay: false,
            deferred_exits: Vec::new(),
            defer_exits: false,
        }
    }

    /// Forget the visits of every block under `phase` (its graph, the graphs of the phases inside
    /// it, imported files included): the content runs again on the next activation.
    fn forget_visits_under(&mut self, phase: usize) {
        let mut todo = vec![phase];
        let mut seen: HashSet<usize> = HashSet::new();
        let mut blocks: HashSet<usize> = HashSet::new();
        while let Some(p) = todo.pop() {
            if !seen.insert(p) {
                continue;
            }
            for &i in self.lib.inner_blocks(p) {
                blocks.insert(i);
                if matches!(self.lib.blocks[i].kind, BlockKind::Phase { .. }) {
                    todo.push(i);
                }
            }
        }
        self.visited.retain(|(b, _)| !blocks.contains(b));
    }

    pub fn fact(&self, id: &str) -> i64 {
        let added = if self.ignore_overlay { 0 } else { self.walk.facts.get(id).copied().unwrap_or(0) };
        (self.base)(id) + added
    }

    /// Which maybe-facts (unpicked scene choices) a condition depends on, for an unresolved message.
    fn maybe_hint(&self, c: &Cond) -> String {
        fn leaves<'a>(c: &'a Cond, out: &mut Vec<&'a str>) {
            match c {
                Cond::Fact { id, .. } => out.push(id),
                Cond::Logic { inner, .. } => inner.iter().for_each(|i| leaves(i, out)),
                _ => {}
            }
        }
        let mut ids = Vec::new();
        leaves(c, &mut ids);
        let hits: Vec<String> = self.walk.maybe_facts.iter().filter(|(f, _)| ids.contains(&f.as_str())).map(|(f, via)| format!("{f} (set in {via})")).collect();
        if hits.is_empty() {
            String::new()
        } else {
            format!("; depends on a scene choice: {} -- add --scene-pick GUID=section", hits.join(", "))
        }
    }

    /// `Some(true/false)` when every leaf is a fact condition, `None` otherwise.
    pub fn eval(&self, c: &Cond) -> Option<bool> {
        match c {
            Cond::Fact { id, cmp, value } => {
                // A fact an unpicked scene choice may have set: unknown until a scene pick says.
                if !self.walk.facts.contains_key(id) && self.walk.maybe_facts.iter().any(|(f, _)| f == id) {
                    return None;
                }
                Some(compare(cmp, self.fact(id), *value))
            }
            Cond::Logic { op, inner } => {
                let vals: Option<Vec<bool>> = inner.iter().map(|x| self.eval(x)).collect();
                let vals = vals?;
                Some(match op.as_str() {
                    s if s.contains("Or") || s.contains("OR") => vals.iter().any(|&b| b),
                    s if s.contains("Xor") || s.contains("XOR") => vals.iter().filter(|&&b| b).count() == 1,
                    _ => vals.iter().all(|&b| b),
                })
            }
            // HYPOTHESIS (Keira KM donor: the 1 ms timer of the default-dialogue restart had passed): a
            // real-time delay of at most a minute is over by the time the player saves.
            Cond::Delay { ms } => Some(*ms <= 60_000 && !self.ignore_overlay),
            Cond::Other(_) => None,
        }
    }

    /// Scopes running now: the template's, plus the phases this walk entered and has not exited
    /// (W-4: an importer the walk already left must not claim the file's blocks again).
    fn running(&self) -> impl Fn(&Guid) -> bool + '_ {
        move |g| {
            self.policy.running.contains(g)
                || self.walk.entered.iter().any(|e| &self.lib.blocks[*e].guid == g && !self.exited_set.contains(e))
        }
    }

    /// Run from these activations until nothing is left. The starts fire one after the other, each
    /// walked to the end before the next (the player satisfied them in that order): a start whose
    /// phase an earlier start's walk ended is gone with it (spirit donor: `c2549952` leaves "leading
    /// horse" and `c4d54c91` inside never fires).
    pub fn run(&mut self, starts: &[(usize, String)]) {
        // Auto-complete: the quest's phase ends only once every start has been walked (all its parallel
        // branches played); the outputs of the quest's phase reached meanwhile are held back and then followed
        // in the order they were reached (the first one ends the phase, W-1 drops the rest).
        self.defer_exits = self.policy.auto.is_some();
        let mut work: Vec<(usize, String)> = Vec::new();
        let mut all: Vec<(usize, String)> = starts.to_vec();
        let mut k = 0;
        let mut replay_from = usize::MAX;
        'starts: loop {
            if k == all.len() {
                if !self.defer_exits || self.deferred_exits.is_empty() {
                    break;
                }
                self.defer_exits = false;
                replay_from = all.len();
                all.extend(std::mem::take(&mut self.deferred_exits));
                continue;
            }
            let (b, input) = all[k].clone();
            k += 1;
            let gone: Vec<Guid> = self.exited_set.iter().map(|p| self.lib.blocks[*p].guid).collect();
            if self.scope_of(b).iter().any(|s| gone.contains(s)) {
                self.walk.events.push(Event::Note { block: b, text: "start not fired: its phase ended earlier in this walk".into() });
                continue;
            }
            // A held-back output is activated again after its first visit was recorded.
            if k > replay_from {
                self.visited.remove(&(b, input.clone()));
            }
            work.push((b, input));
            while let Some((b, input)) = work.pop() {
                if self.policy.max_steps > 0 && self.walk.steps >= self.policy.max_steps {
                    self.walk.events.push(Event::Unresolved { block: b, why: "step limit reached".into() });
                    break 'starts;
                }
                self.walk.steps += 1;
                self.activate(b, &input, &mut work);
            }
        }
        self.defer_exits = false;
        self.finish();
    }

    /// Fire the template's own waiting pause blocks whose fact conditions are now true (what the
    /// game does on load): `frontier` = (block, saved input name). Repeats until stable.
    pub fn fire_listeners(&mut self, frontier: &[(usize, String)]) {
        // A listener only runs to its next wait: `through` is for the decision walk.
        let through = std::mem::replace(&mut self.policy.through, false);
        let mut left: Vec<(usize, String)> = Vec::new();
        for _ in 0..8 {
            let mut starts = Vec::new();
            for (b, input) in frontier {
                if self.walk.consumed.contains(b) || self.visited.contains(&(*b, input.clone())) {
                    continue;
                }
                if let BlockKind::Pause { conds } = &self.lib.blocks[*b].kind {
                    // Only fact listeners start by themselves; timers and world conditions need the game.
                    if !conds.iter().any(has_fact) {
                        continue;
                    }
                    // Already true in the template and still waiting there: the game did not fire it, so neither do we.
                    self.ignore_overlay = true;
                    let before = self.true_socket(*b, conds).is_some();
                    self.ignore_overlay = false;
                    if !before && self.true_socket(*b, conds).is_some() {
                        starts.push((*b, input.clone()));
                    }
                }
            }
            starts.retain(|s| !left.contains(s));
            if starts.is_empty() {
                break;
            }
            if self.policy.auto.is_some() {
                // Auto-complete: a listener whose run meets a point the walker cannot decide (a scripted
                // condition in a hub) is left waiting; the game fires it on load itself (gate J2).
                for s in starts {
                    let snap = (self.walk.clone(), self.visited.clone(), self.exited_set.clone(), self.entered_via.clone(), self.pending_waits.clone(), self.and_inputs.clone());
                    let before = self.walk.events.iter().filter(|e| matches!(e, Event::Unresolved { .. })).count();
                    let mut work = vec![s.clone()];
                    while let Some((b, input)) = work.pop() {
                        self.walk.steps += 1;
                        self.activate(b, &input, &mut work);
                    }
                    if self.walk.events.iter().filter(|e| matches!(e, Event::Unresolved { .. })).count() > before {
                        (self.walk, self.visited, self.exited_set, self.entered_via, self.pending_waits, self.and_inputs) = snap;
                        self.walk.events.push(Event::Note { block: s.0, text: "listener left waiting for the game: its run meets a point the walker cannot decide".into() });
                        left.push(s);
                    }
                }
                continue;
            }
            let mut work: Vec<(usize, String)> = starts.into_iter().rev().collect();
            while let Some((b, input)) = work.pop() {
                self.walk.steps += 1;
                self.activate(b, &input, &mut work);
            }
        }
        self.policy.through = through;
        self.finish();
    }

    /// For a pause block: the output socket of the first condition that is true. One condition:
    /// the block's single socket. Several: the socket at the condition's index (HYPOTHESIS, see
    /// module doc), checked against the socket order of the block.
    fn true_socket(&self, b: usize, conds: &[Cond]) -> Option<String> {
        let blk = &self.lib.blocks[b];
        let sockets = blk.sockets();
        for (i, c) in conds.iter().enumerate() {
            if self.eval(c) == Some(true) {
                let s = if sockets.len() == conds.len() { sockets.get(i).copied() } else { sockets.first().copied() };
                return Some(s.unwrap_or("").to_owned());
            }
        }
        None
    }

    fn follow(&self, b: usize, socket: &str, work: &mut Vec<(usize, String)>) -> usize {
        let blk = &self.lib.blocks[b];
        let mut n = 0;
        // Push in reverse so the first connection is processed first.
        for c in blk.outs.iter().filter(|c| c.socket == socket).rev() {
            work.push((c.target, c.input.clone()));
            n += 1;
        }
        n
    }

    fn follow_all(&self, b: usize, work: &mut Vec<(usize, String)>) {
        for s in self.lib.blocks[b].sockets() {
            self.follow(b, s, work);
        }
    }

    fn scope_of(&self, b: usize) -> Vec<Guid> {
        let running = self.running();
        self.lib.scope_chain(b, &running)
    }

    fn wait_here(&mut self, b: usize, input: &str) {
        let n_conds = match &self.lib.blocks[b].kind {
            BlockKind::Pause { conds } => conds.len(),
            _ => 0,
        };
        let and_inputs = self.and_inputs.get(&b).cloned().unwrap_or_default();
        self.walk.events.push(Event::Wait { block: b, input: input.to_owned() });
        self.pending_waits.push(WaitAt { block: b, input: input.to_owned(), scope: self.scope_of(b), n_conds, and_inputs });
    }

    /// A wait assumed satisfied: time passes there, so everything already queued (the instant
    /// siblings the same socket activated in the same tick) runs first and what follows the wait
    /// goes to the bottom of the work stack (W-5: Roche's `q402_ask_roche_for_help_out` activates
    /// the despawn chain and a journal block at once; the chain reaches the phase output after a 1 s
    /// pause, and a depth-first walk dropped the journal block at the exit).
    fn assume(&mut self, b: usize, socket: &str, why: &'static str, work: &mut Vec<(usize, String)>) {
        self.walk.events.push(Event::Assumed { block: b, socket: socket.to_owned(), why });
        self.walk.consumed.insert(b);
        // Bottom of the stack, first connection processed first.
        for c in self.lib.blocks[b].outs.iter().filter(|c| c.socket == socket) {
            work.insert(0, (c.target, c.input.clone()));
        }
    }

    fn activate(&mut self, b: usize, input: &str, work: &mut Vec<(usize, String)>) {
        if !self.visited.insert((b, input.to_owned())) {
            return;
        }
        if input == "Cut" || input == "Reset" || input == "Off" {
            // Control inputs that end or reset a block: the block leaves the frontier (spirit donor:
            // `1171cb29`'s Cut removed the Soltis dialogue `b9ea9a8e`); its forward path is not taken.
            self.walk.events.push(Event::Skipped { block: b, input: input.to_owned() });
            self.walk.consumed.insert(b);
            self.pending_waits.retain(|w| w.block != b);
            self.walk.waits.retain(|w| w.block != b);
            return;
        }
        let lib = self.lib;
        let blk = &lib.blocks[b];
        let kind = blk.kind.clone();
        let guid = blk.guid;
        match kind {
            BlockKind::Fact { id, value } => {
                *self.walk.facts.entry(id.clone()).or_insert(0) += value;
                self.walk.events.push(Event::Fact { block: b, id, value });
                self.fired(b, input, work);
            }
            BlockKind::Journal { quest, path } => {
                let status = match input {
                    // The graphs' journal inputs: Activate, Success, Failure (W-2: `Failure` was read as Active once).
                    "Success" => "JS_Success",
                    "Fail" | "Failed" | "Failure" => "JS_Failed",
                    "Inactive" | "Deactivate" => "JS_Inactive",
                    _ => "JS_Active",
                };
                if let Some(a) = &self.policy.auto {
                    if status == "JS_Success" && path.len() == a.root.len() && path.iter().zip(&a.root).all(|(p, g)| p.guid == *g) {
                        self.walk.auto_success = true;
                    }
                }
                self.walk.events.push(Event::Journal { block: b, quest, path, status: status.into() });
                self.fired(b, input, work);
            }
            BlockKind::Reward { name } => {
                self.walk.events.push(Event::Reward { block: b, name });
                self.fired(b, input, work);
            }
            BlockKind::Script { function, params } => {
                let arg = |n: &str| params.iter().find(|p| p.0 == n).map(|p| p.1.clone());
                match function.as_str() {
                    // quest_function.ws: RemoveFactQuest(factId), ResetFactQuest(factID), TryToAddUniqueFact(uniqueFactName).
                    "RemoveFactQuest" | "ResetFactQuest" => {
                        if let Some(id) = arg("factId").or_else(|| arg("factID")) {
                            let had = self.fact(&id);
                            self.walk.facts.insert(id.clone(), -(self.base)(&id));
                            if had != 0 && !self.walk.removed_facts.contains(&id) {
                                self.walk.removed_facts.push(id.clone());
                            }
                            self.walk.events.push(Event::FactRemoved { block: b, id });
                        }
                    }
                    "TryToAddUniqueFact" => {
                        if let Some(id) = arg("uniqueFactName") {
                            if self.fact(&id) == 0 {
                                self.walk.facts.insert(id.clone(), 1);
                                self.walk.events.push(Event::Fact { block: b, id, value: 1 });
                            }
                        }
                    }
                    _ => {}
                }
                self.walk.events.push(Event::Script { block: b, function });
                self.fired(b, input, work);
            }
            BlockKind::StoryPhase { actions } => {
                for a in actions {
                    self.walk.events.push(Event::Spawn { block: b, action: a });
                }
                self.fired(b, input, work);
            }
            BlockKind::JournalOther | BlockKind::Instant => {
                self.walk.events.push(Event::Fired { block: b, input: input.to_owned() });
                self.walk.consumed.insert(b);
                if blk.class == "CQuestCutControlBlock" {
                    // Fires `Out` at once and stays in the frontier listening for its cut signal (Keira KM donor: `d5740aa4` waits with input `In`).
                    for s in blk.sockets() {
                        if s != "Out" {
                            self.walk.events.push(Event::Skipped { block: b, input: format!("output {s}") });
                        }
                    }
                    self.walk.consumed.remove(&b);
                    self.pending_waits.push(WaitAt { block: b, input: input.to_owned(), scope: self.scope_of(b), n_conds: 0, and_inputs: Vec::new() });
                    // The other sockets deliver `Cut` to their targets, which then leave the frontier.
                    self.follow_all(b, work);
                } else if let Some(p) = self.policy.picks.get(&guid).cloned() {
                    self.follow(b, &p, work);
                } else if blk.class == "CQuestRandomBlock" && self.auto_in(b) {
                    let sockets: Vec<String> = blk.sockets().into_iter().map(str::to_owned).collect();
                    if let Some(s) = self.auto_forced(b, &sockets) {
                        self.follow(b, &s, work);
                    }
                } else {
                    self.follow_all(b, work);
                }
            }
            BlockKind::Condition { cond } => {
                self.walk.events.push(Event::Fired { block: b, input: input.to_owned() });
                self.walk.consumed.insert(b);
                let auto = self.auto_in(b);
                let socket = match (cond.as_ref().and_then(|c| self.eval(c)), self.policy.picks.get(&guid)) {
                    (Some(true), _) => "True".to_owned(),
                    (Some(false), _) => "False".to_owned(),
                    (None, Some(p)) => p.clone(),
                    (None, None) if auto => {
                        let mut sockets: Vec<String> = blk.sockets().into_iter().map(str::to_owned).collect();
                        if sockets.is_empty() {
                            sockets = vec!["True".into(), "False".into()];
                        }
                        match self.auto_forced(b, &sockets) {
                            Some(s) => s,
                            None => return,
                        }
                    }
                    (None, None) => {
                        let why = cond.map_or("no condition".to_owned(), |c| format!("condition {c:?} cannot be evaluated; add a pick True/False{}", self.maybe_hint(&c)));
                        self.walk.events.push(Event::Unresolved { block: b, why });
                        return;
                    }
                };
                self.follow(b, &socket, work);
            }
            BlockKind::Phase { .. } => {
                self.walk.events.push(Event::Enter { phase: b, socket: input.to_owned() });
                self.walk.consumed.insert(b);
                if self.exited_set.remove(&b) && !self.walk.reentered.contains(&b) {
                    self.walk.reentered.push(b);
                }
                let want = if input.is_empty() { "In" } else { input };
                let inner: Vec<usize> = self
                    .lib
                    .inner_blocks(b)
                    .iter()
                    .copied()
                    .filter(|i| matches!(&self.lib.blocks[*i].kind, BlockKind::PhaseInput { socket } if socket == want))
                    .collect();
                if inner.is_empty() {
                    self.walk.events.push(Event::Unresolved { block: b, why: format!("phase has no input block for socket {want:?} (embedded graph or imported file missing)") });
                    return;
                }
                // W-4 (q401 `weapons_unmount.w2phase`, imported by `719f4f02` and then by `09cd76c3`):
                // a file walked once through one importer is walked afresh when another importer
                // enters it, in that importer's scope; the dedup of `visited` is per importer.
                // W-6 (q402 Cerys `ed29bf82`: entered `First`, left through `To_q208`, entered again
                // through `Second`): a phase re-entered through another input socket runs its content
                // afresh (the game starts the graph again; the dialogue inside waits once more). The
                // same socket twice (a `Restart` loop) keeps its visited blocks: one lap per socket.
                let first_time = !self.walk.entered.contains(&b);
                let other_socket = self.entered_via.get(&b).is_some_and(|s| s != want);
                if (first_time && matches!(kind, BlockKind::Phase { import: Some(_), .. })) || other_socket {
                    self.forget_visits_under(b);
                }
                self.entered_via.insert(b, want.to_owned());
                if first_time {
                    self.walk.entered.push(b);
                }
                for i in inner.into_iter().rev() {
                    work.push((i, "In".into()));
                }
            }
            BlockKind::PhaseInput { .. } => {
                self.walk.events.push(Event::Fired { block: b, input: input.to_owned() });
                self.follow_all(b, work);
            }
            BlockKind::PhaseOutput { socket } => {
                self.walk.events.push(Event::Fired { block: b, input: input.to_owned() });
                // A file imported by several running phases (`random_timer.w2phase` on pack 35: two timers run in
                // the template): the importer this walk itself entered and has not left is the one being exited.
                let mine: Vec<usize> = if self.lib.blocks[b].owner.is_none() {
                    let imps = self.lib.importers_of(&self.lib.blocks[b].file);
                    let mut v: Vec<usize> = imps.iter().copied().filter(|i| self.walk.entered.contains(i) && !self.exited_set.contains(i)).collect();
                    v.dedup();
                    v
                } else {
                    Vec::new()
                };
                // Auto-complete: a shared helper file run by several phases (`empty_phase.w2phase`) is left through
                // the importer that lies inside the quest.
                let in_quest: Vec<usize> = match &self.policy.auto {
                    Some(a) if mine.len() != 1 && self.lib.blocks[b].owner.is_none() => {
                        let running = self.running();
                        self.lib
                            .importers_of(&self.lib.blocks[b].file)
                            .iter()
                            .copied()
                            .filter(|i| running(&self.lib.blocks[*i].guid))
                            .filter(|i| a.instances.contains(&self.lib.blocks[*i].guid) || self.lib.scope_chain(*i, &running).iter().any(|g| a.instances.contains(g)))
                            .collect()
                    }
                    _ => Vec::new(),
                };
                let owner = if let [one] = mine[..] {
                    Ok(Some(one))
                } else if let [one] = in_quest[..] {
                    Ok(Some(one))
                } else {
                    let running = self.running();
                    self.lib.parent_phase_strict(b, &running)
                };
                let owner = match owner {
                    Ok(o) => o,
                    Err(why) => {
                        self.walk.events.push(Event::Unresolved { block: b, why: format!("phase output not followed: {why}") });
                        return;
                    }
                };
                let Some(owner) = owner else {
                    self.walk.events.push(Event::Unresolved { block: b, why: "phase output at a file root that nothing imports (the quest's end)".into() });
                    return;
                };
                if self.defer_exits && self.policy.auto.as_ref().is_some_and(|a| a.instances.contains(&self.lib.blocks[owner].guid)) {
                    self.walk.events.push(Event::Note { block: b, text: format!("the quest's phase output {socket:?} held back until every start is walked") });
                    self.deferred_exits.push((b, input.to_owned()));
                    return;
                }
                self.walk.events.push(Event::Exit { phase: owner, socket: socket.clone() });
                self.exited_set.insert(owner);
                // Everything waiting inside the phase ends now (a later re-entry starts afresh).
                let g = self.lib.blocks[owner].guid;
                self.pending_waits.retain(|w| !w.scope.contains(&g) && self.lib.blocks[w.block].guid != g);
                self.walk.waits.retain(|w| !w.scope.contains(&g) && self.lib.blocks[w.block].guid != g);
                // W-1: work still queued inside the phase (a second exit, a parallel branch) ends
                // with it; otherwise a `through` walk leaves a phase through every exit (q309 Ambush
                // ran `Out` and then `Fail`, setting `q309_failed`).
                let dropped: Vec<String> = work.iter().filter(|(blk, _)| *blk != owner && self.scope_of(*blk).contains(&g)).map(|(blk, i)| format!("{} <- {i:?}", self.lib.describe(*blk))).collect();
                if !dropped.is_empty() {
                    work.retain(|(blk, _)| *blk != owner && !self.scope_of(*blk).contains(&g));
                    self.walk.events.push(Event::Note { block: owner, text: format!("{} queued activation(s) inside the phase dropped at its exit: {}", dropped.len(), dropped.join("; ")) });
                }
                if !self.walk.exited.contains(&owner) {
                    self.walk.exited.push(owner);
                }
                self.walk.consumed.insert(owner);
                self.follow(owner, &socket, work);
            }
            BlockKind::Pause { conds } => {
                if self.policy.stop.contains(&guid) {
                    self.wait_here(b, input);
                    return;
                }
                if let Some(s) = self.true_socket(b, &conds) {
                    self.assume(b, &s, "fact condition already true", work);
                    return;
                }
                let sockets: Vec<String> = blk.sockets().into_iter().map(str::to_owned).collect();
                if let Some(p) = self.policy.picks.get(&guid).cloned() {
                    self.assume(b, &p, "picked", work);
                } else if self.auto_in(b) {
                    self.auto_wait_or_park(b, input, &sockets, work);
                } else if self.through_at(b) && sockets.len() <= 1 {
                    self.assume(b, sockets.first().map_or("", String::as_str), "assumed satisfied", work);
                } else {
                    if self.through_at(b) {
                        self.walk.events.push(Event::Unresolved { block: b, why: format!("pause block with outputs {sockets:?} needs a pick") });
                    }
                    self.wait_here(b, input);
                }
            }
            BlockKind::Wait if blk.class == "CQuestAndBlock" => {
                // Fires once every connected input socket has been activated (Letho invited donor:
                // `e9e84a64` still waits with `In 1` while the walk brought only that input); the
                // template's saved input counts, `through` does not satisfy it.
                if self.policy.stop.contains(&guid) {
                    self.wait_here(b, input);
                    return;
                }
                let seen = self.and_inputs.entry(b).or_default();
                for s in self.policy.waiting.get(&guid).into_iter().flatten().chain(std::iter::once(&input.to_owned())) {
                    if !seen.contains(s) {
                        seen.push(s.clone());
                    }
                }
                let wanted = self.lib.input_sockets(b);
                if wanted.iter().all(|w| seen.contains(w)) {
                    self.pending_waits.retain(|w| w.block != b);
                    self.walk.waits.retain(|w| w.block != b);
                    self.fired(b, input, work);
                } else {
                    self.pending_waits.retain(|w| w.block != b);
                    self.wait_here(b, input);
                }
            }
            BlockKind::Scene { .. } | BlockKind::Interaction { .. } | BlockKind::Wait => {
                if self.policy.stop.contains(&guid) {
                    self.wait_here(b, input);
                    return;
                }
                let sockets: Vec<String> = blk.sockets().into_iter().map(str::to_owned).collect();
                if let Some(p) = self.policy.picks.get(&guid).cloned() {
                    self.scene_effects(b, input, &p);
                    self.assume(b, &p, "picked", work);
                } else if self.auto_in(b) {
                    self.auto_wait_or_park(b, input, &sockets, work);
                } else if self.through_at(b) && sockets.len() <= 1 {
                    let s = sockets.first().cloned().unwrap_or_default();
                    self.scene_effects(b, input, &s);
                    self.assume(b, &s, "assumed satisfied", work);
                } else {
                    if self.through_at(b) {
                        self.walk.events.push(Event::Unresolved { block: b, why: format!("outputs {sockets:?} need a pick") });
                    }
                    self.wait_here(b, input);
                }
            }
        }
    }

    /// The facts a scene sets on the way from `input` to the output `socket` (`scene::Scene::outcome`):
    /// certain ones are set now, the ones behind an unpicked choice are recorded as `Maybe`.
    fn scene_effects(&mut self, b: usize, input: &str, socket: &str) {
        let file = match &self.lib.blocks[b].kind {
            BlockKind::Scene { file } | BlockKind::Interaction { file, .. } => file.clone(),
            _ => return,
        };
        if file.is_empty() {
            return;
        }
        if !self.lib.has_resources(".w2scene") {
            if !self.walk.events.iter().any(|e| matches!(e, Event::Note { text, .. } if text.starts_with("no scene files"))) {
                self.walk.events.push(Event::Note { block: b, text: "no scene files loaded (use --install or a folder with .w2scene): facts set inside scenes are not derived".into() });
            }
            return;
        }
        let scene = match self.lib.scene(&file) {
            Ok(s) => s,
            Err(e) => {
                self.walk.events.push(Event::Note { block: b, text: format!("scene {file} not read: {e}") });
                return;
            }
        };
        let guid = self.lib.blocks[b].guid;
        let picks = self.policy.scene_picks.get(&guid).cloned().unwrap_or_default();
        let outcome = scene.outcome(input, socket, &picks, &|c| self.eval(c));
        match outcome {
            Err(e) => self.walk.events.push(Event::Note { block: b, text: format!("scene {file}: {e}") }),
            Ok(o) => {
                for f in o.certain {
                    match f {
                        FactOp::Add { id, value } => {
                            *self.walk.facts.entry(id.clone()).or_insert(0) += value;
                            self.walk.events.push(Event::Fact { block: b, id, value });
                        }
                        FactOp::Remove { id } => {
                            let had = self.fact(&id);
                            self.walk.facts.insert(id.clone(), -(self.base)(&id));
                            if had != 0 && !self.walk.removed_facts.contains(&id) {
                                self.walk.removed_facts.push(id.clone());
                            }
                            self.walk.events.push(Event::FactRemoved { block: b, id });
                        }
                    }
                }
                for (f, via) in o.maybe {
                    let id = f.id().to_owned();
                    let where_ = format!("{}: {via}", file.rsplit('/').next().unwrap_or(&file));
                    self.walk.events.push(Event::Maybe { block: b, id: id.clone(), via: where_.clone() });
                    if !self.walk.maybe_facts.iter().any(|(x, _)| *x == id) {
                        self.walk.maybe_facts.push((id, where_));
                    }
                }
            }
        }
    }

    /// Auto-complete mode, `through` walk: is this block inside the quest (one of the quest's running phase
    /// blocks is in its scope chain)?
    fn auto_in(&self, b: usize) -> bool {
        match &self.policy.auto {
            Some(a) if self.policy.through => self.scope_of(b).iter().any(|g| a.instances.contains(g)),
            _ => false,
        }
    }

    /// `through` applies to this block: everywhere without the auto mode, only inside the quest with it.
    fn through_at(&self, b: usize) -> bool {
        self.policy.through && (self.policy.auto.is_none() || self.auto_in(b))
    }

    /// The goal in force: the root's Success until it is reached, then the end of the quest's phase.
    fn auto_goal(&self) -> Option<(&Scores, &'static str)> {
        self.policy.auto.as_ref().map(|a| if self.walk.auto_success { (&a.exit, "end") } else { (&a.success, "success") })
    }

    /// The output of `b` nearest the goal (the first such socket on a tie).
    fn auto_best(&self, b: usize, sockets: &[String]) -> Option<(String, String)> {
        let (s, goal) = self.auto_goal()?;
        let best = sockets.iter().filter_map(|x| s.socket(self.lib, b, x).map(|d| (x, d))).min_by_key(|(_, d)| *d)?;
        Some((best.0.clone(), format!("{goal} in {}", best.1 + 1)))
    }

    /// A wait inside the quest: the output to take, or `None` = it stays waiting (no output leads to the
    /// goal: a failure listener, a despawn after the player leaves, ...). A choice among several outputs is recorded.
    fn auto_wait(&mut self, b: usize, sockets: &[String]) -> Option<String> {
        let (s, why) = self.auto_best(b, sockets)?;
        if sockets.len() > 1 {
            self.walk.auto_picks.push(AutoPick { block: b, socket: s.clone(), why });
        }
        Some(s)
    }

    /// An instant block inside the quest that must take one output (a condition the walker cannot evaluate,
    /// a random block): the output nearest the goal, else the first. Recorded.
    fn auto_forced(&mut self, b: usize, sockets: &[String]) -> Option<String> {
        let (s, why) = self.auto_best(b, sockets).or_else(|| sockets.first().map(|s| (s.clone(), "first output".to_owned())))?;
        self.walk.auto_picks.push(AutoPick { block: b, socket: s.clone(), why });
        Some(s)
    }

    /// A wait inside the quest in auto mode: take the chosen output, or park.
    fn auto_wait_or_park(&mut self, b: usize, input: &str, sockets: &[String], work: &mut Vec<(usize, String)>) {
        match self.auto_wait(b, sockets) {
            Some(s) => {
                self.scene_effects(b, input, &s);
                self.assume(b, &s, "auto", work);
            }
            None => {
                self.walk.auto_parked.push(b);
                self.wait_here(b, input);
            }
        }
    }

    fn fired(&mut self, b: usize, input: &str, work: &mut Vec<(usize, String)>) {
        self.walk.events.push(Event::Fired { block: b, input: input.to_owned() });
        self.walk.consumed.insert(b);
        self.follow_all(b, work);
    }

    /// Drop waits inside exited scopes; list entered scopes that still run.
    fn finish(&mut self) {
        let mut waits = std::mem::take(&mut self.walk.waits);
        for w in self.pending_waits.drain(..) {
            if !waits.contains(&w) {
                waits.push(w);
            }
        }
        // W-3: nothing waits inside a phase that ended and was not re-entered (a cut-control block
        // whose scope chain was resolved through another importer slipped through the exit's retain).
        let gone: Vec<Guid> = self.exited_set.iter().map(|p| self.lib.blocks[*p].guid).collect();
        waits.retain(|w| !w.scope.iter().any(|s| gone.contains(s)) && !gone.contains(&self.lib.blocks[w.block].guid));
        self.walk.waits = waits;
        // W-3 (D16 lodge, D11 refuse): a phase entered inside a phase that ended afterwards is gone
        // with it; writing its wrapper would need the ended parent's thread ("questThread is not in
        // the save").
        let entered: Vec<usize> = self.walk.entered.iter().copied().filter(|p| !self.exited_set.contains(p) && !self.scope_of(*p).iter().any(|s| gone.contains(s))).collect();
        self.walk.entered = entered;
    }

    pub fn into_walk(self) -> Walk {
        self.walk
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quest::{Block, Conn};

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
            graph: 0,
            kind,
        }
    }

    fn lib(blocks: Vec<Block>) -> Library {
        let mut l = Library::default();
        for b in blocks {
            l.blocks.push(b);
        }
        l.reindex();
        l
    }

    #[test]
    fn a_decision_sets_facts_and_stops_at_the_next_wait() {
        // 0 scene --A--> 1 fact --> 2 journal(Success) --> 3 pause(fact x > 0) --> 4 pause(other)
        //         --B--> 5 fact
        let l = lib(vec![
            block(0, "CQuestSceneBlock", BlockKind::Scene { file: "s".into() }, vec![("A", 1, "In"), ("B", 5, "In")], None),
            block(1, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "x".into(), value: 1 }, vec![("Out", 2, "Success")], None),
            block(2, "CJournalQuestBlock", BlockKind::Journal { quest: true, path: vec![] }, vec![("Out", 3, "In")], None),
            block(3, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Fact { id: "x".into(), cmp: "CF_Greater".into(), value: 0 }] }, vec![("", 4, "In")], None),
            block(4, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("CQuestInsideTriggerCondition".into())] }, vec![], None),
            block(5, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "y".into(), value: 1 }, vec![], None),
        ]);
        let base = |_: &str| 0;
        let mut p = Policy::default();
        p.picks.insert(l.blocks[0].guid, "A".into());
        let mut w = Walker::new(&l, &base, p);
        w.run(&[(0, "Input".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("x"), Some(&1));
        assert_eq!(walk.facts.get("y"), None);
        assert!(walk.events.iter().any(|e| matches!(e, Event::Journal { status, .. } if status == "JS_Success")));
        // The fact-gated pause fell through; the trigger wait is the new frontier.
        assert_eq!(walk.waits.len(), 1);
        assert_eq!(walk.waits[0].block, 4);
        assert!(walk.consumed.contains(&3));
    }

    #[test]
    fn a_phase_output_cancels_waits_inside_the_phase() {
        // 0 phase(embedded) -> inner: 1 input -> 2 pause(other) and 3 output; 0.Out -> 4 pause(other)
        let l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![("Out", 4, "In")], None),
            block(1, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "In".into() }, vec![("", 2, "In"), ("", 3, "In")], Some(0)),
            block(2, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("x".into())] }, vec![], Some(0)),
            block(3, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(0)),
            block(4, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("x".into())] }, vec![], None),
        ]);
        let mut l = l;
        l.blocks[1].graph = 9;
        l.blocks[2].graph = 9;
        l.blocks[3].graph = 9;
        // graph_blocks is private: register through add_parsed is not possible here, so use inner lookup via owner only.
        let base = |_: &str| 0;
        let mut w = Walker::new(&l, &base, Policy::default());
        // Activate the input block directly (what entering the phase does).
        w.run(&[(1, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.exited, vec![0]);
        assert!(walk.waits.iter().all(|w| w.block != 2), "the wait inside the exited phase is cancelled");
        assert_eq!(walk.waits.iter().map(|w| w.block).collect::<Vec<_>>(), vec![4]);
    }

    #[test]
    fn a_phase_exit_drops_the_work_still_queued_inside_it() {
        // 0 phase(embedded) -> inner: 1 input -> 2 output "Out" and 3 fact x -> 4 output "Fail"; 0.Out -> 5 fact done, 0.Fail -> 6 fact failed
        let mut l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![("Out", 5, "In"), ("Fail", 6, "In")], None),
            block(1, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "In".into() }, vec![("", 2, "In"), ("", 3, "In")], Some(0)),
            block(2, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(0)),
            block(3, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "x".into(), value: 1 }, vec![("Out", 4, "In")], Some(0)),
            block(4, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Fail".into() }, vec![], Some(0)),
            block(5, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "done".into(), value: 1 }, vec![], None),
            block(6, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "failed".into(), value: 1 }, vec![], None),
        ]);
        for i in 1..=4 {
            l.blocks[i].graph = 9;
        }
        let base = |_: &str| 0;
        let mut w = Walker::new(&l, &base, Policy::default());
        w.run(&[(1, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("done"), Some(&1));
        assert_eq!(walk.facts.get("x"), None, "the branch queued inside the phase must not run after the exit");
        assert_eq!(walk.facts.get("failed"), None);
        assert_eq!(walk.events.iter().filter(|e| matches!(e, Event::Exit { .. })).count(), 1);
    }

    #[test]
    fn a_phase_entered_inside_an_exited_phase_is_not_a_new_scope() {
        // 0 phase(embedded 9) -> inner: 1 input -> 2 phase(embedded 8) and 3 cut-control -> 4 output "Out";
        // 2's inner: 5 input -> 6 pause(other). 0.Out -> 7 fact done.
        // W-3: after 0 exits, neither 2 (an entered scope) nor 3 (a cut-control wait) may be written.
        let mut l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![("Out", 7, "In")], None),
            block(1, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "In".into() }, vec![("", 2, "In"), ("", 3, "In")], Some(0)),
            block(2, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(8), import: None }, vec![], Some(0)),
            block(3, "CQuestCutControlBlock", BlockKind::Instant, vec![("Out", 4, "In")], Some(0)),
            block(4, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(0)),
            block(5, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "In".into() }, vec![("", 6, "In")], Some(2)),
            block(6, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("x".into())] }, vec![], Some(2)),
            block(7, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "done".into(), value: 1 }, vec![], None),
        ]);
        for i in 1..=4 {
            l.blocks[i].graph = 9;
        }
        l.blocks[5].graph = 8;
        l.blocks[6].graph = 8;
        let base = |_: &str| 0;
        let mut w = Walker::new(&l, &base, Policy::default());
        w.run(&[(1, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("done"), Some(&1));
        assert!(walk.exited.contains(&0));
        assert!(walk.waits.is_empty(), "nothing waits inside the ended phase: {:?}", walk.waits);
        assert!(walk.entered.is_empty(), "the inner phase ended with its parent: {:?}", walk.entered);
    }

    #[test]
    fn a_file_imported_twice_in_a_row_is_walked_once_per_importer() {
        // W-4: 0 phase(import u) -> Out -> 1 phase(import u) -> Out -> 2 fact done.
        // u.w2phase root: 3 input -> 4 fact x -> 5 output "Out". Entering 1 after 0 must walk u again
        // (x twice, done set); entering 0 a second time (a Restart loop) must not.
        let mut l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: None, import: Some("u.w2phase".into()) }, vec![("Out", 1, "In")], None),
            block(1, "CQuestPhaseBlock", BlockKind::Phase { embedded: None, import: Some("u.w2phase".into()) }, vec![("Out", 2, "In")], None),
            block(2, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "done".into(), value: 1 }, vec![], None),
            block(3, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "In".into() }, vec![("", 4, "In")], None),
            block(4, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "x".into(), value: 1 }, vec![("Out", 5, "In")], None),
            block(5, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], None),
        ]);
        for i in 3..=5 {
            l.blocks[i].file = "u.w2phase".into();
            l.blocks[i].graph = 7;
        }
        l.reindex();
        l.set_root_graph("u.w2phase", 7);
        assert_eq!(l.importers_of("u.w2phase"), &[0, 1]);
        let base = |_: &str| 0;
        let mut w = Walker::new(&l, &base, Policy::default());
        w.run(&[(0, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("x"), Some(&2), "the file ran once per importer: {:?}", walk.events);
        assert_eq!(walk.facts.get("done"), Some(&1));
        assert_eq!(walk.exited, vec![0, 1]);
        assert!(walk.entered.is_empty());
        // The same importer entered again does not walk its file a second time.
        let mut l2 = l;
        l2.blocks[1].outs = vec![("Out", 0, "In")].into_iter().map(|(s, t, i)| Conn { socket: s.into(), target: t, input: i.into() }).collect();
        let mut w = Walker::new(&l2, &base, Policy::default());
        w.run(&[(0, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("x"), Some(&2), "no third lap: {:?}", walk.events);
        assert_eq!(walk.events.iter().filter(|e| matches!(e, Event::Enter { phase: 0, .. })).count(), 1);
    }

    #[test]
    fn a_phase_output_exits_the_importer_this_walk_entered_and_demo_files_never_import() {
        // random_timer (pack 35): importers 0 and 1 of u.w2phase both run in the template; the walk enters 1.
        // u.w2phase: 3 input -> 4 output "Out". 1.Out -> 2 fact. 6 (a demo file) imports u.w2phase too.
        let mut l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: None, import: Some("u.w2phase".into()) }, vec![], None),
            block(1, "CQuestPhaseBlock", BlockKind::Phase { embedded: None, import: Some("u.w2phase".into()) }, vec![("Out", 2, "In")], None),
            block(2, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "done".into(), value: 1 }, vec![], None),
            block(3, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "In".into() }, vec![("", 4, "In")], None),
            block(4, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], None),
            block(5, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "unused".into(), value: 1 }, vec![], None),
            block(6, "CQuestPhaseBlock", BlockKind::Phase { embedded: None, import: Some("u.w2phase".into()) }, vec![], None),
        ]);
        for i in 3..=4 {
            l.blocks[i].file = "u.w2phase".into();
            l.blocks[i].graph = 7;
        }
        l.blocks[6].file = "quests/demo_files/hands_on.w2phase".into();
        l.reindex();
        l.set_root_graph("u.w2phase", 7);
        assert_eq!(l.importers_of("u.w2phase"), &[0, 1], "the demo file is not an importer");
        let base = |_: &str| 0;
        let mut p = Policy::default();
        p.running.insert(l.blocks[0].guid);
        p.running.insert(l.blocks[1].guid);
        let mut w = Walker::new(&l, &base, p);
        w.run(&[(1, "In".into())]);
        let walk = w.into_walk();
        assert!(!walk.events.iter().any(|e| matches!(e, Event::Unresolved { .. })), "{:?}", walk.events);
        assert_eq!(walk.exited, vec![1]);
        assert_eq!(walk.facts.get("done"), Some(&1));
    }

    #[test]
    fn a_phase_re_entered_through_another_socket_runs_its_content_again() {
        // W-6 (Cerys): phase 0 (embedded 9): input "First" 1 -> 2 pause(fact already true) -> 3 output "To_q208"
        // and 1 -> 4 scene (waits); input "Second" 5 -> 4. 0.To_q208 -> 6 phase(embedded 8): 7 input -> 8 output "Out";
        // 6.Out -> 0 "Second". The scene must wait at the end although the first lap visited it.
        let mut l = lib(vec![
            block(0, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![("To_q208", 6, "In")], None),
            block(1, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "First".into() }, vec![("", 2, "In"), ("", 4, "Input")], Some(0)),
            block(2, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Fact { id: "done".into(), cmp: "CF_Greater".into(), value: 0 }] }, vec![("", 3, "In")], Some(0)),
            block(3, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "To_q208".into() }, vec![], Some(0)),
            block(4, "CQuestSceneBlock", BlockKind::Scene { file: String::new() }, vec![("Out", 9, "In")], Some(0)),
            block(5, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "Second".into() }, vec![("", 4, "Input")], Some(0)),
            block(6, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(8), import: None }, vec![("Out", 0, "Second")], None),
            block(7, "CQuestPhaseInputBlock", BlockKind::PhaseInput { socket: "In".into() }, vec![("", 8, "In")], Some(6)),
            block(8, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(6)),
            block(9, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "asked".into(), value: 1 }, vec![], Some(0)),
        ]);
        for i in [1, 2, 3, 4, 5, 9] {
            l.blocks[i].graph = 9;
        }
        l.blocks[7].graph = 8;
        l.blocks[8].graph = 8;
        l.reindex();
        let base = |id: &str| i64::from(id == "done");
        let mut w = Walker::new(&l, &base, Policy::default());
        w.run(&[(0, "First".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.reentered, vec![0], "{:?}", walk.events);
        assert_eq!(walk.waits.iter().map(|w| w.block).collect::<Vec<_>>(), vec![4], "{:?}", walk.events);
        assert_eq!(walk.facts.get("asked"), None);
    }

    #[test]
    fn an_assumed_wait_lets_the_instant_siblings_of_its_socket_run_first() {
        // W-5 (Roche): 0 scene --A--> 1 pause(delay) --> 2 output "Out" (ends phase 5) and --A--> 3 journal(Success);
        // phase 5 (embedded 9) holds 0..3; 5.Out -> 4 fact done. The journal block fires before the exit.
        let mut l = lib(vec![
            block(0, "CQuestSceneBlock", BlockKind::Scene { file: String::new() }, vec![("A", 1, "In"), ("A", 3, "Success")], Some(5)),
            block(1, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Delay { ms: 1000 }] }, vec![("", 2, "In")], Some(5)),
            block(2, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(5)),
            block(3, "CJournalQuestBlock", BlockKind::Journal { quest: true, path: vec![] }, vec![], Some(5)),
            block(4, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "done".into(), value: 1 }, vec![], None),
            block(5, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![("Out", 4, "In")], None),
        ]);
        for i in 0..=3 {
            l.blocks[i].graph = 9;
        }
        let base = |_: &str| 0;
        let mut p = Policy::default();
        p.picks.insert(l.blocks[0].guid, "A".into());
        let mut w = Walker::new(&l, &base, p);
        w.run(&[(0, "Input".into())]);
        let walk = w.into_walk();
        assert!(walk.events.iter().any(|e| matches!(e, Event::Journal { block: 3, .. })), "{:?}", walk.events);
        assert_eq!(walk.facts.get("done"), Some(&1));
        assert_eq!(walk.exited, vec![5]);
    }

    #[test]
    fn starts_fire_in_order_and_a_start_in_an_ended_phase_does_not_fire() {
        // phase 4 (embedded 9): 0 pause(other) -> 1 output "Out"; 2 pause(other) -> 3 fact late. 4.Out -> 5 fact done.
        // Starts [0, 2]: 0's walk ends the phase, so 2 (inside it) never fires (spirit donor `c4d54c91`).
        let mut l = lib(vec![
            block(0, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("x".into())] }, vec![("", 1, "In")], Some(4)),
            block(1, "CQuestPhaseOutputBlock", BlockKind::PhaseOutput { socket: "Out".into() }, vec![], Some(4)),
            block(2, "CQuestPauseConditionBlock", BlockKind::Pause { conds: vec![Cond::Other("x".into())] }, vec![("", 3, "In")], Some(4)),
            block(3, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "late".into(), value: 1 }, vec![], Some(4)),
            block(4, "CQuestPhaseBlock", BlockKind::Phase { embedded: Some(9), import: None }, vec![("Out", 5, "In")], None),
            block(5, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "done".into(), value: 1 }, vec![], None),
        ]);
        for i in 0..=3 {
            l.blocks[i].graph = 9;
        }
        let base = |_: &str| 0;
        let mut w = Walker::new(&l, &base, Policy { through: true, ..Default::default() });
        w.run(&[(0, "In".into()), (2, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("done"), Some(&1));
        assert_eq!(walk.facts.get("late"), None, "{:?}", walk.events);
        assert!(walk.events.iter().any(|e| matches!(e, Event::Note { block: 2, .. })));
        // The other order: 2 fires first, then 0 ends the phase.
        let mut w = Walker::new(&l, &base, Policy { through: true, ..Default::default() });
        w.run(&[(2, "In".into()), (0, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("late"), Some(&1));
        assert_eq!(walk.facts.get("done"), Some(&1));
    }

    #[test]
    fn an_and_block_waits_for_all_its_inputs() {
        // 0 fact a -> 2.In 1 ; 1 fact b -> 2.In 2 ; 2 and -> 3 fact done
        let l = lib(vec![
            block(0, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "a".into(), value: 1 }, vec![("Out", 2, "In 1")], None),
            block(1, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "b".into(), value: 1 }, vec![("Out", 2, "In 2")], None),
            block(2, "CQuestAndBlock", BlockKind::Wait, vec![("Out", 3, "In")], None),
            block(3, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "done".into(), value: 1 }, vec![], None),
        ]);
        assert_eq!(l.input_sockets(2), vec!["In 1".to_owned(), "In 2".to_owned()]);
        let base = |_: &str| 0;
        // One input of two: the block waits with that input, `through` does not pass it.
        let mut w = Walker::new(&l, &base, Policy { through: true, ..Default::default() });
        w.run(&[(0, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("done"), None);
        assert_eq!(walk.waits.iter().map(|w| (w.block, w.input.as_str())).collect::<Vec<_>>(), vec![(2, "In 1")]);
        // Both inputs in one walk: it fires and leaves the frontier.
        let mut w = Walker::new(&l, &base, Policy { through: true, ..Default::default() });
        w.run(&[(0, "In".into()), (1, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("done"), Some(&1));
        assert!(walk.waits.is_empty() && walk.consumed.contains(&2));
        // The template already holds the block with `In 1`: the walk's `In 2` completes it.
        let mut p = Policy { through: true, ..Default::default() };
        p.waiting.insert(l.blocks[2].guid, vec!["In 1".into()]);
        let mut w = Walker::new(&l, &base, p);
        w.run(&[(1, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("done"), Some(&1));
        assert!(walk.waits.is_empty());
    }

    #[test]
    fn journal_inputs_map_to_statuses() {
        let l = lib(vec![block(0, "CJournalQuestBlock", BlockKind::Journal { quest: true, path: vec![] }, vec![], None)]);
        let base = |_: &str| 0;
        for (input, status) in [("Activate", "JS_Active"), ("Success", "JS_Success"), ("Failure", "JS_Failed"), ("Fail", "JS_Failed")] {
            let mut w = Walker::new(&l, &base, Policy::default());
            w.run(&[(0, input.into())]);
            let walk = w.into_walk();
            assert!(walk.events.iter().any(|e| matches!(e, Event::Journal { status: s, .. } if s == status)), "{input} -> {status}");
        }
    }

    #[test]
    fn multi_condition_pause_takes_the_true_conditions_socket() {
        let l = lib(vec![block(
            0,
            "CQuestPauseConditionBlock",
            BlockKind::Pause {
                conds: vec![
                    Cond::Fact { id: "a".into(), cmp: "CF_GreaterEqual".into(), value: 1 },
                    Cond::Fact { id: "b".into(), cmp: "CF_GreaterEqual".into(), value: 1 },
                ],
            },
            vec![("to_a", 1, "In"), ("to_b", 2, "In")],
            None,
        ), block(1, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "ra".into(), value: 1 }, vec![], None), block(2, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "rb".into(), value: 1 }, vec![], None)]);
        // `b` becomes true during the walk (a listener fires only for facts the walk set).
        let mut l = l;
        l.blocks.push(block(3, "CQuestFactsDBChangingBlock", BlockKind::Fact { id: "b".into(), value: 1 }, vec![], None));
        let base = |_: &str| 0;
        let mut w = Walker::new(&l, &base, Policy::default());
        w.run(&[(3, "In".into())]);
        w.fire_listeners(&[(0, "In".into())]);
        let walk = w.into_walk();
        assert_eq!(walk.facts.get("rb"), Some(&1));
        assert_eq!(walk.facts.get("ra"), None);
    }
}
