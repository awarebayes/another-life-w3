//! What the UI receives from `load_questbook`: quests plus precomputed warnings, so
//! the JavaScript never re-implements validation rules.
use std::collections::BTreeMap;

use serde::Serialize;

use crate::diagnostic::Diagnostic;
use crate::questbook::{Quest, Questbook};
use crate::validate;

#[derive(Debug, Serialize)]
pub struct QuestbookView {
    pub quests: Vec<Quest>,
    /// quest id -> soft level warning
    pub warnings: BTreeMap<String, String>,
    /// Lint findings; the app shows errors in a banner.
    pub diagnostics: Vec<Diagnostic>,
    /// The generated side quests (`questbook/side/`, `crate::side`), in side order: the Side quests page.
    pub side: Vec<crate::side::SideQuest>,
}

impl QuestbookView {
    pub fn new(book: &Questbook) -> Self {
        Self {
            quests: book.quests.clone(),
            warnings: validate::level_warnings(book),
            diagnostics: validate::lint(book),
            side: book.side.quests.clone(),
        }
    }
}
