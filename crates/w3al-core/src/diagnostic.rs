use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Error,
}

/// A finding about content (`lint`) or about a run (`validate_run`).
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Quest the finding belongs to, when there is one.
    pub quest: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn error(quest: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            quest: quest.map(str::to_owned),
            message: message.into(),
        }
    }

    pub fn warn(quest: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            quest: quest.map(str::to_owned),
            message: message.into(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}
