//! Dialogue scenes (`.w2scene`, `CStoryScene`): the flow between a scene input and its outputs, and
//! the facts set on the way. A quest `CQuestSceneBlock` / `CQuestInteractionDialogBlock` plays one of
//! these; its output sockets are the scene's `CStorySceneOutput` names. Facts set inside a scene do
//! not appear in the quest graph (`q105_betray_spirit`, `q402_invited_letho_to_km`): this module
//! attributes them to the path the player took.
//!
//! Flow elements (every export is a node; links are export pointers):
//! * `CStorySceneInput { inputName, nextLinkElement }`, `CStorySceneOutput { name }`;
//! * `CStorySceneSection` (also cutscene / video sections) `{ sectionName, nextLinkElement | choice }`:
//!   its tail (bytes after the property list) is `u32 count` then `count` events of
//!   `{ u32 size (from the size field itself), u16 class, u8 0, property list [, class data] }`;
//!   `CStorySceneAddFactEvent { factId, expireTime }`
//!   and `CStorySceneRemoveFactEvent` set / remove a fact when the section plays (the ritual scene's
//!   `q105_betray_spirit` in section `#90`, on the way to `q105_betrayed_out`);
//! * `CStorySceneChoice { choiceLines }` -> `CStorySceneChoiceLine { nextLinkElement, memo }` with
//!   `CFactsDBChoiceMemo { factID, value }` set when the line is picked;
//! * `CStorySceneScript { functionName, nextLinkElement }` whose parameters are a second property list
//!   in the tail: `AddFact_S { factName, value, validFor }` / `RemoveFact_S { factName }` (Letho:
//!   `q402_invited_letho_to_km += 1` and `q402_told_letho_to_go_elsewhere += 1`, both into the one output);
//! * `CStorySceneFlowCondition { questCondition, trueLink, falseLink }` -> `CStorySceneLinkElement
//!   { nextLinkElement }`; `CStorySceneRandomizer { outputs }`.
//!
//! [`Scene::outcome`] prunes the flow with what is known (the fact conditions evaluate against the
//! save, picked choice lines are followed alone), keeps the paths from the input to the chosen output
//! and sorts the facts into *certain* (on every remaining path: dominators of the output) and *maybe*
//! (on some path only: an in-scene choice without a pick). The walker sets the certain ones and
//! refuses to evaluate a condition on a maybe one.
use std::collections::{HashSet, VecDeque};

use crate::cr2w::{self, Cr2w, Prop, Val};
use crate::quest::{cond_of, Cond, Error};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactOp {
    Add { id: String, value: i64 },
    Remove { id: String },
}

