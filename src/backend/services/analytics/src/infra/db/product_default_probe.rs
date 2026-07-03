//! Startup invariant: every `is_enabled = true` `metric_catalog` row has a
//! matching `product-default` `metric_threshold` row (Refs #523).
//!
//! The resolver chain in `cpt-metric-cat-component-threshold-resolver`
//! always walks down to `product-default` as its floor — if that floor is
//! missing for an enabled metric, the resolver returns no threshold and the
//! bullet renders without a status color, breaking the byte-for-byte FE
//! comparison gate from PRD §12. We catch that at boot rather than at the
//! first read.
//!
//! Same shape as `check_probe::assert_required_checks`:
//! - Single SQL probe at startup.
//! - Aggregates every offending row into one error so one bad deploy
//!   surfaces every gap (no N-restart fishing).
//! - Returns `Err` → main bails out before serving traffic.
//!
//! Failure modes this probe catches:
//! - Seed migration ships a row in `metric_catalog` without a matching
//!   `metric_threshold` (developer forgets, or a partial seed gets
//!   replayed).
//! - An operator runs `DELETE FROM metric_threshold WHERE scope =
//!   'product-default' ...` to recover from an incident and forgets to
//!   re-insert.
//! - A future migration introduces a new `is_enabled = true` metric but
//!   forgets the matching `product-default` row.

use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement};

/// SQL behind the probe. Surfaced as a module constant so a unit test can
/// pin its shape against silent string drift — the probe is load-bearing
/// at boot and we never want a refactor to quietly degrade it to a no-op
/// (e.g., a stray `OR` flipping the join into a true predicate, a typo
/// dropping the `is_enabled` filter, etc.).
// `is_enabled = TRUE` is load-bearing: disabled metrics (deprecation
// path per `cpt-metric-cat-fr-enable-flag`) are deliberately exempt from
// the product-default invariant — the read endpoint already filters them
// out. The threshold-side predicates live in the `ON` clause so the
// LEFT JOIN keeps non-matching catalog rows visible; moving them to
// `WHERE` would silently collapse this into an inner join and the probe
// would never report a gap.
const MISSING_PRODUCT_DEFAULT_SQL: &str = "\
    SELECT mc.metric_key AS metric_key \
    FROM metric_catalog mc \
    LEFT JOIN metric_threshold mt \
      ON mt.metric_key = mc.metric_key \
     AND mt.scope = 'product-default' \
     AND mt.tenant_id IS NULL \
    WHERE mc.is_enabled = TRUE \
      AND mt.id IS NULL";

#[derive(FromQueryResult)]
struct MissingRow {
    metric_key: String,
}

/// Verify the product-default-row invariant.
///
/// # Errors
///
/// Returns an error listing every offending `metric_key` if any enabled
/// catalog row lacks a `product-default` `metric_threshold`. Returns an
/// error if the probe query itself fails.
pub async fn assert_product_default_present(db: &DatabaseConnection) -> anyhow::Result<()> {
    let rows = MissingRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        MISSING_PRODUCT_DEFAULT_SQL,
        [],
    ))
    .all(db)
    .await?;

    if rows.is_empty() {
        tracing::info!(
            "product-default probe: every enabled metric_catalog row has a \
             matching product-default metric_threshold"
        );
        return Ok(());
    }

    let summary = rows
        .iter()
        .map(|r| r.metric_key.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    tracing::error!(
        missing = %summary,
        count = rows.len(),
        "enabled metric_catalog rows missing product-default metric_threshold — \
         refusing to start (cpt-metric-cat-fr-tenant-thresholds resolver \
         floor would be empty for these metrics)"
    );

    Err(anyhow::anyhow!(
        "metric_catalog rows missing product-default metric_threshold: {summary}. \
         The threshold resolver requires a product-default floor for every \
         enabled metric (see DESIGN §3.6 seed-migration + \
         cpt-metric-cat-fr-tenant-thresholds)."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_sql_filters_to_enabled_rows_only() {
        // Regression guard: dropping the `is_enabled = TRUE` clause would
        // make the probe fail on every legitimately disabled metric.
        assert!(
            MISSING_PRODUCT_DEFAULT_SQL.contains("mc.is_enabled = TRUE"),
            "probe must scope to enabled rows — disabled metrics are \
             exempt from the product-default invariant"
        );
    }

    #[test]
    fn probe_sql_scopes_threshold_join_to_product_default_nulltenant() {
        // The join MUST be narrowed to (`scope = 'product-default'` AND
        // `tenant_id IS NULL`). A weaker join would treat any tenant-scope
        // row as "covered" and let a missing product-default sneak past
        // the probe.
        assert!(
            MISSING_PRODUCT_DEFAULT_SQL.contains("mt.scope = 'product-default'"),
            "probe must match only product-default thresholds"
        );
        assert!(
            MISSING_PRODUCT_DEFAULT_SQL.contains("mt.tenant_id IS NULL"),
            "probe must match only NULL-tenant rows (product-default is \
             defined as tenant_id IS NULL per DESIGN §3.7)"
        );
    }

    #[test]
    fn probe_sql_is_a_left_join_with_null_check() {
        // The "missing right side" check requires LEFT JOIN + `mt.id IS NULL`
        // — an INNER JOIN with a `NOT IN (...)` subquery would also work but
        // pins us to a specific shape so a refactor that flips the join type
        // breaks this test before reaching production.
        assert!(
            MISSING_PRODUCT_DEFAULT_SQL.contains("LEFT JOIN metric_threshold"),
            "probe must LEFT JOIN to expose the missing right-side rows"
        );
        assert!(
            MISSING_PRODUCT_DEFAULT_SQL.contains("mt.id IS NULL"),
            "probe must filter to rows where no threshold matched (mt.id IS NULL)"
        );
    }
}
