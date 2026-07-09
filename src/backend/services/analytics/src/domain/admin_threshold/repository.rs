//! Repository — SQL access against `metric_threshold` + the joined
//! `metric_catalog.schema_*` read.
//!
//! All SELECTs use raw SQL with `CAST(... AS DOUBLE)` for the DECIMAL
//! threshold columns. sea-orm 1.1.20's entity decoder refuses to read
//! `DECIMAL` into `f64` (rejects with `mismatched types; Rust type
//! Option<f64> ... not compatible with SQL type DECIMAL`); the
//! resolver's `bulk_fetch` already takes this same approach
//! (`CAST(t.good AS DOUBLE) AS good`). Inserts/updates use the typed
//! `ActiveModel` for encoding-side safety, but the post-write read for
//! the response payload always goes through the raw-SQL `select_by_id`
//! so we don't trip the same decoder path.

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveValue, ConnectionTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    FromQueryResult, Statement, TransactionTrait, Value,
};
use uuid::Uuid;

use crate::domain::admin_threshold::dto::{ListFilters, Scope, ThresholdView};
use crate::infra::db::entities::metric_threshold;

/// Projection of `metric_catalog` for the gauntlet's pre-write checks.
/// Carries every column admin-crud needs in one round-trip:
///
/// - `metric_key` — used by lock-enforcer, schema-validator hook, audit.
/// - `is_enabled` — referential-integrity gate (reject `UNKNOWN_OR_DISABLED`).
/// - `higher_is_better` — sanity-bound direction (no second query).
/// - `schema_status` / `schema_error_code` — surfaced on the response view.
#[derive(Debug, Clone, FromQueryResult)]
pub struct CatalogLookup {
    pub metric_key: String,
    pub is_enabled: bool,
    pub higher_is_better: bool,
    pub schema_status: String,
    pub schema_error_code: Option<String>,
}

/// `SELECT metric_key, is_enabled, higher_is_better, schema_status,
///  schema_error_code FROM metric_catalog WHERE id = ?`. Returns
/// `Ok(None)` for unknown `metric_id` so the gauntlet can emit
/// `invalid_argument` with `reason = UNKNOWN_OR_DISABLED`.
///
/// Raw SQL (not the SeaORM entity) because the typed `metric_catalog::Model`
/// declared in #519 only exposes the columns the schema-validator
/// touches — adding `is_enabled` / `higher_is_better` would reshape an
/// entity owned by another component for two booleans.
///
/// # Errors
///
/// Surfaces SeaORM connection / decode errors. Caller maps to a 5xx.
pub async fn find_metric_catalog(
    db: &DatabaseConnection,
    metric_id: Uuid,
) -> Result<Option<CatalogLookup>, sea_orm::DbErr> {
    let backend = db.get_database_backend();
    let stmt = Statement::from_sql_and_values(
        backend,
        "SELECT metric_key, is_enabled, higher_is_better, schema_status, schema_error_code \
         FROM metric_catalog WHERE id = ?",
        [Value::Bytes(Some(Box::new(metric_id.as_bytes().to_vec())))],
    );
    CatalogLookup::find_by_statement(stmt).one(db).await
}

/// Reverse lookup: find the `metric_catalog` row whose `metric_key`
/// matches `metric_key` — used by the GET-by-id + list paths to attach
/// the `schema_status` join columns and the `metric_id` UUID to each
/// `ThresholdView`.
///
/// # Errors
///
/// Surfaces SeaORM connection / decode errors.
pub async fn find_catalog_id_for_metric_key(
    db: &DatabaseConnection,
    metric_key: &str,
) -> Result<Option<CatalogJoinRow>, sea_orm::DbErr> {
    let backend = db.get_database_backend();
    let stmt = Statement::from_sql_and_values(
        backend,
        "SELECT id, higher_is_better, schema_status, schema_error_code \
         FROM metric_catalog WHERE metric_key = ?",
        [Value::from(metric_key)],
    );
    CatalogJoinRow::find_by_statement(stmt).one(db).await
}

