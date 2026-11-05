//! Another Life core: turns a player's answers into a list of effects.
//!
//! This crate never touches save bytes. It loads the questbook (TOML), validates
//! it, walks a [`run::Run`] in chronological order and produces
//! [`resolve::Resolution`], which `export` hands to `w3save`.
#![forbid(unsafe_code)]

pub mod branch_swap;
pub mod chain;
pub mod diagnostic;
pub mod edit;
pub mod export;
pub mod graph_effect;
pub mod journal_effect;
pub mod paths;
pub mod questbook;
pub mod resolve;
pub mod run;
pub mod save_name;
pub mod side;
pub mod start_kit;
pub mod synth_effect;
pub mod templates;
pub mod validate;
pub mod view;

pub use diagnostic::{Diagnostic, Severity};
pub use view::QuestbookView;
pub use questbook::{Effect, Questbook, Quest};
pub use resolve::{resolve, Resolution};
pub use run::{Run, Status};
