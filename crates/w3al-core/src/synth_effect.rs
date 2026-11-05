//! `[effects.<key>.synth]`: an effect synthesized from the game's own quest graphs instead of copied
//! from a donor save (docs/plan/no-human-pipeline.md, DECISIONS.md "Synthesized effects").
//!
//! The table holds exactly the arguments of `w3al synth-apply`:
//!
//! ```toml
//! [effects.keira_km.synth]
//! start   = ["215f2719"]                   # blocks waiting in the save to fire: GUID (8 or 32 hex) [":input"]
//! through = true                           # single-output waits count as done
//! pick    = ["d24f649d=Keira to KM"]       # decision block = output socket to take
//! stop    = ["4c2a9b2d"]                   # waits to park at (end of the decision window)
//! ```
//!
//! Export walks the decision with `w3graph::walk::Walker` from the save as it is when the effect's turn
//! comes, then writes the walk with `w3graph::synth::apply` (quest frontier, scene registry, journal,
//! facts). All format and walk logic lives in `w3graph`; this module only turns the table into the
//! walker's `Policy` and start list.
//!
//! [`walk`] turns the table into a `w3graph::synth::WalkSpec` and calls `w3graph::synth::plan`, the
//! same function `w3al synth-walk` / `synth-apply` use (block-key resolution preferring the save's
//! own frontier, socket check, starts with their saved input, listeners on, max 20,000 steps), then
//! refuses a walk with unresolved points (`w3graph::synth::unresolved`). One deliberate difference
//! from the CLI: a `start` block that does not wait in the save is refused here
//! (`require_waiting_starts`), because export must not walk a decision from a save that is not at it.
//! `scene_pick` (`--scene-pick`) names the choice line inside a scene file for decisions the quest
//! graph does not see (Letho invited / elsewhere, Yennefer love / friends).
use serde::{Deserialize, Serialize};
use w3graph::frontier::{self, Frontier, Prediction};
use w3graph::quest::Library;
use w3graph::synth::{self, SynthReport};
use w3graph::walk::Walk;
use w3save::journal::JournalEdit;
use w3save::Document;

/// `w3al synth-walk` / `synth-apply`'s default `--max-steps`.
pub const MAX_STEPS: usize = 20_000;

/// The `[effects.<key>.synth]` table (field names fixed: content files use them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthEffect {
    /// Blocks waiting in the save to fire, `GUID[:input]` (8 or 32 hex digits; the input defaults to the
    /// one the save holds). `--start`.
    #[serde(default)]
    pub start: Vec<String>,
    /// Assume single-output waits satisfied and go on (`--through`).
    #[serde(default)]
    pub through: bool,
    /// `GUID=socket`: the output to take at a decision (`--pick`).
    #[serde(default)]
    pub pick: Vec<String>,
    /// Waits to park at even in `through` mode (`--stop`).
    #[serde(default)]
    pub stop: Vec<String>,
    /// `GUID=section`: the choice line to take inside that scene block's `.w2scene` (named by the
    /// section the line leads to, as `w3al scene-info` lists them). Resolves facts set inside the scene
    /// file (`--scene-pick`).
    #[serde(default)]
    pub scene_pick: Vec<String>,
    /// Auto-complete mode (`w3graph::complete`): finish this quest to its Success and end from whatever state
    /// the save holds it in, by the walker's own deterministic policy. The quest's journal root as
    /// `GROUP/QUEST` (two GUIDs of 32 hex digits, the save's journal path) or `QUEST` alone. Not together
    /// with `start` / `through`; `pick` / `scene_pick` / `stop` with it are the player's answers inside the
    /// quest (forced outputs, forced scene lines, ways not taken), which only a `decision` completion has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complete: Option<String>,
    /// With `complete`: the quest is a decision quest completed late on a later template (`[effects.*.late]`,
    /// see [`LateEffect`]); the decision-catalogue refusal of `w3graph::complete` is lifted and the answer
    /// comes from `pick` / `scene_pick` / `stop`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub decision: bool,
}

/// `[effects.<key>.late]`: the same answer completed **late**, on a later template that has not reached the
/// quest (Now or Never on the Skellige template: the pack line sailed first). The quests of `complete` are
/// auto-completed in order (`w3graph::complete`, each from the previous one's result: a prerequisite first,
/// e.g. A Matter of Life and Death before Now or Never), with the player's answer forced by `pick`,
/// `scene_pick` and `stop` (as `w3al side-complete --allow-decision --pick ... --stop ...` takes them). The
/// effect's own `facts` / journal tables apply after the last quest. Used only when the chosen template lists
/// the quest in its registry `late` list (`questbook/templates.toml`).
///
/// ```toml
/// [effects.triss_leaves.late]
/// complete   = ["a93885ce.../32105cdf...", "419f978f.../400c9506..."]   # GROUP/QUEST journal roots, in order
/// pick       = ["d6a0ac74=geralt alone"]
/// scene_pick = ["ebdeb52d=sq301_00a_ok"]
/// stop       = ["f9fc4999"]                                            # waits the player did not take
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateEffect {
    pub complete: Vec<String>,
    #[serde(default)]
    pub pick: Vec<String>,
    #[serde(default)]
    pub scene_pick: Vec<String>,
    #[serde(default)]
    pub stop: Vec<String>,
}