#[derive(Debug, Clone, FromQueryResult)]
pub struct CatalogJoinRow {
    pub id: Uuid,
    /// Carried so the PUT path can do sanity-bound validation without a
    /// second round-trip (`good` vs `warn` ordering depends on the
    /// metric's `higher_is_better` flag).
    pub higher_is_better: bool,
    pub schema_status: String,
    pub schema_error_code: Option<String>,
}

/// Row shape returned by every `metric_threshold` SELECT in this module.
/// Numeric columns are `CAST(... AS DOUBLE)` server-side so the decoder
/// reads them straight into `f64` without sea-orm's
/// DECIMAL→f64-rejection (see module doc).
#[derive(Debug, Clone, FromQueryResult)]
pub struct ThresholdRow {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub metric_key: String,
    pub scope: String,
    pub role_slug: String,
    pub team_id: String,
    pub good: f64,
    pub warn: f64,
    pub alert_trigger: Option<f64>,
    pub alert_bad: Option<f64>,
    pub is_locked: bool,
    pub locked_by: Option<String>,
    pub locked_at: Option<DateTime<Utc>>,
    pub lock_reason: Option<String>,
}

/// Common SELECT body — every per-id / list query in this module joins
/// the same column projection.
const THRESHOLD_SELECT_COLS: &str = "SELECT \
    id                                 AS id, \
    tenant_id                          AS tenant_id, \
    metric_key                         AS metric_key, \
    scope                              AS scope, \
    role_slug                          AS role_slug, \
    team_id                            AS team_id, \
    CAST(good          AS DOUBLE)      AS good, \
    CAST(warn          AS DOUBLE)      AS warn, \
    CAST(alert_trigger AS DOUBLE)      AS alert_trigger, \
    CAST(alert_bad     AS DOUBLE)      AS alert_bad, \
    is_locked                          AS is_locked, \
    locked_by                          AS locked_by, \
    locked_at                          AS locked_at, \
    lock_reason                        AS lock_reason \
    FROM metric_threshold";

/// Fetch a threshold row by id. Returns `Ok(None)` when missing —
/// caller emits `not_found`.
///
/// # Errors
///
/// Surfaces SeaORM connection / decode errors.
pub async fn find_threshold<C: ConnectionTrait>(
    conn: &C,
    id: Uuid,
) -> Result<Option<ThresholdRow>, sea_orm::DbErr> {
    let backend = conn.get_database_backend();
    let stmt = Statement::from_sql_and_values(
        backend,
        format!("{THRESHOLD_SELECT_COLS} WHERE id = ?"),
        [Value::Bytes(Some(Box::new(id.as_bytes().to_vec())))],
    );
    ThresholdRow::find_by_statement(stmt).one(conn).await
}

/// List threshold rows for `tenant_id`, applying the in-spec filter
/// set. Filters are bound by name not by position — we render the
/// dynamic WHERE in Rust based on which filters are present.
///
/// **`product-default` rows are NOT listed for tenant callers.** DESIGN
/// §3.3 lists are scoped to "the caller's tenant"; product-default
/// rows have `tenant_id IS NULL` and are intentionally excluded by
/// the `tenant_id = ?` predicate.
///
/// # Errors
///
/// Surfaces SeaORM connection / decode errors.
pub async fn list_thresholds(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    filters: &ListFilters,
) -> Result<Vec<ThresholdRow>, sea_orm::DbErr> {
    // `metric_id` filter resolves to a `metric_key` first; if the id
    // doesn't exist, the list is empty (consistent with filtering on
    // a non-existent key).
    let metric_key_filter = match filters.metric_id {
        Some(mid) => match find_metric_catalog(db, mid).await? {
            Some(cat) => Some(cat.metric_key),
            None => return Ok(Vec::new()),
        },
        None => None,
    };

    let mut where_clauses: Vec<&'static str> = vec!["tenant_id = ?"];
    let mut values: Vec<Value> = vec![Value::Bytes(Some(Box::new(tenant_id.as_bytes().to_vec())))];

    if let Some(mk) = metric_key_filter {
        where_clauses.push("metric_key = ?");
        values.push(Value::from(mk));
    }
    if let Some(scope) = filters.scope {
        where_clauses.push("scope = ?");
        values.push(Value::from(scope.as_db_str()));
    }
    if let Some(role_slug) = filters.role_slug.as_deref() {
        where_clauses.push("role_slug = ?");
        values.push(Value::from(role_slug));
    }
    if let Some(team_id) = filters.team_id.as_deref() {
        where_clauses.push("team_id = ?");
        values.push(Value::from(team_id));
    }

    let where_sql = where_clauses.join(" AND ");
    let sql =
        format!("{THRESHOLD_SELECT_COLS} WHERE {where_sql} ORDER BY scope, role_slug, team_id");
    let stmt = Statement::from_sql_and_values(db.get_database_backend(), sql, values);
    ThresholdRow::find_by_statement(stmt).all(db).await
}

