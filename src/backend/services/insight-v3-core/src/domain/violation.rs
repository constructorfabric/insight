#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    Missing,
    Unknown,
    Duplicate,
    Malformed,
    NotAdmissible,
}

impl Reason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "MISSING",
            Self::Unknown => "UNKNOWN",
            Self::Duplicate => "DUPLICATE",
            Self::Malformed => "MALFORMED",
            Self::NotAdmissible => "NOT_ADMISSIBLE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Violation {
    pub(crate) field: String,
    pub(crate) reason: Reason,
    pub(crate) detail: String,
}

impl Violation {
    pub(crate) fn new(field: impl Into<String>, reason: Reason, detail: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason,
            detail: detail.into(),
        }
    }

    pub(crate) fn reason_code(&self) -> &'static str {
        self.reason.as_str()
    }
}

pub(crate) fn said(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(|violation| format!("{}: {}", violation.field, violation.detail))
        .collect::<Vec<_>>()
        .join("; ")
}
