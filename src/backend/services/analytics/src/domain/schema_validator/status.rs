//! `schema_status` enum + state-transition logging decision.
//!
//! `schema_status` is one of `ok` / `error` / `unchecked` (DB-side ENUM +
//! `chk_metric_catalog_schema_status_enum`). The structured-log policy
//! (DESIGN §3.2): log every transition except no-ops.

use crate::domain::schema_validator::error_code::SchemaErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaStatus {
    Ok,
    Error,
    Unchecked,
}

impl SchemaStatus {
    /// Canonical wire/DB string.
    #[must_use]
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Unchecked => "unchecked",
        }
    }

    /// Parse from a DB `ENUM` string.
    #[must_use]
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(Self::Ok),
            "error" => Some(Self::Error),
            "unchecked" => Some(Self::Unchecked),
            _ => None,
        }
    }
}

/// One state in the `(status, error_code)` pair. The biconditional
/// `(status = Error) ⇔ (error_code ≠ None)` is enforced by
/// `chk_metric_catalog_schema_error_biconditional` and asserted in tests below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaState {
    pub status: SchemaStatus,
    pub error_code: Option<SchemaErrorCode>,
}

impl SchemaState {
    #[must_use]
    pub fn ok() -> Self {
        Self {
            status: SchemaStatus::Ok,
            error_code: None,
        }
    }

    #[must_use]
    pub fn error(code: SchemaErrorCode) -> Self {
        Self {
            status: SchemaStatus::Error,
            error_code: Some(code),
        }
    }

    #[must_use]
    pub fn unchecked() -> Self {
        Self {
            status: SchemaStatus::Unchecked,
            error_code: None,
        }
    }
}

/// Decide whether a state change merits a structured log line.
///
/// Logged: every transition where either `status` changes or, when both sides
/// are `Error`, the `error_code` changes. Suppressed: same-state no-ops.
#[must_use]
pub fn should_log_transition(from: SchemaState, to: SchemaState) -> bool {
    if from.status != to.status {
        return true;
    }
    if from.status == SchemaStatus::Error && from.error_code != to.error_code {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trip() {
        for s in [
            SchemaStatus::Ok,
            SchemaStatus::Error,
            SchemaStatus::Unchecked,
        ] {
            assert_eq!(SchemaStatus::from_db_str(s.as_db_str()), Some(s));
        }
    }

    #[test]
    fn status_db_strings_match_check_constraint_set() {
        // Pin DB strings against migration #519 CHECK `chk_metric_catalog_schema_status_enum`.
        assert_eq!(SchemaStatus::Ok.as_db_str(), "ok");
        assert_eq!(SchemaStatus::Error.as_db_str(), "error");
        assert_eq!(SchemaStatus::Unchecked.as_db_str(), "unchecked");
    }

    #[test]
    fn ok_to_ok_suppressed() {
        assert!(!should_log_transition(SchemaState::ok(), SchemaState::ok()));
    }

    #[test]
    fn same_error_code_suppressed() {
        let s = SchemaState::error(SchemaErrorCode::ColumnNotFound);
        assert!(!should_log_transition(s, s));
    }

    #[test]
    fn unchecked_to_ok_logged() {
        assert!(should_log_transition(
            SchemaState::unchecked(),
            SchemaState::ok()
        ));
    }

    #[test]
    fn ok_to_error_logged() {
        assert!(should_log_transition(
            SchemaState::ok(),
            SchemaState::error(SchemaErrorCode::ColumnNotFound)
        ));
    }

    #[test]
    fn error_to_ok_logged() {
        assert!(should_log_transition(
            SchemaState::error(SchemaErrorCode::TableNotFound),
            SchemaState::ok()
        ));
    }

    #[test]
    fn error_code_change_logged() {
        assert!(should_log_transition(
            SchemaState::error(SchemaErrorCode::ColumnNotFound),
            SchemaState::error(SchemaErrorCode::TableNotFound)
        ));
    }

    #[test]
    fn unchecked_to_unchecked_suppressed() {
        // The startup retry loop emits one summary log when entering
        // degraded mode; per-row unchecked→unchecked is not a transition.
        assert!(!should_log_transition(
            SchemaState::unchecked(),
            SchemaState::unchecked()
        ));
    }
}
