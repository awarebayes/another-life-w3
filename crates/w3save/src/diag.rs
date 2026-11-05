use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// One finding from a `check` pass. Shared by `w3al check-save` and the app.
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
}

impl Issue {
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            message: message.into(),
        }
    }
    pub fn info(m: impl Into<String>) -> Self {
        Self::new(Severity::Info, m)
    }
    pub fn warn(m: impl Into<String>) -> Self {
        Self::new(Severity::Warning, m)
    }
    pub fn error(m: impl Into<String>) -> Self {
        Self::new(Severity::Error, m)
    }
}
