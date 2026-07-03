//! MariaDB read/write paths for the schema-validator.
//!
//! Two reads (catalog cursor + read-one-by-key for debounce/per-write) and a
//! single write path (`UPDATE metric_catalog SET schema_* ...`).
//!
//! ## Why a cursor instead of `Vec<CatalogRow>` for the startup pass
//!
//! `validate_all` is a per-row pipeline — probe → persist → log — that never
//! revisits a prior row. Materializing the whole catalog into a `Vec` would
//! peak memory proportional to row count for no benefit. The catalog is
//! capped at ≤200 rows in v1 per PRD §6, so OOM is not a present risk, but
//! relying on that cap to hold across future per-tenant scaling is exactly
//! the kind of implicit dependency that breaks silently. Streaming via the
//! SeaORM cursor pins the right shape: at most one row in memory at a time.
//!
//! ## Why raw SQL for the write path
//!
//! `metric_catalog.updated_at` is `ON UPDATE CURRENT_TIMESTAMP`. A naive
//! `SeaORM` update that touches `schema_*` would also bump `updated_at` —
//! that's exactly what DESIGN §3.2 forbids ("a dedicated `SeaORM` update path
//! with `updated_at` excluded"). The standard MariaDB idiom is to assign
//! `updated_at = updated_at` in the SET clause; the column appearing in SET
//! suppresses `ON UPDATE CURRENT_TIMESTAMP`. Achieving the same through
//! `SeaORM`'s `ActiveModel` would require manually injecting the pin
//! expression anyway, so a single bound-parameter `UPDATE` keeps the
//! contract load-bearing in one place.

use std::pin::Pin;

use chrono::{DateTime, Utc};
use futures_util::Stream;
use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};
use uuid::Uuid;

use crate::domain::schema_validator::error_code::SchemaErrorCode;
use crate::domain::schema_validator::status::{SchemaState, SchemaStatus};

#[derive(Debug, Clone, FromQueryResult)]
pub struct CatalogRow {
    pub id: Uuid,
    pub metric_key: String,
    pub schema_status: String,
    pub schema_checked_at: Option<DateTime<Utc>>,
    pub schema_error_code: Option<String>,
}

impl CatalogRow {
    /// Resolve the row's current `(status, error_code)` pair from the raw DB
    /// strings. Returns `None` if either column carries a value outside the
    /// canonical set — which the CHECK forbids, but we degrade gracefully
    /// (treat as `unchecked`) rather than panic.
    #[must_use]
    pub fn current_state(&self) -> SchemaState {
        let status =
            SchemaStatus::from_db_str(&self.schema_status).unwrap_or(SchemaStatus::Unchecked);
        let error_code = self
            .schema_error_code
            .as_deref()
            .and_then(SchemaErrorCode::from_db_str);
        SchemaState { status, error_code }
    }
}

/// Count rows in `metric_catalog`. Used as a cheap empty-catalog gate before
/// `validate_all` enters its ClickHouse-wait loop — without it, an empty
/// catalog paired with an unreachable ClickHouse would have the validator
/// retrying forever on a no-op bulk mark.
///
/// # Errors
///
/// Propagates `SeaORM` connection / query errors. A missing COUNT row maps
/// to [`sea_orm::DbErr::Custom`] (defensive — `COUNT(*)` always returns
/// exactly one row).
pub async fn count_all(db: &DatabaseConnection) -> Result<u64, sea_orm::DbErr> {
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "SELECT COUNT(*) AS n FROM metric_catalog",
        ))
        .await?
        .ok_or_else(|| sea_orm::DbErr::Custom("COUNT(*) returned no row".to_owned()))?;
    let n: i64 = row.try_get("", "n")?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// The concrete stream type returned by [`stream_all`]. Aliased to keep the
/// `Pin<Box<dyn …>>` noise out of call sites.
pub type CatalogRowStream<'a> =
    Pin<Box<dyn Stream<Item = Result<CatalogRow, sea_orm::DbErr>> + Send + 'a>>;

/// Open a cursor over every row in `metric_catalog`. The stream borrows `db`
/// for its lifetime and yields `(id, metric_key, schema_*)` triples one row
/// at a time, so callers never hold more than one row in memory.
///
/// # Errors
///
/// Propagates `SeaORM` connection / query errors raised at stream-open time.
/// Per-row errors surface through `Stream::Item = Result<_, DbErr>`.
pub async fn stream_all(db: &DatabaseConnection) -> Result<CatalogRowStream<'_>, sea_orm::DbErr> {
    CatalogRow::find_by_statement(Statement::from_string(
        db.get_database_backend(),
        "SELECT id, metric_key, schema_status, schema_checked_at, schema_error_code \
         FROM metric_catalog",
    ))
    .stream(db)
    .await
}

/// Read one row by `metric_key`. Returns `None` when the row doesn't exist —
/// the per-write hook treats that as `MetricUnknown` rather than an error,
/// because the validator never blocks writes.
///
/// # Errors
///
/// Propagates `SeaORM` connection / query errors.
pub async fn find_by_metric_key(
    db: &DatabaseConnection,
    metric_key: &str,
) -> Result<Option<CatalogRow>, sea_orm::DbErr> {
    CatalogRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT id, metric_key, schema_status, schema_checked_at, schema_error_code \
         FROM metric_catalog WHERE metric_key = ?",
        [Value::from(metric_key)],
    ))
    .one(db)
    .await
}

