//! The typed quest-graph model: every `.w2phase` / `.w2quest` file is a tree of `CQuestGraph`s
//! whose blocks (`CQuest*Block`, `CJournal*Block`) connect output sockets to other blocks' input
//! sockets (`cachedConnections`). A `CQuestPhaseBlock` owns an embedded graph or imports another
//! file; its graph's `CQuestPhaseInputBlock` / `CQuestPhaseOutputBlock` are its sockets.
//!
//! The save's `questSystem` mirrors this: a `questThread[GUID]` wrapper is a phase block whose
//! graph is running, a `questBlock[GUID]` is a block waiting inside it. Every GUID of a 5.01 save's
//! frontier is a block of these files (speed-up study, 1,023 of 1,023 and 2,793 of 2,793).
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::cr2w::{self, Cr2w, Export, Val};
use crate::resources::{self, Community};
use crate::scene::{self, Scene};

#[derive(Debug)]
pub enum Error {
    Io(String, String),
    Format(String),
    Save(w3save::Error),
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(p, e) => write!(f, "{p}: {e}"),
            Error::Format(s) | Error::Other(s) => write!(f, "{s}"),
            Error::Save(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<w3save::Error> for Error {
    fn from(e: w3save::Error) -> Self {
        Error::Save(e)
    }
}

pub type Guid = [u8; 16];

pub fn guid_hex(g: &Guid) -> String {
    cr2w::hex(g)
}

pub fn short(g: &Guid) -> String {
    cr2w::hex(&g[..4])
}

pub fn parse_guid(s: &str) -> Result<Guid, Error> {
    let s = s.trim();
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Other(format!("bad GUID {s:?} (32 hex digits)")));
    }
    let mut g = [0u8; 16];
    for (i, b) in g.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|e| Error::Other(e.to_string()))?;
    }
    Ok(g)
}

