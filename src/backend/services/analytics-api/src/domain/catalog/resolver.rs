//! `threshold-resolver` (`cpt-metric-cat-component-threshold-resolver`).
//!
//! Exactly one bulk SELECT per request, then an in-memory most-specific-wins
//! walk over `{ product-default, tenant, role, team, team+role }`. Halts on
//! the first locked broader-scope row and sets `bounded_by_lock = true` on
//! the resulting [`ThresholdView`].
//!
//! ## Why one bulk SQL, not one per scope
//!
//! Five separate `SELECT`s would be five round-trips and (per `cpt-metric-cat-nfr-read-latency`)
//! a ≤ 500 ms miss-path budget that's hard to hit with a multi-replica
//! analytics-api over a multi-AZ MariaDB. The single SELECT joins
//! `metric_catalog` (enabled rows) with every `metric_threshold` row whose
//! scope matches the request context for the tenant — at most 5 candidates
//! per metric, so the row count is `O(metrics × scopes)` = `O(≤ 200 × 5)`.
//! The walk runs in-process over that small set.
//!
//! ## The `tenant_id_sentinel` join column
//!
//! `metric_threshold.tenant_id` is `NULL` for `product-default` rows and a real
//! UUID for tenant-scoped rows. SQL's "NULLs are distinct" semantics defeat
//! the UNIQUE composite the schema relies on, so #519 added a STORED generated
//! column `tenant_id_sentinel BINARY(16)` that coalesces NULL → all-zero bytes.
//! The resolver joins against `tenant_id_sentinel` (NOT raw `tenant_id`) so it
//! reuses the same composite UNIQUE index that backs writes; querying raw
//! `tenant_id` here would force a wider scan for no benefit.

use std::collections::HashMap;

use chrono::Utc;
use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};
use uuid::Uuid;

use crate::domain::catalog::response::{CatalogResponse, MetricView, ThresholdView};

/// Ordered broad→narrow. The walk halts on the first locked row in this order
/// (with `bounded_by_lock = true` on the result). Otherwise the most-specific
/// matching scope wins.
const SCOPE_ORDER: &[Scope] = &[
    Scope::ProductDefault,
    Scope::Tenant,
    Scope::Role,
    Scope::Team,
    Scope::TeamRole,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    ProductDefault,
    Tenant,
    Role,
    Team,
    TeamRole,
}

impl Scope {
    fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "product-default" => Some(Self::ProductDefault),
            "tenant" => Some(Self::Tenant),
            "role" => Some(Self::Role),
            "team" => Some(Self::Team),
            "team+role" => Some(Self::TeamRole),
            _ => None,
        }
    }

    fn as_wire_str(self) -> &'static str {
        match self {
            Self::ProductDefault => "product-default",
            Self::Tenant => "tenant",
            Self::Role => "role",
            Self::Team => "team",
            Self::TeamRole => "team+role",
        }
    }
}

/// Threshold-resolver — owns the bulk SQL and the in-memory walk.
#[derive(Clone)]
pub struct ThresholdResolver {
    db: DatabaseConnection,
}

impl ThresholdResolver {
    #[must_use]
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Issue the single bulk fetch + run the per-metric walk. Returns the
    /// full [`CatalogResponse`] ready for the cache + the wire.
    ///
    /// `role_slug` / `team_id` use the **canonical empty-string sentinel**:
    /// `None` is converted to `""` before binding so the SQL chain matches
    /// exactly the rows the schema's UNIQUE composite allows. A literal `""`
    /// supplied by the caller is the same as `None` (the cache key shows the
    /// same equivalence — see `cache_key`).
    ///
    /// # Errors
    ///
    /// Propagates SeaORM connection / query errors.
    pub async fn resolve(
        &self,
        tenant_id: Uuid,
        role_slug: &str,
        team_id: &str,
    ) -> Result<CatalogResponse, sea_orm::DbErr> {
        let rows = bulk_fetch(&self.db, tenant_id, role_slug, team_id).await?;
        let metrics = walk_all(rows);
        Ok(CatalogResponse {
            tenant_id,
            generated_at: Utc::now(),
            metrics,
        })
    }
}

