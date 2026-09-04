//! Why a query is refused: the request field, a machine code, and a sentence.

use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// A path into the request document, such as `filters[1].value`.
    pub field: String,
    pub reason: Reason,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Unknown,
    Missing,
    Unexpected,
    OutOfRange,
    Duplicate,
    /// The field cannot hold a value of that type.
    TypeMismatch,
    /// The type fits; the field's own shape rule rejects the value.
    Malformed,
}

impl Reason {
    pub fn as_code(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::Missing => "MISSING",
            Self::Unexpected => "UNEXPECTED",
            Self::OutOfRange => "OUT_OF_RANGE",
            Self::Duplicate => "DUPLICATE",
            Self::TypeMismatch => "TYPE_MISMATCH",
            Self::Malformed => "MALFORMED",
        }
    }
}

impl Violation {
    pub fn new(field: impl Into<String>, reason: Reason, detail: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason,
            detail: detail.into(),
        }
    }

    /// A refusal that also reports what would have been admissible.
    pub fn unknown(field: impl Into<String>, named: &str, admissible: &[&str]) -> Self {
        let mut detail = format!("`{named}` is not declared here");
        if admissible.is_empty() {
            detail.push_str("; this dataset declares none");
        } else {
            let _ = write!(detail, "; declared: {}", admissible.join(", "));
        }

        Self::new(field, Reason::Unknown, detail)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_field_refusal_reports_the_admissible_set() {
        let violation =
            Violation::unknown("group_by[0].field", "branch", &["repository", "source"]);

        assert_eq!(violation.field, "group_by[0].field");
        assert_eq!(violation.reason, Reason::Unknown);
        assert_eq!(
            violation.detail,
            "`branch` is not declared here; declared: repository, source"
        );
    }

    #[test]
    fn an_unknown_field_refusal_over_an_empty_set_says_so() {
        let violation = Violation::unknown("aggregates[0].field", "lines", &[]);
        assert_eq!(
            violation.detail,
            "`lines` is not declared here; this dataset declares none"
        );
    }

    #[test]
    fn every_reason_carries_a_distinct_machine_code() {
        let reasons = [
            Reason::Unknown,
            Reason::Missing,
            Reason::Unexpected,
            Reason::OutOfRange,
            Reason::Duplicate,
            Reason::TypeMismatch,
            Reason::Malformed,
        ];
        let mut codes: Vec<&str> = reasons.iter().map(|reason| reason.as_code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), reasons.len());
    }
}
