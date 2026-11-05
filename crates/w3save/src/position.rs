//! Player position and loaded world. The plan expects this to be the hardest edit.
//!
//! Open spawn-point question (docs/research.md): the palace is only reachable
//! inside scripted quests, so the intended design is a free-roam signpost spawn.
//! Nothing here can be written until Phase 0 answers known unknown 4 (how position
//! and the level name are stored in `universe`/entity data).
use crate::document::Document;
use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub fn get(_doc: &Document) -> Result<Position> {
    todo!("read the player position: location of the entity record is unknown (Phase 0, unknown 4)")
}

pub fn set(_doc: &mut Document, _to: Position, _level_name: &str) -> Result<()> {
    todo!("write position and world/level name together, then rebase (Phase 1)")
}
