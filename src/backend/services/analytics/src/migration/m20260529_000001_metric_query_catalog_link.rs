//! Create `metric_query_catalog` — junction table linking `metrics` rows
//! (one row = one ClickHouse `query_ref`) to the `metric_catalog` rows
//! whose `metric_key` values that query emits.
//!
//! ## Why this exists
//!
//! Before this migration the only link between a metric's compute side
//! (`metrics.query_ref`, an opaque SQL string) and the catalog side
//! (`metric_catalog`, the consumer-facing metadata + thresholds) was a
//! loose string match on `metric_key`. That worked for resolution but it
//! left two questions structurally unanswerable:
//!
//! 1. Given a `metrics` row, which catalog rows describe what its query
//!    will emit? (One query emits many `metric_key`s: the bullet
//!    queries `SELECT metric_key, value, ... FROM <bullet_rows>` pivot a
//!    row-form storage table into N output rows keyed by `metric_key`.)
//! 2. Given a catalog row, which compute query produces it? (The reverse
//!    of the above; needed for "this metric is broken — which query
//!    backs it" debugging.)
//!
//! Both questions take an M:N relationship, so a single FK column on
//! either side would be wrong. The junction table makes the M:N
//! relationship explicit and lets MariaDB enforce referential integrity
//! (no orphan junction rows; cascaded cleanup when either parent is
//! removed).
//!
//! ## DESIGN amendment
//!
//! `docs/domain/metric-catalog/specs/DESIGN.md` §3.1 ("Metric ↔
//! `analytics.metrics.query_ref` by `metric_key`. Loose pointer; no FK")
//! and §6 integration table ("Read-only (loose pointer) … No FK") both
//! ship pre-amendment. The amendment shipping in the same change as this
//! migration narrows that rule: catalog still does NOT open or parse
//! `query_ref` (opacity preserved per PRD §1.1 layer boundary), and
//! catalog still does NOT carry a `query_ref` column or any compute
//! semantics. The new junction adds a *referential* link only — both
//! sides remain agnostic to each other's payload.
//!
//! The asymmetric warning in `m20260522_000002_metric_threshold` ("If a
//! future revision tightens this, amend §3.7 first via ADR — adding the
//! FK silently is the wrong move") is honored: §3.1 and §6 carry the
//! corresponding amendment notes in the same PR as this migration.
//!
//! ## Backfill strategy
//!
//! `metric_catalog.metric_key` is `<table>.<column>` where `<table>` is
//! the ClickHouse storage table the metric ultimately lives in
//! (`task_delivery_bullet_rows`, `ic_kpis`, etc.). Each `metrics` row's
//! `query_ref` reads one such storage table and pivots it into N output
//! rows. The backfill mapping is a hand-curated
//! `(metrics_hex_id, catalog_table_prefix)` list — derived from reading
//! each `query_ref` and identifying its primary `FROM insight.<table>`
//! source — rather than runtime SQL parsing. Three reasons:
//!
//! - The `query_ref`s are opaque ClickHouse SQL; parsing them in a Rust
//!   migration is fragile and pulls in a SQL parser dependency just to
//!   identify the `FROM` clause.
//! - Some `metrics` rows (`team_member`, `exec_summary`, `ic_chart_*`,
//!   `crm_*`) don't map to any catalog row today — the mapping makes
//!   that explicit (their `metrics_hex_id` simply isn't listed).
//! - The mapping is small (9 entries) and eyeball-reviewable against
//!   the metrics + catalog seed files. Real correctness is verified
//!   by running the migration against MariaDB (`live_tests` / docker /
//!   staging), not by Rust unit tests that re-pin the const.
//!
//! For each `(metrics_hex_id, catalog_table_prefix)` pair, the migration
//! INSERTs one junction row per `metric_catalog` row whose `metric_key`
//! starts with `<table_prefix>.`. The same catalog row may be linked
//! from multiple `metrics` rows (the Team and IC variants of a bullet
//! both read the same underlying storage table) — UNIQUE on
//! `(metrics_id, metric_catalog_id)` allows that because the pair is
//! distinct per `metrics_id`.
//!
//! ## What this migration does NOT do
//!
//! - Does NOT add a `metric_key` column to `metrics` and does NOT add a
//!   `query_ref` pointer to `metric_catalog`. Both sides keep their
//!   existing payload shape; only the junction is new.
//! - Does NOT remove the `metric_key` string match in admin-CRUD
//!   validation. Catalog rows are still keyed by `metric_key` because
//!   that's what `metric_threshold` references (no FK there per
//!   `m20260522_000002` — separate decision).
//! - Does NOT amend §3.7 (`metric_threshold`'s "no FK" rule). That's
//!   intentional: threshold rows reference `metric_key` by string, which
//!   has independent rationale (audit-survives-deletion-of-parent), and
//!   the junction is orthogonal to that constraint
//!   (see `m20260522_000002_metric_threshold` doc-comment for context).

