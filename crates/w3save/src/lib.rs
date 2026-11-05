//! Reader/writer for The Witcher 3 (Remastered PC, patch 5.x) saves.
//!
//! Layering follows the plan:
//!   1. `container`  – `SNFHFZLC` outer file, chunk table, raw LZ4 blocks.
//!   2. `payload`    – `SAV3` footer, variable table, `NM`/`RB` sections.
//!   3. `document`   – parse-to-regions / serialize-from-regions (the edit surface).
//!   4. `facts`, `position`, `sidecar` – game-level views on top of the document.
//!
//! Everything about the format is a hypothesis until Phase 0 confirms it on a real
//! 5.01 save. Code that depends on an unconfirmed detail says so in a `HYPOTHESIS:`
//! comment; code that cannot be written without evidence is `todo!()` or returns
//! [`Error::Unverified`].
#![forbid(unsafe_code)]

pub mod character;
pub mod container;
pub mod diag;
pub mod document;
pub mod error;
pub mod facts;
pub mod graph;
pub mod inspect;
pub mod journal;
pub mod newgameplus;
pub mod packed;
pub mod payload;
pub mod position;
pub mod reader;
pub mod sidecar;
pub mod token;

pub use container::Container;
pub use diag::{Issue, Severity};
pub use document::{Document, Edits, Region};
pub use token::{Kind, Node, Part, TokenStats, Value};
pub use error::{Error, Result};
pub use inspect::{inspect, SaveInfo};