/// Insert a new row inside `tx`. Returns the persisted row via a
/// follow-up `select_by_id` so the caller has the same `ThresholdRow`
/// shape it gets from list / get reads (avoids sea-orm's
/// `exec_with_returning` SELECT path, which trips the DECIMAL decoder).
///
/// # Errors
///
/// Surfaces SeaORM insert / CHECK-violation errors — caller maps to a
/// canonical 4xx via `error_map`.
#[allow(clippy::too_many_arguments)] // single-row INSERT, every column is meaningful
pub async fn insert_threshold(
    tx: &DatabaseTransaction,
    id: Uuid,
    tenant_id: Uuid,
    metric_key: &str,
    scope: Scope,
    role_slug: &str,
    team_id: &str,
    good: f64,
    warn: f64,
    alert_trigger: Option<f64>,
    alert_bad: Option<f64>,
    is_locked: bool,
    locked_by: Option<String>,
    locked_at: Option<DateTime<Utc>>,
    lock_reason: Option<String>,
) -> Result<ThresholdRow, sea_orm::DbErr> {
    let now = Utc::now();
    let model = metric_threshold::ActiveModel {
        id: ActiveValue::Set(id),
        tenant_id: ActiveValue::Set(Some(tenant_id)),
        metric_key: ActiveValue::Set(metric_key.to_owned()),
        scope: ActiveValue::Set(scope.as_db_str().to_owned()),
        role_slug: ActiveValue::Set(role_slug.to_owned()),
        team_id: ActiveValue::Set(team_id.to_owned()),
        good: ActiveValue::Set(good),
        warn: ActiveValue::Set(warn),
        alert_trigger: ActiveValue::Set(alert_trigger),
        alert_bad: ActiveValue::Set(alert_bad),
        is_locked: ActiveValue::Set(is_locked),
        locked_by: ActiveValue::Set(locked_by),
        locked_at: ActiveValue::Set(locked_at),
        lock_reason: ActiveValue::Set(lock_reason),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
    };
    // `.exec()` returns the affected-row count + last-insert-id; no
    // SELECT happens here, so the DECIMAL decoder isn't reached.
    metric_threshold::Entity::insert(model).exec(tx).await?;
    // Follow-up read through our CAST-based SELECT so the caller gets
    // the same `ThresholdRow` shape as list / get paths.
    find_threshold(tx, id).await?.ok_or_else(|| {
        sea_orm::DbErr::RecordNotFound(format!("metric_threshold {id} (post-insert)"))
    })
}