/// One step of a journal path (`CJournalPath` chain): the GUID and, on the steps that open a
/// `.journal` file, that file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathStep {
    pub guid: Guid,
    pub resource: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    /// `CQuestFactsDBCondition`: `sum(fact) <cmp> value`.
    Fact { id: String, cmp: String, value: i64 },
    /// `CQuestLogicOperationCondition` over inner conditions.
    Logic { op: String, inner: Vec<Cond> },
    /// `CQuestRealtimeDelayCondition` (`howLong.m_seconds`) / `CQuestHiResRealtimeDelayCondition`
    /// (`miliseconds` or `seconds`): a real-time timer.
    Delay { ms: u64 },
    /// Anything the walker cannot evaluate (triggers, actors, timers, ...): the class name.
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpawnAction {
    Activate { spawnset: String, phase: String },
    Deactivate { spawnset: String },
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockKind {
    Fact { id: String, value: i64 },
    /// `CJournalQuestBlock` (`quest = true`, path from `questEntry`) or `CJournalBlock`
    /// (character / glossary entries, path from `entry`).
    Journal { quest: bool, path: Vec<PathStep> },
    /// Map-pin state, tracking, objective counters: instant and not modelled.
    JournalOther,
    Scene { file: String },
    Interaction { file: String, tags: Vec<String> },
    Pause { conds: Vec<Cond> },
    Condition { cond: Option<Cond> },
    Phase { embedded: Option<usize>, import: Option<String> },
    PhaseInput { socket: String },
    PhaseOutput { socket: String },
    StoryPhase { actions: Vec<SpawnAction> },
    Reward { name: String },
    /// `CQuestScriptBlock`: a call into `quest_function.ws`; `params` = `(name, value)` as text.
    Script { function: String, params: Vec<(String, String)> },
    /// Instant blocks without state the walker needs (layers, checkpoints, teleports, ...).
    Instant,
    /// Blocks that wait for the world (scripted actions, combat, animations, minigames, ...).
    Wait,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conn {
    pub socket: String,
    /// Index into [`Library::blocks`].
    pub target: usize,
    pub input: String,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub guid: Guid,
    pub class: String,
    /// Normalized file name (`quests/part_1/part_1.w2phase`).
    pub file: String,
    pub export: usize,
    /// `name` or `comment` of the block, for reports.
    pub label: String,
    pub outs: Vec<Conn>,
    /// The phase block whose embedded graph holds this block; `None` for a file's root graph.
    pub owner: Option<usize>,
    /// Index of the `CQuestGraph` export that holds this block (for phase input lookups).
    pub graph: usize,
    pub kind: BlockKind,
}

impl Block {
    pub fn short(&self) -> String {
        short(&self.guid)
    }

    pub fn is_wait(&self) -> bool {
        matches!(self.kind, BlockKind::Scene { .. } | BlockKind::Interaction { .. } | BlockKind::Pause { .. } | BlockKind::Wait)
    }

    /// Distinct output sockets in connection order.
    pub fn sockets(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for c in &self.outs {
            if !v.contains(&c.socket.as_str()) {
                v.push(&c.socket);
            }
        }
        v
    }
}

/// All quest graphs of one install or folder, indexed by GUID.
#[derive(Debug, Default)]
pub struct Library {
    pub blocks: Vec<Block>,
    pub files: Vec<String>,
    by_guid: HashMap<Guid, usize>,
    /// First 8 hex digits -> block indices (the short keys the save paths use).
    by_short: HashMap<String, Vec<usize>>,
    /// file -> `(graph export index, phase block owner)` pairs, root graph first.
    root_graph: HashMap<String, usize>,
    /// file -> phase blocks in other files that import it.
    importers: HashMap<String, Vec<usize>>,
    /// `(file, graph export)` -> blocks in that graph.
    graph_blocks: HashMap<(String, usize), Vec<usize>>,
    /// Journal entry GUID (last step) -> first block that activates it.
    pub journal_by_last: HashMap<Guid, usize>,
    /// Side resources (`.w2scene`, `.w2comm`, `.journal`) by normalized name, read on demand.
    resources: HashMap<String, ResSource>,
    scenes: Mutex<HashMap<String, Arc<Scene>>>,
    comms: Mutex<HashMap<String, Arc<Community>>>,
    objectives: Mutex<Option<Arc<HashSet<Guid>>>>,
    /// The install's quest list with its journal blocks (`crate::complete::index`), built on first use.
    pub(crate) quest_index: std::sync::OnceLock<Arc<crate::complete::QuestIndex>>,
    /// The install folder this library was read from (the game's scripts are read from it by the consequence check).
    pub(crate) install: Option<std::path::PathBuf>,
    /// Who reads which fact, across the install (`crate::consequence::readers`), built on first use.
    pub(crate) fact_readers: std::sync::OnceLock<Arc<crate::consequence::FactReaders>>,
}

/// Where a side resource comes from.
#[derive(Debug, Clone)]
enum ResSource {
    File(std::path::PathBuf),
    Bundle { bundle: std::path::PathBuf, entry: crate::bundle::Entry },
}

/// Is this a side resource the synthesis reads (`.w2scene`, `.w2comm`, `.journal`)?
pub fn is_side_resource(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".w2scene") || n.ends_with(".w2comm") || n.ends_with(".journal")
}

/// `quests\part_1\x.w2phase` and `QUESTS/part_1/x.w2phase` are the same file.
/// Graph files the game ships for demos and tests, never part of a playthrough.
pub fn is_demo_file(name: &str) -> bool {
    let n = normalize(name);
    n.starts_with("quests/demo_files/") || n.contains("/demo_files/")
}

pub fn normalize(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

impl Library {
    /// Load every `.w2phase` / `.w2quest` under a folder of extracted files (paths relative to the
    /// folder are the resource names).
    pub fn load_dir(dir: &Path) -> Result<Library, Error> {
        Library::load_dirs(std::slice::from_ref(&dir.to_path_buf()))
    }

    /// Load several folders into one library (later folders do not override earlier files).
    pub fn load_dirs(dirs: &[std::path::PathBuf]) -> Result<Library, Error> {
        let mut lib = Library::default();
        for dir in dirs {
        let mut stack = vec![dir.to_path_buf()];
        let mut files = Vec::new();
        while let Some(d) = stack.pop() {
            let rd = std::fs::read_dir(&d).map_err(|e| Error::Io(d.display().to_string(), e.to_string()))?;
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.to_str().is_some_and(crate::bundle::is_quest_graph) {
                    files.push(p);
                } else if p.to_str().is_some_and(is_side_resource) {
                    let rel = normalize(&p.strip_prefix(dir).unwrap_or(&p).to_string_lossy());
                    lib.resources.entry(rel).or_insert(ResSource::File(p));
                }
            }
        }
        files.sort();
        for p in files {
            let rel = p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().to_string();
            if lib.root_graph.contains_key(&normalize(&rel)) {
                continue;
            }
            let d = std::fs::read(&p).map_err(|e| Error::Io(p.display().to_string(), e.to_string()))?;
            lib.add_file(&normalize(&rel), &d)?;
        }
        }
        lib.link();
        Ok(lib)
    }

    /// Load every quest graph of a game install (read-only; bundles are read in place).
    pub fn load_install(install: &Path) -> Result<Library, Error> {
        let mut lib = Library { install: Some(install.to_path_buf()), ..Library::default() };
        for b in crate::bundle::bundles_of_install(install) {
            for e in crate::bundle::toc(&b)? {
                if is_side_resource(&e.name) {
                    lib.resources.entry(normalize(&e.name)).or_insert_with(|| ResSource::Bundle { bundle: b.clone(), entry: e.clone() });
                    continue;
                }
                if !crate::bundle::is_quest_graph(&e.name) {
                    continue;
                }
                let name = normalize(&e.name);
                if lib.root_graph.contains_key(&name) {
                    continue; // a later bundle (patch) does not override here: first wins, as the bundle order lists content0 first
                }
                let d = crate::bundle::extract(&b, &e)?;
                lib.add_file(&name, &d)?;
            }
        }
        lib.link();
        Ok(lib)
    }

    /// Add one parsed file. Blocks get indices; connections are resolved within the file at once,
    /// imports between files by [`Library::link`].
    pub fn add_file(&mut self, name: &str, data: &[u8]) -> Result<(), Error> {
        let c = cr2w::parse(data, name)?;
        self.add_parsed(name, &c);
        Ok(())
    }

    fn add_parsed(&mut self, name: &str, c: &Cr2w) {
        let base = self.blocks.len();
        let mut export_to_block: HashMap<usize, usize> = HashMap::new();
        // Pass 1: every export with a `guid` whose class ends in `Block` is a graph block.
        for (i, e) in c.exports.iter().enumerate() {
            if !e.class.ends_with("Block") || e.class == "CQuestGraphBlock" {
                continue;
            }
            let Some(guid) = e.guid() else { continue };
            let Some(graph) = e.parent else { continue };
            export_to_block.insert(i, self.blocks.len());
            self.blocks.push(Block {
                guid,
                class: e.class.clone(),
                file: name.to_owned(),
                export: i,
                label: e.str_prop("name").or_else(|| e.str_prop("comment")).or_else(|| e.str_prop("caption")).unwrap_or("").to_owned(),
                outs: Vec::new(),
                owner: None,
                graph,
                kind: BlockKind::Instant,
            });
        }
        // Root graph: the `CQuestGraph` whose parent is the `CQuestPhase` / `CQuest` root export.
        if let Some(root) = c.exports.iter().position(|e| e.class == "CQuestGraph" && e.parent.is_some_and(|p| c.exports[p].parent.is_none())) {
            self.root_graph.insert(name.to_owned(), root);
        }
        // Pass 2: owners, kinds, connections.
        for (ei, bi) in export_to_block.clone() {
            let e = &c.exports[ei];
            let owner = c.exports[e.parent.unwrap_or(ei)].parent.and_then(|p| export_to_block.get(&p).copied());
            let outs = connections(e, &export_to_block);
            let kind = kind_of(c, e, &export_to_block);
            let b = &mut self.blocks[bi];
            b.owner = owner;
            b.outs = outs;
            b.kind = kind;
        }
        for bi in base..self.blocks.len() {
            let b = &self.blocks[bi];
            self.by_guid.insert(b.guid, bi);
            self.by_short.entry(b.short()).or_default().push(bi);
            self.graph_blocks.entry((name.to_owned(), b.graph)).or_default().push(bi);
            if let BlockKind::Phase { import: Some(f), .. } = &b.kind {
                // Demo / test graphs shipped with the game (`quests/demo_files/hands_on_q206_berserkers`) import real
                // quest files but never run: they must not count as importers (the walker left q206 through one into q208).
                if !is_demo_file(name) {
                    self.importers.entry(f.clone()).or_default().push(bi);
                }
            }
            if let BlockKind::Journal { path, .. } = &b.kind {
                if let Some(last) = path.last() {
                    self.journal_by_last.entry(last.guid).or_insert(bi);
                }
            }
        }
        self.files.push(name.to_owned());
    }

    fn link(&mut self) {
        for v in self.importers.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
    }

    /// Rebuild the GUID and graph indices from `blocks` (for libraries built by hand in tests).
    #[cfg(test)]
    pub(crate) fn reindex(&mut self) {
        self.by_guid.clear();
        self.by_short.clear();
        self.graph_blocks.clear();
        self.importers.clear();
        for (i, b) in self.blocks.iter().enumerate() {
            self.by_guid.insert(b.guid, i);
            self.by_short.entry(b.short()).or_default().push(i);
            self.graph_blocks.entry((b.file.clone(), b.graph)).or_default().push(i);
            if let BlockKind::Phase { import: Some(f), .. } = &b.kind {
                if is_demo_file(&b.file) {
                    continue;
                }
                self.importers.entry(normalize(f)).or_default().push(i);
            }
        }
    }

    /// Declare `file`'s root graph index (for libraries built by hand in tests: the blocks of
    /// `file` with that `graph` are what its importers enter).
    #[cfg(test)]
    pub(crate) fn set_root_graph(&mut self, file: &str, graph: usize) {
        self.root_graph.insert(normalize(file), graph);
    }

    pub fn get(&self, guid: &Guid) -> Option<usize> {
        self.by_guid.get(guid).copied()
    }

    /// Does the library know any side resource of this extension (`".w2scene"`, `".w2comm"`,
    /// `".journal"`)? A folder of graphs alone has none; then scene facts, spawn sets and counters
    /// cannot be derived and the walker says so.
    pub fn has_resources(&self, ext: &str) -> bool {
        self.resources.keys().any(|k| k.ends_with(ext))
    }

    /// The side resources of this extension (`".journal"`, ...), sorted by name.
    pub fn resource_names(&self, ext: &str) -> Vec<String> {
        let mut v: Vec<String> = self.resources.keys().filter(|k| k.ends_with(ext)).cloned().collect();
        v.sort();
        v
    }

    /// The bytes of a side resource (`.w2scene`, `.w2comm`, `.journal`) by its normalized name.
    pub fn read_resource(&self, name: &str) -> Result<Vec<u8>, Error> {
        self.resource_bytes(name)
    }

    fn resource_bytes(&self, name: &str) -> Result<Vec<u8>, Error> {
        let n = normalize(name);
        match self.resources.get(&n) {
            Some(ResSource::File(p)) => std::fs::read(p).map_err(|e| Error::Io(p.display().to_string(), e.to_string())),
            Some(ResSource::Bundle { bundle, entry }) => crate::bundle::extract(bundle, entry),
            None => Err(Error::Other(format!("{n}: not among the loaded resources"))),
        }
    }

    /// A dialogue scene, parsed on first use.
    pub fn scene(&self, file: &str) -> Result<Arc<Scene>, Error> {
        let n = normalize(file);
        if let Some(s) = self.scenes.lock().expect("scene cache").get(&n) {
            return Ok(s.clone());
        }
        let s = Arc::new(scene::parse(&self.resource_bytes(&n)?, &n)?);
        self.scenes.lock().expect("scene cache").insert(n, s.clone());
        Ok(s)
    }

    /// A spawn set, parsed on first use.
    pub fn community(&self, file: &str) -> Result<Arc<Community>, Error> {
        let n = normalize(file);
        if let Some(c) = self.comms.lock().expect("community cache").get(&n) {
            return Ok(c.clone());
        }
        let c = Arc::new(resources::parse_community(&self.resource_bytes(&n)?, &n)?);
        self.comms.lock().expect("community cache").insert(n, c.clone());
        Ok(c)
    }

    /// The GUIDs of every quest objective (`CJournalQuestObjective`) of every `.journal` resource,
    /// read once. Empty when the library has no journal resources.
    pub fn objectives(&self) -> Result<Arc<HashSet<Guid>>, Error> {
        if let Some(o) = self.objectives.lock().expect("objective cache").as_ref() {
            return Ok(o.clone());
        }
        let mut set = HashSet::new();
        let names: Vec<String> = self.resources.keys().filter(|k| k.ends_with(".journal")).cloned().collect();
        for n in names {
            set.extend(resources::journal_objectives(&self.resource_bytes(&n)?, &n)?);
        }
        let set = Arc::new(set);
        *self.objectives.lock().expect("objective cache") = Some(set.clone());
        Ok(set)
    }

    /// Blocks whose GUID starts with these 8 hex digits (the key the save paths use).
    pub fn by_short(&self, key: &str) -> &[usize] {
        self.by_short.get(&key.to_ascii_lowercase()).map_or(&[], Vec::as_slice)
    }

    /// Resolve `xxxxxxxx` (short) or a full GUID to exactly one block.
    pub fn resolve(&self, key: &str) -> Result<usize, Error> {
        if key.len() == 32 {
            return self.get(&parse_guid(key)?).ok_or_else(|| Error::Other(format!("block {key} is not in the quest graphs")));
        }
        match self.by_short(key) {
            [one] => Ok(*one),
            [] => Err(Error::Other(format!("block {key} is not in the quest graphs"))),
            many => Err(Error::Other(format!("block {key} is ambiguous ({} blocks); give the full GUID", many.len()))),
        }
    }

    /// The blocks of a phase block's graph: its embedded graph, or the root graph of the file it imports.
    pub fn inner_blocks(&self, phase: usize) -> &[usize] {
        let b = &self.blocks[phase];
        match &b.kind {
            BlockKind::Phase { embedded: Some(g), .. } => self.graph_blocks.get(&(b.file.clone(), *g)).map_or(&[], Vec::as_slice),
            BlockKind::Phase { import: Some(f), .. } => self.root_blocks(f),
            _ => &[],
        }
    }

    /// The blocks of a file's root graph.
    pub fn root_blocks(&self, file: &str) -> &[usize] {
        let f = normalize(file);
        self.root_graph.get(&f).and_then(|g| self.graph_blocks.get(&(f.clone(), *g))).map_or(&[], Vec::as_slice)
    }

    /// The input sockets of a block that something in its graph connects to, in connection order
    /// (what a `CQuestAndBlock` must see activated before it fires: its `In 1`, `In 2`, ..).
    pub fn input_sockets(&self, block: usize) -> Vec<String> {
        let b = &self.blocks[block];
        let mut v: Vec<String> = Vec::new();
        for i in self.graph_blocks.get(&(b.file.clone(), b.graph)).map_or(&[][..], Vec::as_slice) {
            for c in self.blocks[*i].outs.iter().filter(|c| c.target == block) {
                if !v.contains(&c.input) {
                    v.push(c.input.clone());
                }
            }
        }
        v
    }

    /// Phase blocks (in other files) that import this block's file.
    pub fn importers_of(&self, file: &str) -> &[usize] {
        self.importers.get(&normalize(file)).map_or(&[], Vec::as_slice)
    }

    /// The phase block that owns `block`'s graph: its embedded owner, or (for a file root) the
    /// importer chosen by `prefer` (a set of GUIDs the caller knows to be running, say the
    /// template's scopes); the first importer when none is preferred.
    pub fn parent_phase(&self, block: usize, prefer: &dyn Fn(&Guid) -> bool) -> Option<usize> {
        let b = &self.blocks[block];
        if let Some(o) = b.owner {
            return Some(o);
        }
        let imps = self.importers_of(&b.file);
        imps.iter().copied().find(|i| prefer(&self.blocks[*i].guid)).or_else(|| imps.first().copied())
    }

    /// Like [`Library::parent_phase`], but a file imported by several phase blocks of which none is
    /// preferred (running in the template) is ambiguous: `Err` names the candidates. Shared helper
    /// phases (`empty_phase.w2phase`, bug-fix phases) have hundreds of importers.
    pub fn parent_phase_strict(&self, block: usize, prefer: &dyn Fn(&Guid) -> bool) -> Result<Option<usize>, String> {
        let b = &self.blocks[block];
        if let Some(o) = b.owner {
            return Ok(Some(o));
        }
        let imps = self.importers_of(&b.file);
        let preferred: Vec<usize> = imps.iter().copied().filter(|i| prefer(&self.blocks[*i].guid)).collect();
        match (preferred.as_slice(), imps) {
            ([one], _) | (_, [one]) => Ok(Some(*one)),
            ([], []) => Ok(None),
            (many, _) if !many.is_empty() => Err(format!("{} running importers of {}", many.len(), b.file)),
            (_, all) => Err(format!("{} importers of {}, none running in the template", all.len(), b.file)),
        }
    }

    /// The scope chain of a block: the GUIDs of the phase blocks above it, outermost first
    /// (what the save's `questThread[GUID]` wrappers are).
    pub fn scope_chain(&self, block: usize, prefer: &dyn Fn(&Guid) -> bool) -> Vec<Guid> {
        let mut chain = Vec::new();
        let mut at = block;
        let mut guard = 0;
        while let Some(p) = self.parent_phase(at, prefer) {
            chain.push(self.blocks[p].guid);
            at = p;
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        chain.reverse();
        chain
    }

    pub fn describe(&self, i: usize) -> String {
        let b = &self.blocks[i];
        let file = b.file.rsplit('/').next().unwrap_or(&b.file);
        format!("{} {} [{}] ({file})", b.short(), b.class.trim_start_matches("CQuest").trim_start_matches("CJournal"), b.label)
    }
}

fn connections(e: &Export, map: &HashMap<usize, usize>) -> Vec<Conn> {
    let Some(Val::Structs(conns)) = e.prop("cachedConnections") else { return Vec::new() };
    let mut out = Vec::new();
    for c in conns {
        let socket = match cr2w::prop_of(c, "socketId") {
            Some(Val::Name(s)) => s.trim().to_owned(),
            _ => String::new(),
        };
        let Some(Val::Structs(blocks)) = cr2w::prop_of(c, "blocks") else { continue };
        for b in blocks {
            let target = match cr2w::prop_ending(b, "ock") {
                Some(Val::Ptr(Some(t))) => *t,
                _ => continue,
            };
            let input = match cr2w::prop_ending(b, "putName") {
                Some(Val::Name(s)) => s.trim().to_owned(),
                _ => String::new(),
            };
            if let Some(t) = map.get(&target) {
                out.push(Conn { socket: socket.clone(), target: *t, input });
            }
        }
    }
    out
}

fn journal_path(c: &Cr2w, start: Option<usize>) -> Vec<PathStep> {
    let mut path = Vec::new();
    let mut at = start;
    while let Some(i) = at {
        let Some(e) = c.exports.get(i) else { break };
        let Some(guid) = e.guid() else { break };
        path.push(PathStep { guid, resource: e.str_prop("resource").unwrap_or("").to_owned() });
        at = match e.prop("child") {
            Some(Val::Ptr(p)) => *p,
            _ => None,
        };
        if path.len() > 8 {
            break;
        }
    }
    path
}

pub(crate) fn cond_of(c: &Cr2w, i: usize, depth: usize) -> Cond {
    let Some(e) = c.exports.get(i) else { return Cond::Other("?".into()) };
    match e.class.as_str() {
        "CQuestFactsDBCondition" => Cond::Fact {
            id: e.str_prop("factId").unwrap_or("").to_owned(),
            cmp: e.str_prop("compareFunc").unwrap_or("CF_Equal").to_owned(),
            value: e.int_prop("value").unwrap_or(0),
        },
        "CQuestHiResRealtimeDelayCondition" => Cond::Delay { ms: e.int_prop("miliseconds").unwrap_or(0).max(0) as u64 + e.int_prop("seconds").unwrap_or(0).max(0) as u64 * 1000 },
        "CQuestRealtimeDelayCondition" => {
            let secs = match e.prop("howLong") {
                Some(Val::Struct(p)) => match cr2w::prop_of(p, "m_seconds") {
                    Some(Val::Int(s)) => *s,
                    _ => 0,
                },
                _ => 0,
            };
            Cond::Delay { ms: secs.max(0) as u64 * 1000 }
        }
        "CQuestLogicOperationCondition" if depth < 8 => {
            let op = e.props.iter().find(|p| p.ty.starts_with('E')).map(|p| p.value.to_string()).unwrap_or_default();
            let inner = e
                .props
                .iter()
                .filter_map(|p| match &p.value {
                    Val::Ptrs(v) => Some(v),
                    _ => None,
                })
                .flatten()
                .flatten()
                .map(|j| cond_of(c, *j, depth + 1))
                .collect();
            Cond::Logic { op, inner }
        }
        other => Cond::Other(other.to_owned()),
    }
}

pub(crate) fn tag_list(c: &Cr2w, v: Option<&Val>) -> Vec<String> {
    // TagList: u8 count, then u16 name indices (HYPOTHESIS from `013500` = 1 tag, name 0x35).
    let Some(Val::Raw(b)) = v else { return Vec::new() };
    let Some(&n) = b.first() else { return Vec::new() };
    (0..n as usize)
        .filter_map(|i| b.get(1 + 2 * i..3 + 2 * i).map(|x| u16::from_le_bytes([x[0], x[1]])))
        .map(|i| c.names.get(i as usize).cloned().unwrap_or_else(|| format!("#{i}")))
        .collect()
}

fn kind_of(c: &Cr2w, e: &Export, _map: &HashMap<usize, usize>) -> BlockKind {
    let ptr = |name: &str| match e.prop(name) {
        Some(Val::Ptr(p)) => *p,
        _ => None,
    };
    match e.class.as_str() {
        "CQuestFactsDBChangingBlock" => BlockKind::Fact { id: e.str_prop("factID").unwrap_or("").to_owned(), value: e.int_prop("value").unwrap_or(1) },
        "CJournalQuestBlock" => BlockKind::Journal { quest: true, path: journal_path(c, ptr("questEntry")) },
        "CJournalBlock" => BlockKind::Journal { quest: false, path: journal_path(c, ptr("entry")) },
        "CJournalQuestMappinStateBlock" | "CJournalQuestTrackBlock" | "CJournalQuestObjectiveCounterGraphBlock" | "CJournalQuestMonsterKnownGraphBlock" | "CJournalQuestHuntingBlock" => {
            BlockKind::JournalOther
        }
        "CQuestSceneBlock" => BlockKind::Scene { file: e.str_prop("scene").unwrap_or("").to_owned() },
        "CQuestInteractionDialogBlock" | "CQuestContextDialogBlock" => {
            BlockKind::Interaction { file: e.str_prop("scene").unwrap_or("").to_owned(), tags: tag_list(c, e.prop("actorTags")) }
        }
        "CQuestPauseConditionBlock" => {
            let conds = match e.prop("conditions") {
                Some(Val::Ptrs(v)) => v.iter().flatten().map(|i| cond_of(c, *i, 0)).collect(),
                _ => Vec::new(),
            };
            BlockKind::Pause { conds }
        }
        "CQuestConditionBlock" => BlockKind::Condition { cond: ptr("questCondition").map(|i| cond_of(c, i, 0)) },
        "CQuestPhaseBlock" => BlockKind::Phase { embedded: ptr("embeddedGraph"), import: e.str_prop("phase").map(normalize) },
        "CQuestPhaseInputBlock" => BlockKind::PhaseInput { socket: e.str_prop("socketID").map(str::trim).filter(|s| !s.is_empty()).unwrap_or("In").to_owned() },
        "CQuestPhaseOutputBlock" => BlockKind::PhaseOutput { socket: e.str_prop("socketID").map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Out").to_owned() },
        "CQuestStoryPhaseSetterBlock" => {
            let actions = match e.prop("spawnsets") {
                Some(Val::Ptrs(v)) => v
                    .iter()
                    .flatten()
                    .filter_map(|i| c.exports.get(*i))
                    .map(|a| match a.class.as_str() {
                        "CActivateStoryPhase" => SpawnAction::Activate {
                            spawnset: normalize(a.str_prop("spawnset").unwrap_or("")),
                            phase: a.str_prop("phase").unwrap_or("").to_owned(),
                        },
                        "CDeactivateSpawnset" => SpawnAction::Deactivate { spawnset: normalize(a.str_prop("spawnset").unwrap_or("")) },
                        other => SpawnAction::Other(other.to_owned()),
                    })
                    .collect(),
                _ => Vec::new(),
            };
            BlockKind::StoryPhase { actions }
        }
        "CQuestRewardBlock" => BlockKind::Reward { name: e.str_prop("rewardName").unwrap_or("").to_owned() },
        "CQuestScriptBlock" => {
            let params = match e.prop("parameters") {
                Some(Val::Structs(v)) => v
                    .iter()
                    .map(|p| (cr2w::prop_of(p, "name").map(ToString::to_string).unwrap_or_default(), cr2w::prop_of(p, "value").map(ToString::to_string).unwrap_or_default()))
                    .collect(),
                _ => Vec::new(),
            };
            BlockKind::Script { function: e.str_prop("functionName").unwrap_or("").to_owned(), params }
        }
        "CQuestLayersHiderBlock" | "CQuestCheckpointBlock" | "CQuestCutControlBlock" | "CQuestTeleportBlock" | "CQuestTimeManagementBlock" | "CQuestDeletionMarkerBlock"
        | "CQuestStartBlock" | "CQuestEndBlock" | "CQuestFastForwardCommunitiesBlock" | "CQuestManageFastTravelBlock" | "CQuestEncounterManagerBlock"
        | "CQuestContentActivatorBlock" | "CQuestXorBlock" | "CQuestRandomBlock" | "CQuestResetScriptedActionsBlock" | "CManageSwitchBlock" | "CQuestChangeWorldBlock"
        | "CQuestEncounterFullRespawn" | "CQuestEncounterPhaseBlock" | "CDeniedAreaBlock" | "CQuestScenePrepareBlock" | "CQuestStaticCameraSwitchBlock"
        | "CQuestStaticCameraStopBlock" | "CQuestStaticCameraRunBlock" | "CCounterUtilityGraphBlock" | "CQuestEncounterActivator" => BlockKind::Instant,
        // CQuestAndBlock waits for all its inputs; CQuestScriptedActionsBlock, combat managers,
        // animations, minigames, vehicles, motion, look-at: they stay in the frontier while they run.
        _ => BlockKind::Wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_normalize() {
        assert_eq!(normalize("Quests\\Part_1\\X.w2phase"), "quests/part_1/x.w2phase");
        assert_eq!(short(&[0xab, 0xcd, 0xef, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "abcdef01");
    }

    #[test]
    fn guid_parsing() {
        let g = parse_guid("300f579b470d8e43af03591cc6eaa70b").unwrap();
        assert_eq!(short(&g), "300f579b");
        assert!(parse_guid("300f").is_err());
    }
}