/// One row of the bulk fetch — a `(metric, candidate threshold)` pair. Each
/// metric returns up to 5 rows (one per scope chain it matches) plus 1 if it
/// has no matching threshold (the LEFT JOIN side); the walk dedupes that case
/// by requiring at least the `product-default` row to win.
#[derive(Debug, Clone, FromQueryResult)]
struct ResolverRow {
    // Catalog columns (repeat per row — denormalized for one round-trip).
    metric_id: Uuid,
    label: String,
    sublabel: Option<String>,
    description: Option<String>,
    unit: Option<String>,
    format: Option<String>,
    higher_is_better: bool,
    is_member_scale: bool,
    /// JSON array text — parsed in-process. Always an array per the DB CHECK.
    source_tags: String,
    schema_status: String,
    schema_error_code: Option<String>,

    // Threshold columns (NULL on no-match for that scope chain).
    scope: Option<String>,
    /// DECIMAL(20,6) in MariaDB — cast to DOUBLE in SQL to give us a clean
    /// `f64` here (per Q2 ack — float is acceptable; full-precision decimal
    /// is deferred to a separate fix if drift surfaces against the FE
    /// byte-for-byte gate).
    good: Option<f64>,
    warn: Option<f64>,
    alert_trigger: Option<f64>,
    alert_bad: Option<f64>,
    is_locked: Option<bool>,
}

/// Issue the bulk SELECT. One round-trip, regardless of metric count.
async fn bulk_fetch(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    role_slug: &str,
    team_id: &str,
) -> Result<Vec<ResolverRow>, sea_orm::DbErr> {
    let backend = db.get_database_backend();
    // The five scope arms are spelled out to make the index plan obvious to
    // anyone reading EXPLAIN: each arm hits the
    // `uq_metric_threshold_scope_target` UNIQUE composite via
    // `(tenant_id_sentinel, metric_key, scope, role_slug, team_id)`.
    //
    // `tenant_id_sentinel` is BINARY(16); we bind the tenant UUID as bytes.
    // For `product-default` rows the stored sentinel is all-zero (COALESCE on
    // NULL `tenant_id`), so we never need to bind a special value for that
    // arm — it just won't match the real-tenant arms by construction.
    //
    // The catalog side filters `c.is_enabled = TRUE` so disabled rows never
    // even reach the walk. `c.tenant_id IS NULL` honors the v1 CHECK
    // (`cpt-metric-cat-fr-metadata-writes` — catalog rows are product-owned).
    let sql = "\
        SELECT \
            c.id                        AS metric_id, \
            c.label                     AS label, \
            c.sublabel                  AS sublabel, \
            c.description               AS description, \
            c.unit                      AS unit, \
            c.format                    AS format, \
            c.higher_is_better          AS higher_is_better, \
            c.is_member_scale           AS is_member_scale, \
            CAST(c.source_tags AS CHAR) AS source_tags, \
            c.schema_status             AS schema_status, \
            c.schema_error_code         AS schema_error_code, \
            t.scope                     AS scope, \
            CAST(t.good          AS DOUBLE) AS good, \
            CAST(t.warn          AS DOUBLE) AS warn, \
            CAST(t.alert_trigger AS DOUBLE) AS alert_trigger, \
            CAST(t.alert_bad     AS DOUBLE) AS alert_bad, \
            t.is_locked                 AS is_locked \
        FROM metric_catalog c \
        LEFT JOIN metric_threshold t \
            ON t.metric_key = c.metric_key \
           AND ( \
                  (t.scope = 'product-default') \
               OR (t.scope = 'tenant'    AND t.tenant_id_sentinel = ?) \
               OR (t.scope = 'role'      AND t.tenant_id_sentinel = ? AND t.role_slug = ?) \
               OR (t.scope = 'team'      AND t.tenant_id_sentinel = ? AND t.team_id   = ?) \
               OR (t.scope = 'team+role' AND t.tenant_id_sentinel = ? AND t.role_slug = ? AND t.team_id = ?) \
           ) \
        WHERE c.is_enabled = TRUE \
          AND c.tenant_id IS NULL";

    let tenant_bytes = Value::Bytes(Some(Box::new(tenant_id.as_bytes().to_vec())));

    ResolverRow::find_by_statement(Statement::from_sql_and_values(
        backend,
        sql,
        [
            // tenant scope
            tenant_bytes.clone(),
            // role scope
            tenant_bytes.clone(),
            Value::from(role_slug),
            // team scope
            tenant_bytes.clone(),
            Value::from(team_id),
            // team+role scope
            tenant_bytes,
            Value::from(role_slug),
            Value::from(team_id),
        ],
    ))
    .all(db)
    .await
}

