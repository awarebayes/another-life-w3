//! The questbook: hand-curated TOML, one file per quest (see questbook/SCHEMA.md).
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::graph_effect::GraphEffect;
use crate::synth_effect::SynthEffect;
use crate::journal_effect::{JournalAppendEffect, JournalCounterEffect, JournalStatusEffect, JournalTrackEffect};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    WhiteOrchard,
    Velen,
    Novigrad,
    Skellige,
    KaerMorhen,
    Vizima,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Main,
    Secondary,
    Contract,
}

/// Quest phase an effect can put a game quest into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Started,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    /// Script line or quest graph this was taken from. Required: reviewers check it.
    pub source: String,
    /// Scaffold value, not yet traced to a script or a save diff. Lint warns on it.
    #[serde(default)]
    pub placeholder: bool,
    #[serde(default)]
    pub facts: BTreeMap<String, i64>,
    /// Facts to remove after `facts` are set (what the script `FactsRemove` does), when the donor no longer
    /// has them (e.g. `keira_part1_dd_start`, removed by every Keira donor). A name the template lacks is
    /// already "removed" and is skipped.
    #[serde(default)]
    pub facts_remove: Vec<String>,
    #[serde(default)]
    pub quests: BTreeMap<String, Phase>,
    /// Legacy id list from the scaffold; never reached a save (export refuses it).
    #[serde(default)]
    pub journal: Vec<String>,
    /// Quest-log edits captured with `w3al journal-diff` (see `journal_effect`). Export applies
    /// them together with the effect's `graph` (gates G, P, S, F, R; docs/findings/export-wiring.md).
    #[serde(default)]
    pub journal_status: Vec<JournalStatusEffect>,
    #[serde(default)]
    pub journal_append: Vec<JournalAppendEffect>,
    #[serde(default)]
    pub journal_counters: Vec<JournalCounterEffect>,
    #[serde(default)]
    pub journal_track: Option<JournalTrackEffect>,
    /// Quest-graph / spawn-set delta (`[effects.X.graph]`, the output of `w3al graph-diff --toml`;
    /// see `graph_effect`). Applied before the journal and the facts of the same effect.
    #[serde(default)]
    pub graph: Option<GraphEffect>,
    /// Synthesized from the game's quest graphs (`[effects.X.synth]`, see `synth_effect`): the walk writes
    /// the quest frontier, journal and facts itself, in the slot `graph` takes; the effect's own journal
    /// tables, `facts` and `facts_remove` apply after it. Never together with `graph`.
    #[serde(default)]
    pub synth: Option<SynthEffect>,
    /// The same answer completed late on a later template that has not reached the quest
    /// (`[effects.X.late]`, see `synth_effect::LateEffect`); used instead of `synth` / `graph` when the chosen
    /// template lists the quest in its registry `late` list.
    #[serde(default)]
    pub late: Option<crate::synth_effect::LateEffect>,
}

impl Effect {
    /// True when the effect carries a `synth` table.
    pub fn has_synth(&self) -> bool {
        self.synth.is_some()
    }

    /// True when the effect carries a `graph` table.
    pub fn has_graph_edit(&self) -> bool {
        self.graph.is_some()
    }

    /// The graph ops, validated (`None` when there is no `graph` table).
    pub fn graph_ops(&self) -> Result<Option<Vec<crate::graph_effect::GuardedOp>>, String> {
        self.graph.as_ref().map(GraphEffect::to_ops).transpose()
    }

    /// True when the effect carries any `journal_*` table.
    pub fn has_journal_edit(&self) -> bool {
        !self.journal_status.is_empty()
            || !self.journal_append.is_empty()
            || !self.journal_counters.is_empty()
            || self.journal_track.is_some()
    }

    /// The `journal_*` tables as one `w3save` edit (validated: GUIDs, status names, path lengths).
    pub fn journal_edit(&self) -> Result<w3save::journal::JournalEdit, String> {
        crate::journal_effect::to_edit(
            &self.journal_status,
            &self.journal_append,
            &self.journal_counters,
            self.journal_track.as_ref(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub id: String,
    pub text: String,
    /// Key into the quest's `[effects.*]` table. Absent = the option changes nothing.
    pub effects: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub id: String,
    pub prompt: String,
    /// Option id used when the player doesn't care. Required on every choice.
    pub default: String,
    pub options: Vec<ChoiceOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quest {
    pub id: String,
    pub title: String,
    pub act: u8,
    /// Global chronological sort key.
    pub order: u32,
    pub kind: Kind,
    pub region: Region,
    /// The game's recommended level.
    pub level: u8,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub excludes: Vec<String>,
    pub image: Option<String>,
    pub summary: String,
    /// Effects of leaving the quest at its first objective ("started").
    pub started_effects: Option<String>,
    /// Effects of the quest's "done" phase, applied after the chosen options.
    pub done_effects: Option<String>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub effects: BTreeMap<String, Effect>,
    #[serde(skip)]
    pub file: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Questbook {
    /// Sorted by `order`.
    pub quests: Vec<Quest>,
    /// The generated side quests (`<questbook>/side/`, `crate::side`); empty when the folder is missing.
    #[serde(skip)]
    pub side: crate::side::SideBook,
}

impl Questbook {
    pub fn load_dir(dir: &Path) -> Result<Self, LoadError> {
        let mut files = Vec::new();
        collect_toml(dir, &mut files)?;
        let side = crate::side::SideBook::load_dir(&dir.join(crate::side::SIDE_DIR))?;
        files.sort();

        let mut quests = Vec::with_capacity(files.len());
        for path in files {
            let text = fs::read_to_string(&path).map_err(|source| LoadError::Io {
                path: path.clone(),
                source,
            })?;
            let mut q: Quest = toml::from_str(&text).map_err(|e| LoadError::Parse {
                path: path.clone(),
                message: e.to_string(),
            })?;
            q.file = path;
            quests.push(q);
        }
        Ok(Self { side, ..Self::from_quests(quests) })
    }

    pub fn from_quests(mut quests: Vec<Quest>) -> Self {
        // Stable: equal `order` keys keep file order so lint can report them.
        quests.sort_by_key(|q| q.order);
        Self { quests, side: Default::default() }
    }

    pub fn get(&self, id: &str) -> Option<&Quest> {
        self.quests.iter().find(|q| q.id == id)
    }
}

fn collect_toml(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), LoadError> {
    let io = |source| LoadError::Io {
        path: dir.to_owned(),
        source,
    };
    for entry in fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.is_dir() {
            // `side/` holds the generated side quests (crate::side), loaded apart.
            if path.file_name().is_some_and(|n| n == crate::side::SIDE_DIR) && path.parent() == Some(dir) {
                continue;
            }
            collect_toml(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "toml") && path.file_name().is_none_or(|n| n != crate::templates::REGISTRY_FILE && n != "images.toml") {
            // `templates.toml` is the template registry (crate::templates), `images.toml` the illustration links
            // (read by the app shell); neither is a quest.
            out.push(path);
        }
    }
    Ok(())
}