/// Persist a single row's `schema_*` triple, pinning `updated_at` so MariaDB's
/// `ON UPDATE CURRENT_TIMESTAMP` doesn't fire.
///
/// Per the DB CHECK biconditional `(status = 'error') = (error_code IS NOT NULL)`,
/// `error_code` is forced to `NULL` whenever `status` is not `Error`, even if
/// the in-memory `SchemaState` carries a code (e.g., an `unchecked` row whose
/// reason is `ClickhouseUnreachable` — the reason goes to the log, not the DB).
///
/// # Errors
///
/// Propagates `SeaORM` connection / query errors.
pub async fn update_schema_columns(
    db: &DatabaseConnection,
    id: Uuid,
    state: SchemaState,
    checked_at: DateTime<Utc>,
) -> Result<(), sea_orm::DbErr> {
    let db_status = state.status.as_db_str();
    let db_error_code = db_error_code_for(state);

    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE metric_catalog \
         SET schema_status = ?, \
             schema_checked_at = ?, \
             schema_error_code = ?, \
             updated_at = updated_at \
         WHERE id = ?",
        [
            Value::from(db_status),
            Value::from(checked_at),
            match db_error_code {
                Some(c) => Value::from(c),
                None => Value::from(Option::<String>::None),
            },
            Value::from(id),
        ],
    ))
    .await?;
    Ok(())
}

/// Project a [`SchemaState`] to the value bound into `schema_error_code` on
/// the wire, enforcing the DB CHECK biconditional `(status = 'error') ⇔
/// (error_code IS NOT NULL)`: any in-memory `error_code` carried alongside a
/// non-`Error` status (e.g., the unchecked-with-`ClickhouseUnreachable`
/// reason used by transition logs) is coerced to `None` on write. Extracted
/// so the test below can pin the coercion without a live MariaDB.
#[must_use]
fn db_error_code_for(state: SchemaState) -> Option<&'static str> {
    match state.status {
        SchemaStatus::Error => state.error_code.map(SchemaErrorCode::as_db_str),
        SchemaStatus::Ok | SchemaStatus::Unchecked => None,
    }
}

/// Bulk-mark every catalog row as `(unchecked, now, NULL)`. Used by the startup
/// retry loop when ClickHouse is unreachable so the read endpoint reflects
/// degraded mode without paying a per-row update during the storm.
///
/// `error_code` is `NULL` on the wire (CHECK biconditional); the
/// `clickhouse_unreachable` reason lands in the structured summary log.
///
/// # Errors
///
/// Propagates `SeaORM` connection / query errors.
pub async fn mark_all_unchecked(
    db: &DatabaseConnection,
    checked_at: DateTime<Utc>,
) -> Result<u64, sea_orm::DbErr> {
    let exec = db
        .execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE metric_catalog \
             SET schema_status = 'unchecked', \
                 schema_checked_at = ?, \
                 schema_error_code = NULL, \
                 updated_at = updated_at",
            [Value::from(checked_at)],
        ))
        .await?;
    Ok(exec.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_state_falls_back_to_unchecked_for_unknown_status() {
        let row = CatalogRow {
            id: Uuid::nil(),
            metric_key: "a.b".to_owned(),
            schema_status: "not_a_status".to_owned(),
            schema_checked_at: None,
            schema_error_code: None,
        };
        // Don't panic on a column value the CHECK forbids — degrade gracefully.
        assert_eq!(row.current_state().status, SchemaStatus::Unchecked);
        assert_eq!(row.current_state().error_code, None);
    }

    #[test]
    fn current_state_parses_canonical_error_code() {
        let row = CatalogRow {
            id: Uuid::nil(),
            metric_key: "a.b".to_owned(),
            schema_status: "error".to_owned(),
            schema_checked_at: None,
            schema_error_code: Some("column_not_found".to_owned()),
        };
        let s = row.current_state();
        assert_eq!(s.status, SchemaStatus::Error);
        assert_eq!(s.error_code, Some(SchemaErrorCode::ColumnNotFound));
    }

    #[test]
    fn db_error_code_for_enforces_biconditional() {
        // status=Ok → NULL regardless of any carried code.
        assert_eq!(db_error_code_for(SchemaState::ok()), None);
        // status=Unchecked → NULL even though the in-memory state may carry
        // a reason (e.g., the degraded-mode mark). The reason goes to logs.
        assert_eq!(db_error_code_for(SchemaState::unchecked()), None);
        // status=Error → the canonical code string.
        assert_eq!(
            db_error_code_for(SchemaState::error(SchemaErrorCode::ColumnNotFound)),
            Some("column_not_found")
        );
        assert_eq!(
            db_error_code_for(SchemaState::error(SchemaErrorCode::TableNotFound)),
            Some("table_not_found")
        );
        // Defensive: an Error state with None error_code shouldn't be
        // constructable through `SchemaState::error()`, but if it ever is
        // (manual struct init), we'd write NULL — which then violates the
        // biconditional. The CHECK at the DB layer would reject the write.
        // This row pins that the projection itself returns None in that case
        // so the failure surfaces as a DB CHECK violation, not silent.
        let degenerate = SchemaState {
            status: SchemaStatus::Error,
            error_code: None,
        };
        assert_eq!(db_error_code_for(degenerate), None);
    }

    #[test]
    fn current_state_drops_noncanonical_error_code() {
        // A raw-SQL writer that snuck a value past the CHECK shouldn't poison
        // the in-memory state.
        let row = CatalogRow {
            id: Uuid::nil(),
            metric_key: "a.b".to_owned(),
            schema_status: "error".to_owned(),
            schema_error_code: Some("connection refused".to_owned()),
            schema_checked_at: None,
        };
        assert_eq!(row.current_state().status, SchemaStatus::Error);
        assert_eq!(row.current_state().error_code, None);
    }
}
