//! What a refusal says, and where in the submitted body it belongs.
//!
//! Every kind reports its own rules, but they all report them the same way:
//! a path into the body the caller sent, a reason it can branch on, and a
//! sentence a reader can act on. A body wrong in several places is answered
//! once, with all of them.

/// Why one part of a submitted body cannot be accepted.
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

/// One problem, addressed to the place in the body that carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Violation {
    /// Where the problem is, as the submitted body is shaped:
    /// `fields[2].type`, `row_identity[0]`, `name`.
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

    /// The reason as a caller branches on it, rather than reads.
    pub(crate) fn reason_code(&self) -> &'static str {
        self.reason.as_str()
    }
}

/// Every violation in one line, for a reader with nowhere to mark a field.
///
/// The paths are kept: a model repairing its own body needs to know which
/// part of it was refused, and the sentence alone does not say.
pub(crate) fn said(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(|violation| format!("{}: {}", violation.field, violation.detail))
        .collect::<Vec<_>>()
        .join("; ")
}
