//! Persons-seed write store (MariaDB).
//!
//! Two halves, ported from the .NET `IPersonsSeedStore` / `SqlPersonsSeed`:
//!   * resolver-feeding reads — current `account → person` bindings and the
//!     latest `email → person` map (fed to [`crate::domain::seed`]);
//!   * the transactional `apply` — `INSERT IGNORE` the resolved observations
//!     into `persons`, then rebuild the tenant's `account_person_map` (SCD2).
//!
//! The transactional `apply` also rebuilds `org_chart`. All SQL is verbatim
//! from the .NET service for parity. These queries use `LEAD()`/`ROW_NUMBER()`
//! window functions (SCD2 `valid_from`/`valid_to`) and `INSERT … SELECT` cache
//! rebuilds — constructs `toolkit-db` cannot express, hence the raw-SQL /
//! self-managed pool (see `infra::db` module docs + constructorfabric/gears-rust#4239).

#![allow(dead_code)]

use std::collections::HashMap;

use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait, Value};
use uuid::Uuid;

use crate::domain::seed::{SeedObservationRow, SourceAccountKey, normalize_email};
use crate::domain::seed_service::{ApplyCounts, SeedStore};

/// MariaDB-backed [`SeedStore`] — wraps a connection so the persons-seed service
/// can be driven against the real DB (or a fake in tests).
pub struct MariaDbSeedStore<'a> {
    db: &'a DatabaseConnection,
}

impl<'a> MariaDbSeedStore<'a> {
    #[must_use]
    pub fn new(db: &'a DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SeedStore for MariaDbSeedStore<'_> {
    async fn known_account_bindings(
        &self,
        tenant_id: Uuid,
    ) -> anyhow::Result<HashMap<SourceAccountKey, Uuid>> {
        known_account_bindings(self.db, tenant_id).await
    }

    async fn latest_email_to_person(
        &self,
        tenant_id: Uuid,
    ) -> anyhow::Result<HashMap<String, Uuid>> {
        latest_email_to_person(self.db, tenant_id).await
    }

    async fn apply(
        &self,
        tenant_id: Uuid,
        author_person_id: Uuid,
        rows: &[SeedObservationRow],
    ) -> anyhow::Result<ApplyCounts> {
        apply(self.db, tenant_id, author_person_id, rows).await
    }
}

/// Current `source_account_id → person_id` bindings for the tenant — the latest
/// `value_type='id'` observation per account. Feeds the known-account branch of
/// the resolver. Ported from `SqlPersonsSeed.KnownAccountBindings`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
pub async fn known_account_bindings(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> anyhow::Result<HashMap<SourceAccountKey, Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                insight_source_type,
                insight_source_id,
                value_id AS source_account_id,
                person_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE value_type = 'id'
              AND value_id IS NOT NULL
              AND insight_tenant_id = ?
        )
        SELECT insight_source_type, insight_source_id, source_account_id, person_id
        FROM ranked
        WHERE rn = 1
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [tenant_id.as_bytes().to_vec().into()],
    );

    let rows = db.query_all(stmt).await?;
    let mut map = HashMap::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let source_id: Vec<u8> = row.try_get("", "insight_source_id")?;
        let account_id: String = row.try_get("", "source_account_id")?;
        let person_id: Vec<u8> = row.try_get("", "person_id")?;
        map.insert(
            SourceAccountKey {
                source_type,
                source_id: Uuid::from_slice(&source_id)?,
                account_id,
            },
            Uuid::from_slice(&person_id)?,
        );
    }
    Ok(map)
}

/// Current `email → person_id` map for the tenant — the latest
/// `value_type='email'` observation per email. Keys are normalized via
/// [`normalize_email`] (lowercase only, no trim — ADR-0011, .NET parity) so the
/// resolver's lookups match. Ported from `SqlPersonsSeed.LatestEmailToPerson`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn latest_email_to_person(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> anyhow::Result<HashMap<String, Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                value_id AS email,
                person_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE value_type = 'email'
              AND value_id IS NOT NULL
              AND value_id != ''
              AND insight_tenant_id = ?
        )
        SELECT email, person_id
        FROM ranked
        WHERE rn = 1
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [tenant_id.as_bytes().to_vec().into()],
    );

    let rows = db.query_all(stmt).await?;
    let mut map = HashMap::with_capacity(rows.len());
    for row in rows {
        let email: String = row.try_get("", "email")?;
        let person_id: Vec<u8> = row.try_get("", "person_id")?;
        map.insert(normalize_email(&email), Uuid::from_slice(&person_id)?);
    }
    Ok(map)
}

