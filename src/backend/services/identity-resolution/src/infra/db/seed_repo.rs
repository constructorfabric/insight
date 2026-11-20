use std::collections::HashMap;

use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait, Value,
};
use uuid::Uuid;

use crate::domain::people::PersonChange;
use crate::domain::seed::{KnownBinding, SeedObservationRow, SourceAccountKey, normalize_email};
use crate::domain::seed_service::{ApplyCounts, SeedStore};
use crate::infra::db::{people_repo, resolution_repo};
use crate::infra::metrics::{self, DbQuery};

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
    ) -> anyhow::Result<HashMap<SourceAccountKey, KnownBinding>> {
        let started = std::time::Instant::now();
        let result = known_account_bindings(self.db, tenant_id).await;
        metrics::record_db_query(DbQuery::KnownAccountBindings, started.elapsed());
        result
    }

    async fn latest_email_to_person(
        &self,
        tenant_id: Uuid,
    ) -> anyhow::Result<HashMap<String, Uuid>> {
        let started = std::time::Instant::now();
        let result = latest_email_to_person(self.db, tenant_id).await;
        metrics::record_db_query(DbQuery::LatestEmailToPerson, started.elapsed());
        result
    }

    async fn apply(
        &self,
        tenant_id: Uuid,
        author_person_id: Uuid,
        rows: &[SeedObservationRow],
        people: &[PersonChange],
        retained_people: Option<&std::collections::HashSet<Uuid>>,
    ) -> anyhow::Result<ApplyCounts> {
        let started = std::time::Instant::now();
        let result = apply(
            self.db,
            tenant_id,
            author_person_id,
            rows,
            people,
            retained_people,
        )
        .await;
        metrics::record_db_query(DbQuery::Apply, started.elapsed());
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantPresence {
    pub has_own: bool,
    pub has_other: bool,
}

pub async fn distinct_tenants(db: &DatabaseConnection, cap: u64) -> anyhow::Result<Vec<Uuid>> {
    const SQL: &str = "SELECT DISTINCT insight_tenant_id FROM persons LIMIT ?";
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [cap.into()],
        ))
        .await?;
    rows.iter()
        .map(|r| {
            let id: Vec<u8> = r.try_get("", "insight_tenant_id")?;
            Ok(Uuid::from_slice(&id)?)
        })
        .collect()
}

pub async fn tenant_presence(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> anyhow::Result<TenantPresence> {
    const SQL: &str = r"
        SELECT
            EXISTS(SELECT 1 FROM persons WHERE insight_tenant_id = ?)  AS has_own,
            EXISTS(SELECT 1 FROM persons WHERE insight_tenant_id <> ?) AS has_other
    ";
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [
                tenant_id.as_bytes().to_vec().into(),
                tenant_id.as_bytes().to_vec().into(),
            ],
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("tenant_presence query returned no row"))?;
    Ok(TenantPresence {
        has_own: row.try_get::<i64>("", "has_own")? != 0,
        has_other: row.try_get::<i64>("", "has_other")? != 0,
    })
}

pub async fn known_account_bindings(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> anyhow::Result<HashMap<SourceAccountKey, KnownBinding>> {
    Ok(resolution_repo::current_bindings_in_tenant(
        db,
        tenant_id,
        resolution_repo::Ceiling::Unbounded,
    )
    .await?
    .by_account)
}

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

    let rows = db.query_all_raw(stmt).await?;
    let mut map = HashMap::with_capacity(rows.len());
    for row in rows {
        let email: String = row.try_get("", "email")?;
        let person_id: Vec<u8> = row.try_get("", "person_id")?;
        map.insert(normalize_email(&email), Uuid::from_slice(&person_id)?);
    }
    Ok(map)
}

// Length is the verbatim org_chart CTE string constant, not control flow.
#[expect(clippy::too_many_lines)]
async fn rebuild_org_chart(
    txn: &DatabaseTransaction,
    tenant_id: Uuid,
    author_person_id: Uuid,
) -> anyhow::Result<u64> {
    const DELETE_ORG_CHART: &str = "DELETE FROM org_chart WHERE insight_tenant_id = ?";
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

    txn.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        DELETE_ORG_CHART,
        [tenant_bytes.clone().into()],
    ))
    .await?;
    let org_chart = txn
        .execute_raw(Statement::from_sql_and_values(
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

    Ok(org_chart_rows_rebuilt)
}

pub async fn apply(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    author_person_id: Uuid,
    rows: &[SeedObservationRow],
    people: &[PersonChange],
    retained_people: Option<&std::collections::HashSet<Uuid>>,
) -> anyhow::Result<ApplyCounts> {
    const INSERT_PREFIX: &str = "INSERT IGNORE INTO persons \
        (value_type, insight_source_type, insight_source_id, insight_tenant_id, \
         value_id, value_full_text, value, person_id, author_person_id, reason, \
         created_at) VALUES ";
    const ROW_TUPLE: &str = "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
    const INSERT_CHUNK: usize = 500; // 500 rows × 11 cols = 5500 binds (< 65535)
    let tenant_bytes = tenant_id.as_bytes().to_vec();
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
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                &sql,
                params,
            ))
            .await?;
        inserted += res.rows_affected();
    }
    tracing::info!(inserted, "persons-seed apply: observations inserted");

    let people_counts = people_repo::reconcile(&txn, tenant_id, people, retained_people).await?;
    let org_chart_rows_rebuilt = rebuild_org_chart(&txn, tenant_id, author_person_id).await?;

    txn.commit().await?;
    Ok(ApplyCounts {
        observations_inserted: inserted,
        org_chart_rows_rebuilt,
        people_opened: people_counts.opened,
        people_closed: people_counts.closed,
        people_unchanged: people_counts.unchanged,
    })
}