impl LateEffect {
    /// One `decision` completion per quest, in order, each with all the forced answers.
    pub fn steps(&self) -> Vec<SynthEffect> {
        self.complete
            .iter()
            .map(|k| SynthEffect { complete: Some(k.clone()), pick: self.pick.clone(), scene_pick: self.scene_pick.clone(), stop: self.stop.clone(), decision: true, ..Default::default() })
            .collect()
    }

    pub fn check(&self) -> Result<(), String> {
        if self.complete.is_empty() {
            return Err("late: `complete` names no quest".into());
        }
        self.steps().iter().try_for_each(SynthEffect::check).map_err(|e| format!("late: {e}"))
    }
}

/// Shape check of a `complete` key: `GROUP/QUEST` or `QUEST`, 32 hex digits each.
pub fn check_complete_key(k: &str) -> Result<(), String> {
    let hex32 = |s: &str| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit());
    let ok = match k.split_once('/') {
        Some((g, q)) => hex32(g) && hex32(q),
        None => hex32(k),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("synth complete {k:?}: want the quest's journal root as GROUP/QUEST (32 hex digits each) or QUEST"))
    }
}

fn check_key(what: &str, k: &str) -> Result<(), String> {
    let ok = (k.len() == 8 || k.len() == 32) && k.bytes().all(|b| b.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(format!("synth {what} {k:?}: a block key is 8 or 32 hex digits (as `w3al synth-walk` takes it)"))
    }
}

impl SynthEffect {
    /// Shape checks that need no game files (lint). Whether the blocks exist and the sockets are real is
    /// checked against the graphs at export.
    pub fn check(&self) -> Result<(), String> {
        if let Some(k) = &self.complete {
            if !self.start.is_empty() || self.through {
                return Err("synth `complete` walks by its own policy: no `start` or `through` with it".into());
            }
            if !self.decision && (!self.pick.is_empty() || !self.stop.is_empty() || !self.scene_pick.is_empty()) {
                return Err("synth `complete` walks by its own policy: no `start`, `pick`, `stop` or `scene_pick` with it (forced answers belong to a decision quest completed late: `[effects.*.late]`)".into());
            }
            self.check_answers()?;
            return check_complete_key(k);
        }
        if self.decision {
            return Err("synth `decision` goes with `complete`".into());
        }
        if self.start.is_empty() {
            return Err("synth needs at least one `start` block (a block waiting in the template)".into());
        }
        if self.pick.is_empty() && self.stop.is_empty() {
            return Err("synth needs `pick` (the decision) or `stop` (where the walk ends), or both".into());
        }
        for s in &self.start {
            let (k, input) = s.split_once(':').map_or((s.as_str(), None), |(k, i)| (k, Some(i)));
            check_key("start", k)?;
            if input.is_some_and(|i| i.trim().is_empty()) {
                return Err(format!("synth start {s:?}: empty input after `:`"));
            }
        }
        self.check_answers()
    }

    /// Shape of `pick` / `stop` / `scene_pick`.
    fn check_answers(&self) -> Result<(), String> {
        for p in &self.pick {
            let (k, socket) = p.split_once('=').ok_or_else(|| format!("synth pick {p:?}: want `GUID=socket`"))?;
            check_key("pick", k)?;
            if socket.is_empty() {
                return Err(format!("synth pick {p:?}: empty socket"));
            }
        }
        for s in &self.stop {
            check_key("stop", s)?;
        }
        for p in &self.scene_pick {
            let (k, section) = p.split_once('=').ok_or_else(|| format!("synth scene_pick {p:?}: want `GUID=section`"))?;
            check_key("scene_pick", k)?;
            if section.trim().is_empty() {
                return Err(format!("synth scene_pick {p:?}: empty section"));
            }
        }
        Ok(())
    }
}

/// Walk the decision from `save` (what `w3al synth-walk` does with the same arguments). Refuses a walk
/// with unresolved points, as `synth-apply` does.
pub fn walk(lib: &Library, save: &Document, tf: &Frontier, spec: &SynthEffect) -> Result<Walk, String> {
    spec.check()?;
    let ws = synth::WalkSpec {
        starts: spec.start.clone(),
        picks: spec.pick.clone(),
        scene_picks: spec.scene_pick.clone(),
        stops: spec.stop.clone(),
        through: spec.through,
        listeners: true,
        max_steps: MAX_STEPS,
        require_waiting_starts: true,
    };
    let (walk, _notes) = synth::plan(lib, save, tf, &ws).map_err(|e| e.to_string())?;
    let unresolved = synth::unresolved(lib, &walk);
    if !unresolved.is_empty() {
        return Err(format!("the walk has {} unresolved point(s) (add a pick or a stop): {}", unresolved.len(), unresolved.join("; ")));
    }
    Ok(walk)
}