/// Apply a seed's resolved observations: `INSERT IGNORE` each into `persons`,
/// then rebuild the tenant's `account_person_map` and `org_chart` — all in one
/// transaction, so the log and the derived caches are never left
/// cross-inconsistent. `author_person_id` stamps the computed `org_chart`
/// no-parent rows (the seed operation's author). Returns the number of
/// observation rows actually inserted (duplicates are ignored).
///
/// # Errors
///
/// Returns an error if any statement fails; the transaction is rolled back.
// The length is dominated by the verbatim org_chart CTE string constant, not
// control flow — keeping the SQL inline (co-located, greppable) over hoisting it.
#[allow(clippy::too_many_lines)]
pub async fn apply(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    author_person_id: Uuid,
    rows: &[SeedObservationRow],
) -> anyhow::Result<ApplyCounts> {
    // Idempotent insert — uq_person_observation dedups a re-emitted identical
    // observation; INSERT IGNORE swallows the duplicate-key error. Batched
    // (multi-row VALUES) so N observations cost ~N/INSERT_CHUNK round-trips
    // instead of N — 25k+ single-row inserts over a remote pool take minutes;
    // batches take seconds. Same semantics as per-row INSERT IGNORE.
    const INSERT_PREFIX: &str = "INSERT IGNORE INTO persons \
        (value_type, insight_source_type, insight_source_id, insight_tenant_id, \
         value_id, value_full_text, value, person_id, author_person_id, reason, \
         created_at) VALUES ";
    const ROW_TUPLE: &str = "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
    const INSERT_CHUNK: usize = 500; // 500 rows × 11 cols = 5500 binds (< 65535)
    const DELETE_APM: &str = "DELETE FROM account_person_map WHERE insight_tenant_id = ?";
    const INSERT_APM: &str = r"
        INSERT INTO account_person_map
            (insight_tenant_id, insight_source_type, insight_source_id, source_account_id,
             person_id, author_person_id, reason, valid_from, valid_to)
        SELECT
            insight_tenant_id,
            insight_source_type,
            insight_source_id,
            value_id AS source_account_id,
            person_id,
            author_person_id,
            reason,
            created_at AS valid_from,
            LEAD(created_at) OVER (
                PARTITION BY insight_tenant_id, insight_source_type,
                             insight_source_id, value_id
                ORDER BY created_at
            ) AS valid_to
        FROM persons
        WHERE value_type = 'id'
          AND value_id IS NOT NULL
          AND insight_tenant_id = ?
    ";
    const DELETE_ORG_CHART: &str = "DELETE FROM org_chart WHERE insight_tenant_id = ?";
    // Ported verbatim from Sql.PersonsSeed.cs::InsertOrgChartForTenant. The `?`
    // markers bind, in order: `insight_tenant_id` SIX times (state_log,
    // default_active, pe_periods, email_to_person, existing_edges,
    // source_member_latest_active), then `author_person_id` once (the Path-B
    // no-parent rows). Keep this order in lock-step with the params vec below.
    const INSERT_ORG_CHART: &str = r"
        INSERT INTO org_chart
            (insight_tenant_id, insight_source_type, insight_source_id,
             child_person_id, parent_person_id,
             author_person_id, reason, valid_from, valid_to)
        WITH
        state_log AS (
            SELECT
                insight_tenant_id, insight_source_type, insight_source_id, person_id,
                created_at, id,
                CASE
                    WHEN value_full_text IN ('Inactive', 'Terminated', 'inactive', 'terminated')
                        THEN 0 ELSE 1
                END AS is_active,
                LAG(CASE
                    WHEN value_full_text IN ('Inactive', 'Terminated', 'inactive', 'terminated')
                        THEN 0 ELSE 1
                END) OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, person_id
                    ORDER BY created_at, id
                ) AS prev_is_active
            FROM persons
            WHERE value_type = 'status'
              AND value_full_text IS NOT NULL
              AND insight_tenant_id = ?
        ),
        state_transitions AS (
            SELECT
                insight_tenant_id, insight_source_type, insight_source_id, person_id,
                created_at, id, is_active,
                LEAD(created_at) OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, person_id
                    ORDER BY created_at, id
                ) AS next_transition_at
            FROM state_log
            WHERE prev_is_active IS NULL OR prev_is_active <> is_active
        ),
        active_intervals AS (
            SELECT
                insight_tenant_id, insight_source_type, insight_source_id, person_id,
                created_at         AS interval_start,
                next_transition_at AS interval_end
            FROM state_transitions
            WHERE is_active = 1
        ),
        default_active AS (
            SELECT DISTINCT
                pe.insight_tenant_id, pe.insight_source_type, pe.insight_source_id, pe.person_id,
                CAST('1970-01-01 00:00:00.000000' AS DATETIME(6)) AS interval_start,
                CAST(NULL AS DATETIME(6)) AS interval_end
            FROM persons pe
            WHERE pe.value_type = 'parent_email'
              AND pe.value_id IS NOT NULL
              AND pe.insight_tenant_id = ?
              AND NOT EXISTS (
                  SELECT 1 FROM persons s
                  WHERE s.insight_tenant_id   = pe.insight_tenant_id
                    AND s.insight_source_type = pe.insight_source_type
                    AND s.insight_source_id   = pe.insight_source_id
                    AND s.person_id           = pe.person_id
                    AND s.value_type          = 'status'
              )
        ),
        all_active AS (
            SELECT * FROM active_intervals
            UNION ALL
            SELECT * FROM default_active
        ),
        pe_periods AS (
            SELECT
                pe.insight_tenant_id, pe.insight_source_type, pe.insight_source_id,
                pe.person_id AS child_person_id,
                pe.value_id AS parent_email,
                pe.author_person_id, pe.reason,
                pe.created_at AS pe_from,
                LEAD(pe.created_at) OVER (
                    PARTITION BY pe.insight_tenant_id, pe.insight_source_type,
                                 pe.insight_source_id, pe.person_id
                    ORDER BY pe.created_at, pe.id
                ) AS pe_to
            FROM persons pe
            WHERE pe.value_type = 'parent_email'
              AND pe.value_id IS NOT NULL
              AND pe.insight_tenant_id = ?
        ),
        email_to_person AS (
            SELECT
                p.insight_tenant_id, p.value_id, p.person_id,
                ROW_NUMBER() OVER (
                    PARTITION BY p.insight_tenant_id, p.value_id
                    ORDER BY p.created_at DESC, p.id DESC
                ) AS rn
            FROM persons p
            WHERE p.value_type = 'email'
              AND p.value_id IS NOT NULL
              AND p.insight_tenant_id = ?
        ),
        existing_edges AS (
            SELECT
                insight_tenant_id, insight_source_type, insight_source_id,
                person_id                                       AS child_person_id,
                UNHEX(REPLACE(value_id, '-', ''))               AS parent_person_id,
                author_person_id, reason,
                created_at                                      AS valid_from,
                LEAD(created_at) OVER (
                    PARTITION BY insight_tenant_id, insight_source_type,
                                 insight_source_id, person_id
                    ORDER BY created_at
                )                                               AS valid_to
            FROM persons
            WHERE value_type = 'parent_person_id'
              AND value_id IS NOT NULL
              AND insight_tenant_id = ?
              AND value_id REGEXP '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$'
              AND HEX(person_id) <> REPLACE(value_id, '-', '')

            UNION ALL

            SELECT
                pe.insight_tenant_id, pe.insight_source_type, pe.insight_source_id,
                pe.child_person_id,
                parent.person_id                                AS parent_person_id,
                pe.author_person_id, pe.reason,
                GREATEST(pe.pe_from, ai.interval_start)         AS valid_from,
                CASE
                    WHEN pe.pe_to IS NULL AND ai.interval_end IS NULL THEN NULL
                    WHEN pe.pe_to        IS NULL                      THEN ai.interval_end
                    WHEN ai.interval_end IS NULL                      THEN pe.pe_to
                    ELSE LEAST(pe.pe_to, ai.interval_end)
                END                                             AS valid_to
            FROM pe_periods pe
            INNER JOIN email_to_person parent
                ON parent.insight_tenant_id = pe.insight_tenant_id
               AND parent.value_id          = pe.parent_email
               AND parent.rn                = 1
            INNER JOIN all_active ai
                ON ai.insight_tenant_id   = pe.insight_tenant_id
               AND ai.insight_source_type = pe.insight_source_type
               AND ai.insight_source_id   = pe.insight_source_id
               AND ai.person_id           = pe.child_person_id
               AND ai.interval_start < COALESCE(pe.pe_to, '9999-12-31 23:59:59.999999')
               AND COALESCE(ai.interval_end, '9999-12-31 23:59:59.999999') > pe.pe_from
            WHERE parent.person_id <> pe.child_person_id
              AND NOT EXISTS (
                  SELECT 1 FROM persons ppi
                  WHERE ppi.insight_tenant_id   = pe.insight_tenant_id
                    AND ppi.person_id           = pe.child_person_id
                    AND ppi.insight_source_type = pe.insight_source_type
                    AND ppi.insight_source_id   = pe.insight_source_id
                    AND ppi.value_type          = 'parent_person_id'
                    AND ppi.value_id IS NOT NULL
              )
        ),
        source_member_latest_active AS (
            SELECT
                m.insight_tenant_id, m.insight_source_type, m.insight_source_id, m.person_id,
                m.first_obs,
                latest.interval_end
            FROM (
                SELECT
                    insight_tenant_id, insight_source_type, insight_source_id, person_id,
                    MIN(created_at) AS first_obs,
                    MAX(CASE WHEN value_type = 'status' THEN 1 ELSE 0 END) AS has_status
                FROM persons
                WHERE insight_tenant_id = ?
                GROUP BY insight_tenant_id, insight_source_type, insight_source_id, person_id
            ) m
            LEFT JOIN (
                SELECT
                    insight_tenant_id, insight_source_type, insight_source_id, person_id,
                    interval_end,
                    ROW_NUMBER() OVER (
                        PARTITION BY insight_tenant_id, insight_source_type,
                                     insight_source_id, person_id
                        ORDER BY interval_start DESC,
                                 COALESCE(interval_end, '9999-12-31 23:59:59.999999') DESC
                    ) AS rn
                FROM active_intervals
            ) latest
                ON latest.insight_tenant_id   = m.insight_tenant_id
               AND latest.insight_source_type = m.insight_source_type
               AND latest.insight_source_id   = m.insight_source_id
               AND latest.person_id           = m.person_id
               AND latest.rn                  = 1
            WHERE m.has_status = 0
               OR latest.person_id IS NOT NULL
        )

        SELECT * FROM existing_edges

        UNION ALL

        SELECT
            sm.insight_tenant_id, sm.insight_source_type, sm.insight_source_id,
            sm.person_id                                    AS child_person_id,
            CAST(NULL AS BINARY(16))                        AS parent_person_id,
            ?                                               AS author_person_id,
            ''                                              AS reason,
            sm.first_obs                                    AS valid_from,
            sm.interval_end                                 AS valid_to
        FROM source_member_latest_active sm
        WHERE NOT EXISTS (
              SELECT 1 FROM existing_edges e
              WHERE e.insight_tenant_id   = sm.insight_tenant_id
                AND e.insight_source_type = sm.insight_source_type
                AND e.insight_source_id   = sm.insight_source_id
                AND e.child_person_id     = sm.person_id
          )
    ";

    let tenant_bytes = tenant_id.as_bytes().to_vec();
    let author_bytes = author_person_id.as_bytes().to_vec();
    let txn = db.begin().await?;

    let mut inserted = 0u64;
    for chunk in rows.chunks(INSERT_CHUNK) {
        let values = vec![ROW_TUPLE; chunk.len()].join(", ");
        let sql = format!("{INSERT_PREFIX}{values}");
        let mut params: Vec<Value> = Vec::with_capacity(chunk.len() * 11);
        for r in chunk {
            params.push(r.value_type.clone().into());
            params.push(r.source_type.clone().into());
            params.push(r.source_id.as_bytes().to_vec().into());
            params.push(tenant_bytes.clone().into());
            params.push(r.value_id.clone().into());
            params.push(r.value_full_text.clone().into());
            params.push(r.value.clone().into());
            params.push(r.person_id.as_bytes().to_vec().into());
            params.push(r.author_person_id.as_bytes().to_vec().into());
            params.push(r.reason.clone().into());
            params.push(r.created_at.into());
        }
        let res = txn
            .execute(Statement::from_sql_and_values(
                DbBackend::MySql,
                &sql,
                params,
            ))
            .await?;
        inserted += res.rows_affected();
    }
    tracing::info!(inserted, "persons-seed apply: observations inserted");

    // Rebuild account_person_map for the tenant (delete + reinsert).
    txn.execute(Statement::from_sql_and_values(
        DbBackend::MySql,
        DELETE_APM,
        [tenant_bytes.clone().into()],
    ))
    .await?;
    txn.execute(Statement::from_sql_and_values(
        DbBackend::MySql,
        INSERT_APM,
        [tenant_bytes.clone().into()],
    ))
    .await?;
    tracing::info!("persons-seed apply: account_person_map rebuilt");

    // Rebuild org_chart for the tenant. The CTE binds the tenant six times, then
    // the author once — same order as INSERT_ORG_CHART's `?` markers.
    txn.execute(Statement::from_sql_and_values(
        DbBackend::MySql,
        DELETE_ORG_CHART,
        [tenant_bytes.clone().into()],
    ))
    .await?;
    let org_chart = txn
        .execute(Statement::from_sql_and_values(
            DbBackend::MySql,
            INSERT_ORG_CHART,
            [
                tenant_bytes.clone().into(),
                tenant_bytes.clone().into(),
                tenant_bytes.clone().into(),
                tenant_bytes.clone().into(),
                tenant_bytes.clone().into(),
                tenant_bytes.into(),
                author_bytes.into(),
            ],
        ))
        .await?;
    let org_chart_rows_rebuilt = org_chart.rows_affected();
    tracing::info!(
        org_chart_rows_rebuilt,
        "persons-seed apply: org_chart rebuilt"
    );

    txn.commit().await?;
    Ok(ApplyCounts {
        observations_inserted: inserted,
        org_chart_rows_rebuilt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db;

    /// Integration test against a live MariaDB — reads only (no writes). Set
    /// `IDENTITY_TEST_DB_URL` + `IDENTITY_TEST_TENANT_ID` and a port-forward to
    /// run; skips cleanly otherwise so CI stays green.
    #[tokio::test]
    async fn read_maps_against_dev_db() -> anyhow::Result<()> {
        let (Ok(url), Ok(tenant_raw)) = (
            std::env::var("IDENTITY_TEST_DB_URL"),
            std::env::var("IDENTITY_TEST_TENANT_ID"),
        ) else {
            eprintln!("skip: set IDENTITY_TEST_DB_URL + IDENTITY_TEST_TENANT_ID to run");
            return Ok(());
        };
        let tenant = Uuid::parse_str(tenant_raw.trim())?;
        let conn = db::connect(&url).await?;

        let known = known_account_bindings(&conn, tenant).await?;
        let emails = latest_email_to_person(&conn, tenant).await?;
        // A seeded dev tenant has bindings and emails; assert the reads work and
        // the maps are non-trivial without pinning to specific data.
        assert!(!known.is_empty(), "dev tenant should have account bindings");
        assert!(
            !emails.is_empty(),
            "dev tenant should have email→person rows"
        );
        Ok(())
    }
}
