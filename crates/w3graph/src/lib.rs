//! Quest graphs read from the player's own Witcher 3 install, and a walker that follows one
//! decision through them to predict what the game would write into the save: the facts set, the
//! journal entries activated, the spawn-set phases switched, and the blocks that wait next
//! (the new quest frontier). Design and evidence: `docs/plan/no-human-pipeline.md`.
//!
//! Nothing from the game files is stored in the repository. Everything here reads the bundles
//! (`content\content0\bundles\*.bundle`, `dlc\*\content\bundles\*.bundle`) or a folder of
//! already extracted `.w2phase` / `.w2quest` files at run time, read-only.
//!
//! Layers:
//! 1. [`bundle`]: `POTATO70` bundle table of contents and file extraction.
//! 2. [`cr2w`]: the `CR2W` resource container: names, imports, exports and their property streams
//!    (the format facts came from our own reading of the files, see `docs/plan/no-human-pipeline.md`;
//!    no third-party code).
//! 3. [`quest`]: the typed quest-graph model: blocks, sockets, conditions, phases, imports.
//! 4. [`walk`]: the decision walker.
//! 5. [`frontier`]: the save side: which graph blocks a save waits at, in which scopes, and the
//!    comparison of a walk against a real played donor.
//! 6. [`synth`]: writes a walk into a copy of the template save (no donor).
//!
//! Format details that no fixture has confirmed carry a `HYPOTHESIS:` comment, as in `w3save`.
#![forbid(unsafe_code)]

pub mod bundle;
pub mod catalog;
pub mod community;
pub mod complete;
pub mod cr2w;
pub mod frontier;
pub mod names;
pub mod quest;
pub mod resources;
pub mod scene;
pub mod strings;
pub mod synth;
pub mod walk;

pub use quest::{Block, BlockKind, Cond, Library};
pub use walk::{Event, Policy, Walk};