/// A synthesized effect: the new document plus what export verifies afterwards.
pub struct Synthesized {
    pub doc: Document,
    pub report: SynthReport,
    /// The journal edit the synthesis applied (`w3graph::synth::journal_edit_with` of the walk on the save, counters included).
    pub journal: JournalEdit,
    /// Facts removed by the walk's script blocks (whether or not the save had them).
    pub facts_removed: Vec<String>,
    /// Frontier GUIDs the walk removes / adds (`w3graph::frontier::predict`).
    pub prediction: Prediction,
}

/// The effects of quests completed late (`Choice::late`): each effect's `late` table becomes one
/// `decision` completion per quest (its own applied effect, in order), the effect's other tables (facts,
/// journal) going with the last one; `synth` / `graph` (the window's own walk) are dropped. An effect that
/// changes something but has no `late` table is refused.
pub fn late_effects(effects: Vec<crate::resolve::AppliedEffect>) -> Result<Vec<crate::resolve::AppliedEffect>, String> {
    let mut out = Vec::new();
    for a in effects {
        let Some(late) = a.effect.late.clone() else {
            return Err(format!("{}/{}: the effect has no `[late]` table, so it cannot be completed on a later template", a.quest, a.origin));
        };
        late.check().map_err(|e| format!("{}/{}: {e}", a.quest, a.origin))?;
        let steps = late.steps();
        let n = steps.len();
        for (i, s) in steps.into_iter().enumerate() {
            let origin = format!("{} (late {}/{n})", a.origin, i + 1);
            let effect = if i + 1 == n {
                crate::Effect { synth: Some(s), graph: None, late: None, ..a.effect.clone() }
            } else {
                crate::Effect { source: a.effect.source.clone(), synth: Some(s), ..Default::default() }
            };
            out.push(crate::resolve::AppliedEffect { quest: a.quest.clone(), origin, effect });
        }
    }
    Ok(out)
}

/// The auto-complete walk of a `complete` table (`w3graph::complete`): decision-catalogue files refused, unless
/// the table is a `decision` completion, whose forced answers (`pick`, `scene_pick`, `stop`) the walk takes.
pub fn complete_walk(lib: &Library, save: &Document, tf: &Frontier, key: &str, spec: &SynthEffect) -> Result<Walk, String> {
    check_complete_key(key)?;
    let idx = w3graph::complete::index(lib).map_err(|e| e.to_string())?;
    let q = idx.find(key).ok_or_else(|| format!("quest {key} is not in the install's journals"))?;
    let mut opts = w3graph::complete::Options::default();
    if spec.decision {
        opts.deny_files.clear();
        opts.check_consequences = false;
    }
    let opts = opts.with_picks(lib, tf, &spec.pick, &spec.scene_pick, &spec.stop)?;
    let c = w3graph::complete::complete(lib, &idx, save, tf, q, &opts).map_err(|r| format!("complete {}: {r}", q.base_name))?;
    Ok(c.walk)
}