/// Update the mutable fields of an existing row. `scope` / `role_slug`
/// / `team_id` are immutable post-create — the gauntlet rejects PUTs
/// that change them BEFORE this is called, so they're not parameters.
/// Uses raw SQL `UPDATE` so the SELECT-side DECIMAL decoder isn't
/// reached.
///
/// # Errors
///
/// Surfaces SeaORM update / CHECK-violation errors.
#[allow(clippy::too_many_arguments)]
pub async fn update_threshold(
    tx: &DatabaseTransaction,
    id: Uuid,
    good: f64,
    warn: f64,
    alert_trigger: Option<f64>,
    alert_bad: Option<f64>,
    is_locked: bool,
    locked_by: Option<String>,
    locked_at: Option<DateTime<Utc>>,
    lock_reason: Option<String>,
) -> Result<ThresholdRow, sea_orm::DbErr> {
    // `updated_at` has `ON UPDATE CURRENT_TIMESTAMP` in the schema so
    // we don't set it here — the DB stamps it on its own. The five
    // mutable columns + the four lock columns plus the PK bind.
    let sql = "UPDATE metric_threshold SET \
                 good          = ?, \
                 warn          = ?, \
                 alert_trigger = ?, \
                 alert_bad     = ?, \
                 is_locked     = ?, \
                 locked_by     = ?, \
                 locked_at     = ?, \
                 lock_reason   = ? \
               WHERE id = ?";
    let values: Vec<Value> = vec![
        Value::Double(Some(good)),
        Value::Double(Some(warn)),
        Value::Double(alert_trigger),
        Value::Double(alert_bad),
        Value::Bool(Some(is_locked)),
        Value::String(locked_by.map(Box::new)),
        Value::ChronoDateTimeUtc(locked_at.map(Box::new)),
        Value::String(lock_reason.map(Box::new)),
        Value::Bytes(Some(Box::new(id.as_bytes().to_vec()))),
    ];
    let stmt = Statement::from_sql_and_values(tx.get_database_backend(), sql, values);
    let result = tx.execute(stmt).await?;
    if result.rows_affected() == 0 {
        return Err(sea_orm::DbErr::RecordNotFound(format!(
            "metric_threshold {id}"
        )));
    }
    find_threshold(tx, id).await?.ok_or_else(|| {
        sea_orm::DbErr::RecordNotFound(format!("metric_threshold {id} (post-update)"))
    })
}

/// Delete by id. Returns the number of rows deleted (0 ⇒ not found).
/// Raw SQL for symmetry with insert/update; sea-orm's `delete_by_id`
/// would work but going through one path keeps the rollback story
/// identical.
///
/// # Errors
///
/// Surfaces SeaORM transport / query errors.
pub async fn delete_threshold(tx: &DatabaseTransaction, id: Uuid) -> Result<u64, sea_orm::DbErr> {
    let stmt = Statement::from_sql_and_values(
        tx.get_database_backend(),
        "DELETE FROM metric_threshold WHERE id = ?",
        [Value::Bytes(Some(Box::new(id.as_bytes().to_vec())))],
    );
    Ok(tx.execute(stmt).await?.rows_affected())
}

/// Begin a transaction. Thin wrapper so the service code doesn't import
/// `TransactionTrait` directly.
///
/// # Errors
///
/// Surfaces SeaORM transaction begin failures.
pub async fn begin_tx(db: &DatabaseConnection) -> Result<DatabaseTransaction, sea_orm::DbErr> {
    db.begin().await
}

/// Project a `ThresholdRow` + the joined `metric_catalog` row into the
/// wire shape. `role_slug` / `team_id` empty-string sentinels collapse
/// to `None` on the wire so the JSON carries `null` instead of `""`
/// (FE "is this set?" predicates work cleanly).
///
/// # Errors
///
/// Returns `DbErr` when the row's `scope` column is not a known enum
/// value — schema drift the caller surfaces as a 5xx (`get_one`) or
/// skip+log (`list`). Silently coercing to `ProductDefault` would put a
/// misleading scope on the wire.
pub fn view_from_row(
    row: &ThresholdRow,
    catalog: &CatalogJoinRow,
) -> Result<ThresholdView, sea_orm::DbErr> {
    let scope = Scope::from_db_str(&row.scope).ok_or_else(|| {
        sea_orm::DbErr::Custom(format!(
            "metric_threshold row {} has unknown scope {:?} (DB drift)",
            row.id, row.scope
        ))
    })?;
    Ok(ThresholdView {
        id: row.id,
        tenant_id: row.tenant_id,
        metric_id: catalog.id,
        scope,
        role_slug: (!row.role_slug.is_empty()).then(|| row.role_slug.clone()),
        team_id: (!row.team_id.is_empty()).then(|| row.team_id.clone()),
        good: row.good,
        warn: row.warn,
        alert_trigger: row.alert_trigger,
        alert_bad: row.alert_bad,
        is_locked: row.is_locked,
        locked_by: row.locked_by.clone(),
        locked_at: row.locked_at,
        lock_reason: row.lock_reason.clone(),
        schema_status: catalog.schema_status.clone(),
        schema_error_code: catalog.schema_error_code.clone(),
    })
}