use sea_orm::{ConnectionTrait, Statement, Value};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Hand-curated mapping: which `metrics` rows back which `metric_catalog`
/// rows, expressed by the ClickHouse storage-table prefix shared by every
/// `metric_key` the query emits. The migration expands each pair into N
/// junction rows by `LIKE '<prefix>.%'` on `metric_catalog.metric_key`.
///
/// Source: every `metrics` seed row was inspected; the primary
/// `FROM insight.<table>` clause names the storage table. Rows whose
/// storage table has no `metric_catalog` coverage today
/// (`exec_summary`, `team_member`, `ic_chart_*`, `ic_drill`, `ic_timeoff`,
/// `crm_*`) are intentionally absent from this list. If a future PR adds
/// catalog rows for any of those tables, append the corresponding
/// `(hex_id, prefix)` pair here and re-run the migration cleanup script.
const QUERY_TO_CATALOG_PREFIX: &[(&str, &str)] = &[
    // Team bullets — read storage tables shared with the IC variants.
    (
        "00000000000000000001000000000003",
        "task_delivery_bullet_rows",
    ),
    (
        "00000000000000000001000000000004",
        "code_quality_bullet_rows",
    ),
    ("00000000000000000001000000000005", "collab_bullet_rows"),
    ("00000000000000000001000000000006", "ai_bullet_rows"),
    // IC KPIs — the 7 catalog rows that map to real `insight.ic_kpis`
    // columns (`tasks_closed`, `bugs_fixed`, `prs_merged`,
    // `pr_cycle_time_h`, `focus_time_pct`, `ai_loc_share_pct`,
    // `ai_sessions`). All other catalog rows still in `ic_kpis.*` shape
    // would come through here too if any were added.
    ("00000000000000000001000000000010", "ic_kpis"),
    // IC bullets — same storage tables as the Team bullets above; both
    // queries link to the same catalog rows.
    (
        "00000000000000000001000000000011",
        "task_delivery_bullet_rows",
    ),
    ("00000000000000000001000000000012", "collab_bullet_rows"),
    ("00000000000000000001000000000013", "ai_bullet_rows"),
    ("00000000000000000001000000000018", "git_bullet_rows"),
];

/// Set-based backfill: one INSERT per mapping entry, expanding the
/// `(metrics_id, table_prefix)` pair into one junction row per matching
/// `metric_catalog` row in a single round-trip.
///
/// `?` params:
///   1. `metrics_hex_id` — 32-char hex; `UNHEX` converts to BINARY(16).
///   2. `table_prefix` — prefix WITHOUT trailing `.`; the `CONCAT(?, '.%')`
///      keeps the dot anchor in the SQL literal so the match can't
///      slip through (defense in depth even though prefixes are
///      compile-time constants today).
///
/// Junction PKs are generated by MariaDB via `UNHEX(REPLACE(UUID(),'-',''))`
/// — `UUIDv1` in BINARY(16) form. Time-ordered for index locality isn't
/// load-bearing on a junction populated once at migration time, so the
/// v1 vs v7 distinction doesn't matter here.
///
/// `INSERT IGNORE` makes re-runs a no-op (the UNIQUE
/// `(metrics_id, metric_catalog_id)` composite catches duplicates).
const INSERT_LINKS_FOR_METRICS_SQL: &str = "\
    INSERT IGNORE INTO metric_query_catalog \
        (id, metrics_id, metric_catalog_id) \
    SELECT UNHEX(REPLACE(UUID(),'-','')), UNHEX(?), c.id \
    FROM metric_catalog c \
    WHERE c.metric_key LIKE CONCAT(?, '.%')";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MetricQueryCatalog::Table)
                    .if_not_exists()
                    // UUIDv7 BINARY(16) — same shape as metric_catalog.id
                    // and metric_threshold.id. Time-ordered for insert
                    // locality on a junction table that grows with every
                    // (metrics × catalog) cross-product.
                    .col(
                        ColumnDef::new(MetricQueryCatalog::Id)
                            .binary_len(16)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MetricQueryCatalog::MetricsId)
                            .binary_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MetricQueryCatalog::MetricCatalogId)
                            .binary_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MetricQueryCatalog::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    // FK → metrics(id). ON DELETE CASCADE: the
                    // junction row is meaningless without its query
                    // row. ON UPDATE NO ACTION (default): UUIDs never
                    // change.
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_metric_query_catalog_metrics")
                            .from(MetricQueryCatalog::Table, MetricQueryCatalog::MetricsId)
                            .to(Metrics::Table, Metrics::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    // FK → metric_catalog(id). Same cascade rationale.
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_metric_query_catalog_metric_catalog")
                            .from(
                                MetricQueryCatalog::Table,
                                MetricQueryCatalog::MetricCatalogId,
                            )
                            .to(MetricCatalog::Table, MetricCatalog::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // UNIQUE composite — a (query, catalog) pair appears at most
        // once. Also serves as the supporting index for `WHERE metrics_id = ?`
        // lookups (leftmost-prefix), which is the dominant access
        // pattern (given a query, list its catalog rows).
        manager
            .create_index(
                Index::create()
                    .name("uq_metric_query_catalog_pair")
                    .table(MetricQueryCatalog::Table)
                    .col(MetricQueryCatalog::MetricsId)
                    .col(MetricQueryCatalog::MetricCatalogId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Reverse-direction lookup index — given a catalog row, list
        // the queries that produce it. Without this index the FK
        // constraint check on `metric_catalog` deletes would full-scan
        // the junction table.
        manager
            .create_index(
                Index::create()
                    .name("idx_metric_query_catalog_catalog_id")
                    .table(MetricQueryCatalog::Table)
                    .col(MetricQueryCatalog::MetricCatalogId)
                    .to_owned(),
            )
            .await?;

        // ── Backfill ───────────────────────────────────────────────
        let conn = manager.get_connection();
        let backend = manager.get_database_backend();

        for (metrics_hex, table_prefix) in QUERY_TO_CATALOG_PREFIX {
            conn.execute(Statement::from_sql_and_values(
                backend,
                INSERT_LINKS_FOR_METRICS_SQL,
                [Value::from(*metrics_hex), Value::from(*table_prefix)],
            ))
            .await?;
        }

        tracing::info!(
            mappings = QUERY_TO_CATALOG_PREFIX.len(),
            "metric_query_catalog backfill applied"
        );

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // we have only forward migrations
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[derive(DeriveIden)]
enum MetricQueryCatalog {
    Table,
    Id,
    MetricsId,
    MetricCatalogId,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Metrics {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum MetricCatalog {
    Table,
    Id,
}