/// Walk and write one synth effect into a copy of `save` (`w3graph::synth::apply`).
pub fn apply(lib: &Library, save: &Document, spec: &SynthEffect) -> Result<Synthesized, String> {
    let tf = frontier::read(save).map_err(|e| e.to_string())?;
    let walk = match &spec.complete {
        Some(k) => {
            spec.check()?;
            complete_walk(lib, save, &tf, k, spec)?
        }
        None => walk(lib, save, &tf, spec)?,
    };
    let prediction = frontier::predict(lib, &walk, &tf);
    let journal = synth::journal_edit_with(lib, &walk, save).map_err(|e| e.to_string())?;
    let (doc, report) = synth::apply(lib, &walk, save).map_err(|e| e.to_string())?;
    Ok(Synthesized { doc, report, journal, facts_removed: walk.removed_facts.clone(), prediction })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(start: &[&str], pick: &[&str], stop: &[&str]) -> SynthEffect {
        let v = |x: &[&str]| x.iter().map(|s| (*s).to_owned()).collect();
        SynthEffect { start: v(start), through: true, pick: v(pick), stop: v(stop), ..Default::default() }
    }

    #[test]
    fn late_table_expands_to_decision_completions() {
        let g = "419f978f".repeat(4);
        let (a, b) = ("32105cdf".repeat(4), "400c9506".repeat(4));
        let l: LateEffect = toml::from_str(&format!(
            "complete = [\"{g}/{a}\", \"{g}/{b}\"]\npick = [\"d6a0ac74=geralt alone\"]\nscene_pick = [\"ebdeb52d=sq301_00a_ok\"]\nstop = [\"f9fc4999\"]\n"
        ))
        .unwrap();
        assert!(l.check().is_ok(), "{:?}", l.check());
        let steps = l.steps();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].complete.as_deref(), Some(format!("{g}/{a}").as_str()));
        assert!(steps.iter().all(|s| s.decision && s.pick == l.pick && s.stop == l.stop && s.scene_pick == l.scene_pick));
        assert!(LateEffect::default().check().is_err(), "no quest");
        let bad: LateEffect = toml::from_str(&format!("complete = [\"{g}/{a}\"]\npick = [\"d6a0ac74\"]\n")).unwrap();
        assert!(bad.check().is_err(), "pick without socket");
        // Forced answers only on a decision completion; `decision` only with `complete`.
        let side = SynthEffect { complete: Some(format!("{g}/{a}")), pick: vec!["d6a0ac74=x".into()], ..Default::default() };
        assert!(side.check().is_err());
        assert!(SynthEffect { decision: true, ..side.clone() }.check().is_ok());
        assert!(SynthEffect { decision: true, ..spec(&["215f2719"], &["d24f649d=x"], &[]) }.check().is_err());
        assert!(SynthEffect { through: true, decision: true, ..side }.check().is_err());
    }

    #[test]
    fn complete_shape_checks() {
        let g = "67e5afe3ceb5c148a9cde5c4a99c7a84";
        let q = "8ce50f9eee92164cade1edae10fe019b";
        let c = |k: &str| SynthEffect { complete: Some(k.into()), ..Default::default() };
        assert!(c(&format!("{g}/{q}")).check().is_ok());
        assert!(c(q).check().is_ok());
        for bad in ["", "8ce50f9e", &format!("{g}/"), &format!("{g}/{q}/x"), "PLACEHOLDER_x"] {
            assert!(c(bad).check().is_err(), "{bad:?}");
        }
        let mut mixed = c(q);
        mixed.start = vec!["215f2719".into()];
        assert!(mixed.check().unwrap_err().contains("no `start`"));
        let s: SynthEffect = toml::from_str(&format!("complete = \"{g}/{q}\"\n")).unwrap();
        assert_eq!(s.complete.as_deref(), Some(format!("{g}/{q}").as_str()));
        assert!(s.check().is_ok());
    }

    #[test]
    fn shape_checks() {
        assert!(spec(&["215f2719"], &["d24f649d=Keira to KM"], &["4c2a9b2d"]).check().is_ok());
        let mut with_scene = spec(&["2da9cd80"], &["70d40ee9=NoFight"], &[]);
        with_scene.scene_pick = vec!["ed477b93=sq102_go _to_km_letho".into()];
        assert!(with_scene.check().is_ok());
        for bad in ["ed477b93", "ed477b93=", "ed477b9=x"] {
            with_scene.scene_pick = vec![bad.into()];
            assert!(with_scene.check().is_err(), "{bad}");
        }
        assert!(spec(&["215f2719:In"], &[], &["4c2a9b2d"]).check().is_ok());
        assert!(spec(&["215f2719"], &["d24f649d=x"], &[]).check().is_ok());
        assert!(spec(&[&"a".repeat(32)], &[], &["4c2a9b2d"]).check().is_ok());
        for bad in [
            spec(&[], &["d24f649d=x"], &[]),
            spec(&["215f2719"], &[], &[]),
            spec(&["215f271"], &[], &["4c2a9b2d"]),
            spec(&["215f2719:"], &[], &["4c2a9b2d"]),
            spec(&["215f2719"], &["d24f649d"], &[]),
            spec(&["215f2719"], &["d24f649d="], &[]),
            spec(&["215f2719"], &[], &["PLACEHOLDER_x"]),
        ] {
            assert!(bad.check().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn toml_table() {
        let s: SynthEffect = toml::from_str("start = [\"215f2719\"]\nthrough = true\npick = [\"d24f649d=Keira to KM\"]\nstop = [\"4c2a9b2d\"]\n").unwrap();
        assert_eq!(s, spec(&["215f2719"], &["d24f649d=Keira to KM"], &["4c2a9b2d"]));
        let s: SynthEffect = toml::from_str("start = [\"2da9cd80\"]\npick = [\"70d40ee9=NoFight\"]\nscene_pick = [\"ed477b93=sq102_go _to_km_letho\"]\n").unwrap();
        assert_eq!(s.scene_pick, vec!["ed477b93=sq102_go _to_km_letho".to_owned()]);
        assert!(toml::from_str::<SynthEffect>("starts = [\"215f2719\"]").is_err(), "unknown keys are rejected");
    }
}