/// Group rows by metric and run the per-metric walk. Drops metrics that have
/// no matching threshold (the resolver's non-null guarantee per
/// `cpt-metric-cat-fr-tenant-thresholds` is enforced at write time by the
/// seed migration; here we simply omit metrics that somehow have no chain).
fn walk_all(rows: Vec<ResolverRow>) -> Vec<MetricView> {
    let mut grouped: HashMap<Uuid, Vec<ResolverRow>> = HashMap::new();
    let mut order: Vec<Uuid> = Vec::new();
    for row in rows {
        let id = row.metric_id;
        let bucket = grouped.entry(id).or_default();
        if bucket.is_empty() {
            order.push(id);
        }
        bucket.push(row);
    }

    let mut out = Vec::with_capacity(order.len());
    for id in order {
        let Some(mut bucket) = grouped.remove(&id) else {
            continue;
        };
        if let Some(view) = walk_one(&mut bucket) {
            out.push(view);
        } else {
            // No matching threshold (not even product-default). Log and skip:
            // surfacing such a row would force the consumer to defend against
            // `thresholds: null` on the wire, and the seed migration is the
            // contract that prevents this from happening in healthy state.
            tracing::warn!(
                metric_id = %id,
                "resolver: no threshold candidates for enabled metric — \
                 product-default seed appears to be missing. Skipping metric \
                 in response."
            );
        }
    }
    out
}

/// Run the most-specific-wins, lock-bounded walk for one metric.
fn walk_one(rows: &mut [ResolverRow]) -> Option<MetricView> {
    if rows.is_empty() {
        return None;
    }

    // Sort by SCOPE_ORDER position so we walk broad→narrow deterministically
    // regardless of how MariaDB returned the join rows.
    rows.sort_by_key(|r| scope_order_index(r.scope.as_deref()));

    // Pull immutable metric metadata from the first row (same across the bucket).
    let head = rows.first()?;
    let metric_id = head.metric_id;
    let label = head.label.clone();
    let sublabel = head.sublabel.clone();
    let description = head.description.clone();
    let unit = head.unit.clone();
    let format = head.format.clone();
    let higher_is_better = head.higher_is_better;
    let is_member_scale = head.is_member_scale;
    let source_tags = parse_source_tags(&head.source_tags);
    let schema_status = head.schema_status.clone();
    let schema_error_code = head.schema_error_code.clone();

    // Walk the candidates. `winner` is the most-specific row seen so far that
    // is NOT shadowed by an earlier-walked lock. `bounded_by_lock` is set when
    // a broader-scope row is locked and we therefore refuse to advance past it.
    let mut winner: Option<&ResolverRow> = None;
    let mut bounded_by_lock = false;
    for r in rows.iter() {
        // Skip the no-threshold LEFT JOIN side (scope IS NULL).
        let Some(scope_str) = r.scope.as_deref() else {
            continue;
        };
        let Some(_scope) = Scope::from_db_str(scope_str) else {
            tracing::warn!(
                metric_id = %metric_id,
                scope = %scope_str,
                "resolver: unknown scope value from DB; ignoring row"
            );
            continue;
        };

        if bounded_by_lock {
            // Already halted on a broader-scope lock; narrower rows are
            // shadowed by definition.
            continue;
        }

        // This row becomes the new candidate winner (more specific than the
        // previous, by virtue of the broad→narrow sort).
        winner = Some(r);

        // If this row is locked, the walk halts here: any narrower row is
        // shadowed. `winner` keeps this row as the result.
        if r.is_locked.unwrap_or(false) {
            bounded_by_lock = true;
        }
    }

    let win = winner?;
    let scope = Scope::from_db_str(win.scope.as_deref()?)?;

    // `good` / `warn` are NOT NULL on the DB side; surfacing NULL here would
    // be a DB-shape regression. Defensive: log + drop the metric rather than
    // emit a malformed wire payload.
    let (Some(good), Some(warn)) = (win.good, win.warn) else {
        tracing::error!(
            metric_id = %metric_id,
            "resolver: winning threshold row has NULL good/warn (DB invariant violated)"
        );
        return None;
    };

    Some(MetricView {
        id: metric_id,
        label,
        sublabel,
        description,
        unit,
        format,
        higher_is_better,
        is_member_scale,
        source_tags,
        schema_status,
        schema_error_code,
        thresholds: ThresholdView {
            good,
            warn,
            alert_trigger: win.alert_trigger,
            alert_bad: win.alert_bad,
            resolved_from: scope.as_wire_str().to_owned(),
            bounded_by_lock,
        },
    })
}

