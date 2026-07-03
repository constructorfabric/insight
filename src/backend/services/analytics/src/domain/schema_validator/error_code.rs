//! Canonical `schema_error_code` set (DESIGN §3.7).
//!
//! Closed enum — these four strings are the **only** values that ever land
//! in `metric_catalog.schema_error_code`. Raw ClickHouse error text NEVER
//! reaches this column; it goes to the structured log only.
//!
//! The DB-side CHECK `chk_metric_catalog_schema_error_enum` enforces the same
//! set; this enum is the application-layer mirror per the dual-validate
//! principle.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SchemaErrorCode {
    TableNotFound,
    ColumnNotFound,
    ClickhouseUnreachable,
    Unknown,
}

impl SchemaErrorCode {
    /// Canonical wire/DB string. Must match
    /// `chk_metric_catalog_schema_error_enum` exactly.
    #[must_use]
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::TableNotFound => "table_not_found",
            Self::ColumnNotFound => "column_not_found",
            Self::ClickhouseUnreachable => "clickhouse_unreachable",
            Self::Unknown => "unknown",
        }
    }

    /// Parse from a DB string. Returns `None` if the string is outside the
    /// canonical set (which the DB CHECK forbids; defensive for raw-SQL paths).
    #[must_use]
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "table_not_found" => Some(Self::TableNotFound),
            "column_not_found" => Some(Self::ColumnNotFound),
            "clickhouse_unreachable" => Some(Self::ClickhouseUnreachable),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

impl fmt::Display for SchemaErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_db_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_every_variant() {
        for code in [
            SchemaErrorCode::TableNotFound,
            SchemaErrorCode::ColumnNotFound,
            SchemaErrorCode::ClickhouseUnreachable,
            SchemaErrorCode::Unknown,
        ] {
            let s = code.as_db_str();
            assert_eq!(SchemaErrorCode::from_db_str(s), Some(code), "{s}");
        }
    }

    #[test]
    fn unknown_string_returns_none() {
        // Anything outside the canonical four — including raw ClickHouse text —
        // must parse to None. This is the application-layer mirror of the
        // CHECK constraint refusing the same value at the DB layer.
        assert_eq!(SchemaErrorCode::from_db_str(""), None);
        assert_eq!(SchemaErrorCode::from_db_str("table_not_found "), None);
        assert_eq!(
            SchemaErrorCode::from_db_str("connection refused: 127.0.0.1:8123"),
            None
        );
        assert_eq!(SchemaErrorCode::from_db_str("TABLE_NOT_FOUND"), None);
    }

    #[test]
    fn db_strings_match_check_constraint_set() {
        // Pin the canonical set against the DESIGN §3.7 / migration #519 CHECK.
        // If a future refactor renames a variant, this test catches the drift
        // before it hits the DB layer.
        let canonical = [
            "table_not_found",
            "column_not_found",
            "clickhouse_unreachable",
            "unknown",
        ];
        let from_enum = [
            SchemaErrorCode::TableNotFound.as_db_str(),
            SchemaErrorCode::ColumnNotFound.as_db_str(),
            SchemaErrorCode::ClickhouseUnreachable.as_db_str(),
            SchemaErrorCode::Unknown.as_db_str(),
        ];
        assert_eq!(from_enum, canonical);
    }
}
