//! ClickHouse `system.columns` probe.
//!
//! Single round-trip per metric. The query is bound-parameters-only —
//! `database`, `table`, and `column` are passed via `?` placeholders even
//! though `chk_metric_catalog_metric_key_shape` constrains the shape DB-side
//! and [`super::parse`] re-validates application-side. Defense in depth.
//!
//! ## Required ClickHouse privileges
//!
//! The principal used here MUST be granted `SELECT` on `system.columns` and
//! `SHOW TABLES` (or equivalent) on the analytics database. `system.columns`
//! is filtered server-side to tables the principal can see, so a too-narrow
//! grant silently degrades every row to `TableNotFound` — indistinguishable
//! from a real missing table. The read-only ClickHouse role and its runbook
//! ship out-of-band with #523/#525; this module assumes it has been wired.
//!
//! ## Why one query with `countIf`, not the spec's verbatim three-AND form
//!
//! Canonical error codes (DESIGN §3.7) distinguish `table_not_found` from
//! `column_not_found`. A single
//! `count() WHERE database=? AND table=? AND name=?` returns 0 in both cases;
//! disambiguating with a follow-up query doubles the miss-path round-trip.
//! `countIf` keeps it to one query, still scoped to `system.columns`, still
//! bound-parameters-only.

use clickhouse::Row;
use serde::Deserialize;

use crate::domain::schema_validator::error_code::SchemaErrorCode;

/// Bound-parameter probe of `system.columns`.
///
/// `clickhouse-rs` replaces `?` placeholders in `bind()` invocation order
/// (first `bind` fills the first `?` left-to-right in the SQL string). Bind
/// order is therefore `(column, database, table)` to match the SQL below.
const PROBE_SQL: &str = "\
    SELECT count() AS total_columns, countIf(name = ?) AS matching_columns, \
           countIf(name = ?) AS metric_key_columns, \
           countIf(name = ?) AS metric_value_columns \
    FROM system.columns \
    WHERE database = ? AND table = ?";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    Ok,
    Error(SchemaErrorCode),
}

#[derive(Row, Deserialize)]
#[allow(clippy::struct_field_names)] // fields ARE counts of distinct column kinds; postfix is semantic.
struct ProbeRow {
    total_columns: u64,
    matching_columns: u64,
    metric_key_columns: u64,
    metric_value_columns: u64,
}

/// Pure mapping from probed `(total_columns, matching_columns)` to a
/// `ProbeOutcome`. Extracted so the unit test exercises the same function
/// the production path uses (a parallel reimplementation in the test would
/// not catch a swap of the two columns inside `probe`).
fn classify(row: &ProbeRow) -> ProbeOutcome {
    if row.total_columns == 0 {
        ProbeOutcome::Error(SchemaErrorCode::TableNotFound)
    } else if row.metric_key_columns > 0 {
        if row.metric_value_columns > 0 {
            ProbeOutcome::Ok
        } else {
            ProbeOutcome::Error(SchemaErrorCode::ColumnNotFound)
        }
    } else if row.matching_columns == 0 {
        ProbeOutcome::Error(SchemaErrorCode::ColumnNotFound)
    } else {
        ProbeOutcome::Ok
    }
}

/// Probe ClickHouse `system.columns` for the existence of `database.table.column`.
///
/// Returns a [`ProbeOutcome`] mapping per DESIGN §3.7:
/// - `total_columns == 0` → `Error(TableNotFound)`
/// - `total_columns > 0 && matching_columns == 0` → `Error(ColumnNotFound)`
/// - `matching_columns >= 1` → `Ok`
///
/// Network/auth/timeout failures are surfaced through the `Err` channel; the
/// caller maps them to `SchemaErrorCode::ClickhouseUnreachable` and the raw
/// error text lands in a structured log line **only** (never in
/// `schema_error_code`).
///
/// # Errors
///
/// Returns [`ProbeError::Unreachable`] for any transport / auth / timeout
/// failure talking to ClickHouse.
pub async fn probe(
    ch: &insight_clickhouse::Client,
    database: &str,
    table: &str,
    column: &str,
) -> Result<ProbeOutcome, ProbeError> {
    let mut query = ch.query(PROBE_SQL);
    query = query
        .bind(column)
        .bind("metric_key")
        .bind("metric_value")
        .bind(database)
        .bind(table);
    let row: ProbeRow = query.fetch_one().await.map_err(ProbeError::Unreachable)?;
    Ok(classify(&row))
}

/// Errors the probe surfaces to the caller. The caller is responsible for
/// translating these to a canonical `schema_error_code` and a log line — raw
/// error text never lands in the DB column per DESIGN §3.2.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// Transport, auth, or timeout failure talking to ClickHouse.
    /// Mapped to `SchemaErrorCode::ClickhouseUnreachable`.
    #[error("clickhouse probe transport failure: {0}")]
    Unreachable(#[from] clickhouse::error::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_contains_only_bound_placeholders() {
        // Regression guard against accidental string-interpolation refactors —
        // the only allowed values are the five `?` placeholders.
        assert_eq!(PROBE_SQL.matches('?').count(), 5);
        // No single-quoted literal anywhere in the SQL — every value lives in a bind.
        assert!(!PROBE_SQL.contains('\''));
        // Scoped to system.columns only.
        assert!(PROBE_SQL.contains("system.columns"));
        // The match predicate is on `name = ?`, not interpolated.
        assert!(PROBE_SQL.contains("countIf(name = ?)"));
    }

    #[test]
    fn classify_distinguishes_table_and_column_misses() {
        // Exercises the production mapping — a future refactor that swaps
        // `total_columns` and `matching_columns` (a real, easy regression
        // because the two field names are visually similar) will trip this.
        assert_eq!(
            classify(&ProbeRow {
                total_columns: 0,
                matching_columns: 0,
                metric_key_columns: 0,
                metric_value_columns: 0
            }),
            ProbeOutcome::Error(SchemaErrorCode::TableNotFound)
        );
        assert_eq!(
            classify(&ProbeRow {
                total_columns: 12,
                matching_columns: 0,
                metric_key_columns: 0,
                metric_value_columns: 0
            }),
            ProbeOutcome::Error(SchemaErrorCode::ColumnNotFound)
        );
        assert_eq!(
            classify(&ProbeRow {
                total_columns: 12,
                matching_columns: 1,
                metric_key_columns: 0,
                metric_value_columns: 0
            }),
            ProbeOutcome::Ok
        );
    }

    #[test]
    fn classify_long_format_table_is_ok_without_metric_column() {
        assert_eq!(
            classify(&ProbeRow {
                total_columns: 5,
                matching_columns: 0,
                metric_key_columns: 1,
                metric_value_columns: 1
            }),
            ProbeOutcome::Ok
        );
        assert_eq!(
            classify(&ProbeRow {
                total_columns: 4,
                matching_columns: 0,
                metric_key_columns: 1,
                metric_value_columns: 0
            }),
            ProbeOutcome::Error(SchemaErrorCode::ColumnNotFound)
        );
    }
}