/// Index into [`SCOPE_ORDER`] so we can sort broad→narrow. Unknown / NULL
/// scopes sort to the end so they are naturally ignored by the walk.
fn scope_order_index(scope: Option<&str>) -> usize {
    let Some(scope) = scope.and_then(Scope::from_db_str) else {
        return SCOPE_ORDER.len();
    };
    SCOPE_ORDER
        .iter()
        .position(|s| *s == scope)
        .unwrap_or(SCOPE_ORDER.len())
}

/// Parse a JSON array string from `metric_catalog.source_tags`. Any parse
/// failure degrades to an empty array — the DB CHECK ensures the column is
/// well-formed JSON, so this is purely defensive.
fn parse_source_tags(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_else(|e| {
        tracing::warn!(
            error = %e,
            raw = %raw,
            "resolver: source_tags is not a JSON string array; surfacing []"
        );
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(scope: Option<&str>, is_locked: bool, good: f64, warn: f64) -> ResolverRow {
        ResolverRow {
            metric_id: Uuid::nil(),
            label: "L".to_owned(),
            sublabel: None,
            description: None,
            unit: None,
            format: None,
            higher_is_better: true,
            is_member_scale: false,
            source_tags: "[]".to_owned(),
            schema_status: "ok".to_owned(),
            schema_error_code: None,
            scope: scope.map(str::to_owned),
            good: scope.map(|_| good),
            warn: scope.map(|_| warn),
            alert_trigger: None,
            alert_bad: None,
            is_locked: scope.map(|_| is_locked),
        }
    }

    fn must_resolve(rows: &mut [ResolverRow]) -> MetricView {
        walk_one(rows).unwrap_or_else(|| panic!("walk_one must resolve a row"))
    }

    #[test]
    fn walk_picks_most_specific_when_no_lock() {
        // product-default + tenant + team — narrower wins.
        let mut rows = vec![
            row(Some("team"), false, 30.0, 15.0),
            row(Some("product-default"), false, 10.0, 5.0),
            row(Some("tenant"), false, 20.0, 10.0),
        ];
        let v = must_resolve(&mut rows);
        assert_eq!(v.thresholds.resolved_from, "team");
        assert!(!v.thresholds.bounded_by_lock);
        assert!((v.thresholds.good - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn walk_halts_on_broader_lock_and_sets_bounded_by_lock() {
        // tenant row is locked; team+role row exists but is shadowed.
        let mut rows = vec![
            row(Some("team+role"), false, 99.0, 88.0),
            row(Some("tenant"), true, 20.0, 10.0),
            row(Some("product-default"), false, 10.0, 5.0),
        ];
        let v = must_resolve(&mut rows);
        assert_eq!(
            v.thresholds.resolved_from, "tenant",
            "walk MUST halt on the locked tenant row"
        );
        assert!(
            v.thresholds.bounded_by_lock,
            "bounded_by_lock MUST be true when a broader lock shadowed a narrower candidate"
        );
        assert!((v.thresholds.good - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn walk_returns_product_default_when_only_seed_present() {
        let mut rows = vec![row(Some("product-default"), false, 10.0, 5.0)];
        let v = must_resolve(&mut rows);
        assert_eq!(v.thresholds.resolved_from, "product-default");
        assert!(!v.thresholds.bounded_by_lock);
    }

    #[test]
    fn walk_ignores_no_match_rows_with_null_scope() {
        // The LEFT JOIN emits a single row with NULL scope when no threshold
        // matches. With ONE such row, the walk yields None (catalog row has
        // no chain — the resolver logs + skips). Pair it with a real
        // product-default row → resolver still works.
        let mut rows = vec![
            row(None, false, 0.0, 0.0),
            row(Some("product-default"), false, 1.0, 0.5),
        ];
        let v = must_resolve(&mut rows);
        assert_eq!(v.thresholds.resolved_from, "product-default");
    }

    #[test]
    fn walk_returns_none_when_only_null_scope_rows() {
        // No matching threshold chain at all → resolver omits the metric.
        let mut rows = vec![row(None, false, 0.0, 0.0)];
        assert!(walk_one(&mut rows).is_none());
    }

    #[test]
    fn walk_handles_unsorted_input() {
        // Confirm the sort is what makes the walk correct — feed narrowest first.
        let mut rows = vec![
            row(Some("team+role"), false, 4.0, 2.0),
            row(Some("team"), false, 3.0, 1.5),
            row(Some("role"), false, 2.5, 1.25),
            row(Some("tenant"), false, 2.0, 1.0),
            row(Some("product-default"), false, 1.0, 0.5),
        ];
        let v = must_resolve(&mut rows);
        assert_eq!(v.thresholds.resolved_from, "team+role");
        assert!(!v.thresholds.bounded_by_lock);
    }

    #[test]
    fn walk_lock_at_product_default_bounds_everything() {
        // v1 allows `is_locked` only on `product-default` / `tenant`. A
        // locked `product-default` shadows the entire chain — `bounded_by_lock = true`,
        // `resolved_from = product-default`.
        let mut rows = vec![
            row(Some("tenant"), false, 50.0, 25.0),
            row(Some("product-default"), true, 10.0, 5.0),
        ];
        let v = must_resolve(&mut rows);
        assert_eq!(v.thresholds.resolved_from, "product-default");
        assert!(v.thresholds.bounded_by_lock);
    }

    #[test]
    fn scope_order_index_unknown_sorts_to_end() {
        assert_eq!(scope_order_index(None), SCOPE_ORDER.len());
        assert_eq!(scope_order_index(Some("garbage")), SCOPE_ORDER.len());
        assert_eq!(scope_order_index(Some("product-default")), 0);
        assert_eq!(scope_order_index(Some("team+role")), 4);
    }

    #[test]
    fn parse_source_tags_empty_array() {
        assert_eq!(parse_source_tags("[]"), Vec::<String>::new());
    }

    #[test]
    fn parse_source_tags_well_formed() {
        assert_eq!(
            parse_source_tags(r#"["jira","github"]"#),
            vec!["jira".to_owned(), "github".to_owned()]
        );
    }

    #[test]
    fn parse_source_tags_malformed_degrades_to_empty() {
        // DB CHECK should prevent this, but defense in depth: malformed JSON
        // must not crash the request.
        assert_eq!(parse_source_tags("not json"), Vec::<String>::new());
    }
}
