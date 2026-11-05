//! A player's run: what the `run.toml` profile file holds and the UI keeps in memory.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const RUN_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Completed,
    Started,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestEntry {
    pub status: Status,
    /// choice id -> option id. Missing choices fall back to the choice's default.
    #[serde(default)]
    pub answers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Simulate the game's Witcher 2 import prompts. The individual W2 decisions
    /// are an open question in the plan; for now this is a single switch.
    #[serde(default)]
    pub w2_import: bool,
    /// Manual override: the save the player pointed at on the Welcome screen. `None` = the story chain
    /// picks the template from `questbook/templates.toml` (latest window template at or before
    /// `stop_after`, found by stem in `template_dirs`; docs/plan/story-chain.md).
    pub template: Option<String>,
    /// Folders searched for the registry's template saves (`<stem>.sav`, not recursive). Empty = the
    /// `templates` folder next to the questbook.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub template_dirs: Vec<String>,
    /// Last quest whose effects are included ("Wake up in Vizima" after it).
    /// `None` = walk everything in the run.
    pub stop_after: Option<String>,
    /// quest id -> entry. A quest with no entry counts as skipped.
    #[serde(default)]
    pub quests: BTreeMap<String, QuestEntry>,
    /// Folders holding donor saves (`<donor stem>.sav`) for graph effects. Donors are real saves and
    /// never part of the repo or the questbook; export looks each one up by stem (see `export`).
    #[serde(default)]
    pub donors: Vec<String>,
    /// Allow `NameMode::Remap` for a graph effect whose donor's NM names do not extend the save's.
    /// HYPOTHESIS: proven offline only (docs/findings/name-remap.md); off by default.
    #[serde(default)]
    pub allow_name_remap: bool,
    /// The Witcher 3 install whose quest graphs `synth` effects walk (read-only). `None` or empty =
    /// `export::DEFAULT_GAME_DIR` (the Steam default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_dir: Option<String>,
    /// The start kit (`[start_kit]`): level, crowns and supplies written into the exported save after the
    /// story effects (`start_kit.rs`). Absent = the template's character is kept as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_kit: Option<crate::start_kit::StartKit>,
    /// Side quest id (`questbook/side/`, `crate::side`) -> completed or skipped. Completed ones are finished in the
    /// exported save by the auto-complete walk after the story's own effects; absent = skipped.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub side_quests: BTreeMap<String, Status>,
}

fn default_version() -> u32 {
    RUN_VERSION
}

impl Default for Run {
    fn default() -> Self {
        Self {
            version: RUN_VERSION,
            w2_import: false,
            template: None,
            template_dirs: Vec::new(),
            stop_after: None,
            quests: BTreeMap::new(),
            donors: Vec::new(),
            allow_name_remap: false,
            game_dir: None,
            start_kit: None,
            side_quests: BTreeMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("run file: {0}")]
    Parse(String),
    #[error("run file is version {0}; this build understands version {RUN_VERSION}")]
    Version(u32),
}

impl Run {
    pub fn load(path: &Path) -> Result<Self, RunError> {
        let run: Run = toml::from_str(&fs::read_to_string(path)?).map_err(|e| RunError::Parse(e.to_string()))?;
        if run.version != RUN_VERSION {
            return Err(RunError::Version(run.version));
        }
        Ok(run)
    }

    /// Overwriting a run profile is fine (it is the user's own file); only *saves*
    /// are never overwritten.
    pub fn save(&self, path: &Path) -> Result<(), RunError> {
        let text = toml::to_string_pretty(self).map_err(|e| RunError::Parse(e.to_string()))?;
        fs::write(path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_roundtrip() {
        let mut run = Run {
            stop_after: Some("family_matters".into()),
            ..Run::default()
        };
        run.quests.insert(
            "family_matters".into(),
            QuestEntry {
                status: Status::Completed,
                answers: [("botchling".to_string(), "lubberkin".to_string())].into(),
            },
        );
        let text = toml::to_string_pretty(&run).unwrap();
        assert_eq!(toml::from_str::<Run>(&text).unwrap(), run);
    }
}