impl FactOp {
    pub fn id(&self) -> &str {
        match self {
            FactOp::Add { id, .. } | FactOp::Remove { id } => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    Next(usize),
    /// A flow condition's branch.
    Cond { to: usize, when: bool },
    /// A choice line (export index of the line) and where it leads.
    Choice { line: usize, to: usize },
}

impl Link {
    pub fn to(&self) -> usize {
        match self {
            Link::Next(t) | Link::Cond { to: t, .. } | Link::Choice { to: t, .. } => *t,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneNode {
    pub class: String,
    /// `sectionName` / `inputName` / output `name` / a script's function.
    pub name: String,
    pub links: Vec<Link>,
    pub facts: Vec<FactOp>,
    pub cond: Option<Cond>,
    /// Section events decoded from the tail (count, classes), for `scene-info`.
    pub events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    pub file: String,
    pub nodes: Vec<SceneNode>,
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
}

/// What a walk through the scene does to the facts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    /// Set on every path from the input to the output.
    pub certain: Vec<FactOp>,
    /// Set on some paths only: `(fact, the section / line that sets it)`. A scene pick naming the
    /// section it leads through resolves it.
    pub maybe: Vec<(FactOp, String)>,
    /// Section names of unresolved choices on the way (what a pick may name).
    pub choices: Vec<Vec<String>>,
}

fn ptr(v: Option<&Val>) -> Option<usize> {
    match v {
        Some(Val::Ptr(p)) => *p,
        _ => None,
    }
}

fn str_of(props: &[Prop], name: &str) -> Option<String> {
    match cr2w::prop_of(props, name) {
        Some(Val::Str(s) | Val::Name(s)) => Some(s.trim().to_owned()),
        _ => None,
    }
}

/// The events serialized in a section's tail: `u32 count`, then per event `u32 size, u16 class
/// name, u8 0, property list [, class-specific data]` where `size` counts from the size field itself
/// (read on `q105_12_perform_ritual` section `#90`: 48 events, sizes 650 / 125 / .. / 113 for the
/// AddFact event, each ending where the next begins). Decoding stops at the first event that is not
/// well formed; the list is followed by other section data.
fn section_events(c: &Cr2w, tail: &[u8]) -> Vec<(String, Vec<Prop>)> {
    let mut out = Vec::new();
    if tail.len() < 4 {
        return out;
    }
    let count = u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]) as usize;
    if count == 0 || count > 4096 {
        return out;
    }
    let mut o = 4;
    for _ in 0..count {
        let Some(h) = tail.get(o..o + 7) else { break };
        let size = u32::from_le_bytes([h[0], h[1], h[2], h[3]]) as usize;
        let Some(class) = c.names.get(u16::from_le_bytes([h[4], h[5]]) as usize).cloned() else { break };
        if h[6] != 0 || size < 9 || o + size > tail.len() || !class.starts_with("CStoryScene") {
            break;
        }
        let body = &tail[o + 7..o + size];
        let Some(props) = c.props_in(body) else { break };
        out.push((class, props));
        o += size;
    }
    out
}

fn fact_of(props: &[Prop], class: &str) -> Option<FactOp> {
    let id = str_of(props, "factId").or_else(|| str_of(props, "factID")).or_else(|| str_of(props, "factName"))?;
    if id.is_empty() {
        return None;
    }
    let value = match cr2w::prop_of(props, "value") {
        Some(Val::Int(v)) => *v,
        _ => 1,
    };
    if class.contains("Remove") {
        Some(FactOp::Remove { id })
    } else {
        Some(FactOp::Add { id, value })
    }
}

/// Parse a `.w2scene` file.
pub fn parse(data: &[u8], name: &str) -> Result<Scene, Error> {
    let c = cr2w::parse(data, name)?;
    let mut s = Scene { file: name.to_owned(), nodes: Vec::with_capacity(c.exports.len()), inputs: Vec::new(), outputs: Vec::new() };
    for (i, e) in c.exports.iter().enumerate() {
        let mut n = SceneNode { class: e.class.clone(), ..Default::default() };
        let next = ptr(e.prop("nextLinkElement"));
        match e.class.as_str() {
            "CStorySceneInput" => {
                n.name = e.str_prop("inputName").unwrap_or("").trim().to_owned();
                n.links.extend(next.map(Link::Next));
                s.inputs.push(i);
            }
            "CStorySceneOutput" => {
                n.name = e.str_prop("name").unwrap_or("").trim().to_owned();
                s.outputs.push(i);
            }
            "CStorySceneSection" | "CStorySceneCutsceneSection" | "CStorySceneVideoSection" => {
                n.name = e.str_prop("sectionName").unwrap_or("").trim().to_owned();
                if let Some(choice) = ptr(e.prop("choice")) {
                    if let Some(Val::Ptrs(lines)) = c.exports.get(choice).and_then(|x| x.prop("choiceLines")) {
                        for l in lines.iter().flatten() {
                            n.links.push(Link::Choice { line: *l, to: *l });
                        }
                    }
                } else {
                    n.links.extend(next.map(Link::Next));
                }
                for (class, props) in section_events(&c, &e.tail) {
                    n.events.push(class.clone());
                    if class.ends_with("AddFactEvent") || class.ends_with("RemoveFactEvent") {
                        n.facts.extend(fact_of(&props, &class));
                    }
                }
            }
            "CStorySceneChoiceLine" => {
                n.links.extend(next.map(Link::Next));
                if let Some(Val::Ptrs(memos)) = e.prop("memo") {
                    for m in memos.iter().flatten().filter_map(|m| c.exports.get(*m)) {
                        if m.class == "CFactsDBChoiceMemo" {
                            n.facts.extend(fact_of(&m.props, "Add"));
                        }
                    }
                }
                // The line's name: the section it leads to (choice line texts are localized ids).
                n.name = next.and_then(|t| c.exports.get(t)).and_then(|t| t.str_prop("sectionName")).unwrap_or("").trim().to_owned();
            }
            "CStorySceneScript" => {
                n.name = e.str_prop("functionName").unwrap_or("").to_owned();
                n.links.extend(next.map(Link::Next));
                if matches!(n.name.as_str(), "AddFact_S" | "RemoveFact_S") {
                    if let Some(params) = c.props_in(&e.tail) {
                        n.facts.extend(fact_of(&params, &n.name));
                    }
                }
            }
            "CStorySceneFlowCondition" => {
                n.cond = ptr(e.prop("questCondition")).map(|q| cond_of(&c, q, 0));
                n.links.extend(ptr(e.prop("trueLink")).map(|t| Link::Cond { to: t, when: true }));
                n.links.extend(ptr(e.prop("falseLink")).map(|t| Link::Cond { to: t, when: false }));
            }
            "CStorySceneRandomizer" => {
                if let Some(Val::Ptrs(outs)) = e.prop("outputs") {
                    n.links.extend(outs.iter().flatten().map(|t| Link::Next(*t)));
                }
            }
            _ => {
                // Plain link elements and anything else with a next pointer.
                n.links.extend(next.map(Link::Next));
            }
        }
        s.nodes.push(n);
    }
    // A choice line's name is the first named section it leads to (through link elements, scripts, ..).
    for i in 0..s.nodes.len() {
        if s.nodes[i].class != "CStorySceneChoiceLine" || !s.nodes[i].name.is_empty() {
            continue;
        }
        let mut at = s.nodes[i].links.first().map(Link::to);
        let mut name = String::new();
        for _ in 0..8 {
            let Some(x) = at.and_then(|x| s.nodes.get(x)) else { break };
            if !x.name.is_empty() && x.class != "CStorySceneScript" {
                name = x.name.clone();
                break;
            }
            at = x.links.first().map(Link::to);
        }
        s.nodes[i].name = name;
    }
    Ok(s)
}

impl Scene {
    /// The input node a quest socket name activates: the input with that name, else the only one.
    pub fn input(&self, socket: &str) -> Option<usize> {
        let want = socket.trim();
        self.inputs.iter().copied().find(|i| self.nodes[*i].name == want).or_else(|| if self.inputs.len() == 1 { self.inputs.first().copied() } else { None }).or_else(|| {
            // The quest side names a default input `In` / `Input`; take the unnamed input if there is one.
            self.inputs.iter().copied().find(|i| self.nodes[*i].name.is_empty())
        })
    }

    /// The output node a quest output socket stands for.
    pub fn output(&self, socket: &str) -> Option<usize> {
        let want = socket.trim();
        self.outputs
            .iter()
            .copied()
            .find(|o| self.nodes[*o].name == want)
            .or_else(|| if self.outputs.len() == 1 { self.outputs.first().copied() } else { None })
            // The quest side names the default output `Output` / `Out` (context dialogs: `sq301_00a_triss_about_note`
            // has outputs "" and "triss_refused", its block sockets "Output" and "triss_refused"); take the unnamed one.
            .or_else(|| if matches!(want, "Output" | "Out") { self.outputs.iter().copied().find(|o| self.nodes[*o].name.is_empty()) } else { None })
    }

    /// Facts set when the scene is played from `input` to `output`. `picks` names sections (the
    /// targets of choice lines) to follow alone; `eval` evaluates a flow condition (`None` = unknown).
    pub fn outcome(&self, input: &str, output: &str, picks: &[String], eval: &dyn Fn(&Cond) -> Option<bool>) -> Result<Outcome, Error> {
        let start = self.input(input).ok_or_else(|| Error::Other(format!("{}: no scene input for socket {input:?}", self.file)))?;
        let end = self.output(output).ok_or_else(|| Error::Other(format!("{}: no scene output named {output:?}", self.file)))?;
        let n = self.nodes.len();
        // Pruned adjacency.
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut choices: Vec<Vec<String>> = Vec::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let mut targets: Vec<usize> = Vec::new();
            if let Some(c) = &node.cond {
                match eval(c) {
                    Some(b) => targets.extend(node.links.iter().filter_map(|l| match l {
                        Link::Cond { to, when } if *when == b => Some(*to),
                        _ => None,
                    })),
                    None => targets.extend(node.links.iter().map(Link::to)),
                }
            } else if node.links.iter().any(|l| matches!(l, Link::Choice { .. })) {
                let lines: Vec<usize> = node.links.iter().filter_map(|l| if let Link::Choice { line, .. } = l { Some(*line) } else { None }).collect();
                let picked: Vec<usize> = lines.iter().copied().filter(|l| picks.iter().any(|p| p.trim() == self.nodes[*l].name)).collect();
                if picked.is_empty() {
                    choices.push(lines.iter().map(|l| self.nodes[*l].name.clone()).collect());
                    targets.extend(lines);
                } else {
                    targets.extend(picked);
                }
            } else {
                targets.extend(node.links.iter().map(Link::to));
            }
            adj[i] = targets.into_iter().filter(|t| *t < n).collect();
        }
        let reach = |from: usize, skip: Option<usize>| -> HashSet<usize> {
            let mut seen = HashSet::new();
            let mut q = VecDeque::new();
            if Some(from) != skip {
                seen.insert(from);
                q.push_back(from);
            }
            while let Some(x) = q.pop_front() {
                for &t in &adj[x] {
                    if Some(t) != skip && seen.insert(t) {
                        q.push_back(t);
                    }
                }
            }
            seen
        };
        let forward = reach(start, None);
        let end = if forward.contains(&end) {
            end
        } else {
            // A quest socket can be stale (renamed output) or the input may lead to another output
            // only (`q109_01_geralt_on_keira`: input `light` -> output `seen light`): the one
            // reachable output is what plays.
            let reachable: Vec<usize> = self.outputs.iter().copied().filter(|o| forward.contains(o)).collect();
            match reachable.as_slice() {
                [one] => *one,
                _ => return Err(Error::Other(format!("{}: output {output:?} is not reachable from input {input:?} with these picks ({} outputs are)", self.file, reachable.len()))),
            }
        };
        // Nodes on some input -> output path: forward-reachable and backward-reachable from the output.
        let mut rev: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, ts) in adj.iter().enumerate() {
            for &t in ts {
                rev[t].push(i);
            }
        }
        let mut backward = HashSet::new();
        let mut q = VecDeque::from([end]);
        backward.insert(end);
        while let Some(x) = q.pop_front() {
            for &p in &rev[x] {
                if backward.insert(p) {
                    q.push_back(p);
                }
            }
        }
        let on_path: Vec<usize> = (0..n).filter(|i| forward.contains(i) && backward.contains(i)).collect();
        // Only choices on the kept paths matter.
        let choices: Vec<Vec<String>> = choices.into_iter().filter(|c| c.iter().any(|name| on_path.iter().any(|i| self.nodes[*i].name == *name && self.nodes[*i].class == "CStorySceneChoiceLine"))).collect();
        let mut out = Outcome { choices, ..Default::default() };
        // Order by distance from the input (BFS order) so facts come out in play order.
        let mut order: Vec<usize> = Vec::new();
        let mut seen = HashSet::new();
        let mut q = VecDeque::from([start]);
        seen.insert(start);
        while let Some(x) = q.pop_front() {
            order.push(x);
            for &t in &adj[x] {
                if seen.insert(t) {
                    q.push_back(t);
                }
            }
        }
        for i in order {
            if !on_path.contains(&i) || self.nodes[i].facts.is_empty() {
                continue;
            }
            let certain = i == start || i == end || !reach(start, Some(i)).contains(&end);
            for f in &self.nodes[i].facts {
                if certain {
                    out.certain.push(f.clone());
                } else {
                    out.maybe.push((f.clone(), self.nodes[i].name.clone()));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(class: &str, name: &str, links: Vec<Link>, facts: Vec<FactOp>) -> SceneNode {
        SceneNode { class: class.into(), name: name.into(), links, facts, cond: None, events: Vec::new() }
    }

    fn add(id: &str) -> FactOp {
        FactOp::Add { id: id.into(), value: 1 }
    }

    /// 0 input -> 1 section(choice: lines 2, 3) ; 2 -> 4 section "km" (fact km) -> 6 ; 3 -> 5 section "away" (fact away) -> 6 ; 6 script (fact both) -> 7 output
    fn letho_like() -> Scene {
        Scene {
            file: "t.w2scene".into(),
            nodes: vec![
                node("CStorySceneInput", "", vec![Link::Next(1)], vec![]),
                node("CStorySceneSection", "choice", vec![Link::Choice { line: 2, to: 2 }, Link::Choice { line: 3, to: 3 }], vec![]),
                node("CStorySceneChoiceLine", "km", vec![Link::Next(4)], vec![]),
                node("CStorySceneChoiceLine", "away", vec![Link::Next(5)], vec![]),
                node("CStorySceneSection", "km", vec![Link::Next(6)], vec![add("invited")]),
                node("CStorySceneSection", "away", vec![Link::Next(6)], vec![add("elsewhere")]),
                node("CStorySceneScript", "AddFact_S", vec![Link::Next(7)], vec![add("both")]),
                node("CStorySceneOutput", "", vec![], vec![]),
            ],
            inputs: vec![0],
            outputs: vec![7],
        }
    }

    #[test]
    fn an_unpicked_choice_leaves_its_facts_maybe_and_the_common_ones_certain() {
        let s = letho_like();
        let o = s.outcome("In", "Out", &[], &|_| None).unwrap();
        assert_eq!(o.certain, vec![add("both")]);
        assert_eq!(o.maybe.len(), 2);
        assert_eq!(o.choices, vec![vec!["km".to_owned(), "away".to_owned()]]);
    }

    #[test]
    fn a_pick_makes_the_lines_facts_certain() {
        let s = letho_like();
        let o = s.outcome("", "", &["km".into()], &|_| None).unwrap();
        assert_eq!(o.certain, vec![add("invited"), add("both")]);
        assert!(o.maybe.is_empty() && o.choices.is_empty());
    }

    #[test]
    fn the_default_output_socket_names_the_unnamed_scene_output() {
        // Two outputs, "" and "refused" (sq301_00a_triss_about_note): the block's `Output` socket is the unnamed one.
        let mut s = letho_like();
        s.nodes.push(node("CStorySceneOutput", "refused", vec![], vec![]));
        s.outputs.push(8);
        assert_eq!(s.output("Output"), Some(7));
        assert_eq!(s.output("Out"), Some(7));
        assert_eq!(s.output("refused"), Some(8));
        assert_eq!(s.output("other"), None);
    }

    #[test]
    fn a_flow_condition_prunes_with_the_evaluation() {
        // 0 input -> 1 cond (true -> 2 fact a -> 4 out, false -> 3 fact b -> 4 out)
        let mut s = Scene {
            file: "c.w2scene".into(),
            nodes: vec![
                node("CStorySceneInput", "", vec![Link::Next(1)], vec![]),
                node("CStorySceneFlowCondition", "", vec![Link::Cond { to: 2, when: true }, Link::Cond { to: 3, when: false }], vec![]),
                node("CStorySceneSection", "yes", vec![Link::Next(4)], vec![add("a")]),
                node("CStorySceneSection", "no", vec![Link::Next(4)], vec![add("b")]),
                node("CStorySceneOutput", "x", vec![], vec![]),
            ],
            inputs: vec![0],
            outputs: vec![4],
        };
        s.nodes[1].cond = Some(Cond::Fact { id: "f".into(), cmp: "CF_Greater".into(), value: 0 });
        let o = s.outcome("", "x", &[], &|_| Some(false)).unwrap();
        assert_eq!(o.certain, vec![add("b")]);
        let o = s.outcome("", "x", &[], &|_| None).unwrap();
        assert!(o.certain.is_empty() && o.maybe.len() == 2);
    }
}
