//! Persons-seed write store (MariaDB).
//!
//! Two halves, ported from the .NET `IPersonsSeedStore` / `SqlPersonsSeed`:
//!   * resolver-feeding reads — current `account → person` bindings and the
//!     latest `email → person` map (fed to [`crate::domain::seed`]);
//!   * the transactional `apply` — `INSERT IGNORE` the resolved observations
//!     into `persons`, then rebuild the tenant's `account_person_map` (SCD2).
//!
//! `org_chart` rebuild + the `ClickHouse` `identity_inputs` reader land in later
//! slices. All SQL is verbatim from the .NET service for parity.

#![allow(dead_code)]

use std::collections::HashMap;

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait, Value};
use uuid::Uuid;

use crate::domain::seed::{SeedObservationRow, SourceAccountKey, normalize_email};

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
/// `value_type='email'` observation per email. Keys are normalized (trim +
/// lowercase, ADR-0011) so the resolver's lookups match. Ported from
/// `SqlPersonsSeed.LatestEmailToPerson`.
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
/// then rebuild the tenant's `account_person_map` — all in one transaction, so
/// the log and the derived cache are never left cross-inconsistent. Returns the
/// number of observation rows actually inserted (duplicates are ignored).
///
/// # Errors
///
/// Returns an error if any statement fails; the transaction is rolled back.
pub async fn apply(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    rows: &[SeedObservationRow],
) -> anyhow::Result<u64> {
    // Idempotent insert — uq_person_observation dedups a re-emitted identical
    // observation; INSERT IGNORE swallows the duplicate-key error.
    const INSERT_OBSERVATION: &str = r"
        INSERT IGNORE INTO persons
            (value_type, insight_source_type, insight_source_id, insight_tenant_id,
             value_id, value_full_text, value,
             person_id, author_person_id, reason, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ";
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

    let tenant_bytes = tenant_id.as_bytes().to_vec();
    let txn = db.begin().await?;

    let mut inserted = 0u64;
    for r in rows {
        let params: Vec<Value> = vec![
            r.value_type.clone().into(),
            r.source_type.clone().into(),
            r.source_id.as_bytes().to_vec().into(),
            tenant_bytes.clone().into(),
            r.value_id.clone().into(),
            r.value_full_text.clone().into(),
            r.value.clone().into(),
            r.person_id.as_bytes().to_vec().into(),
            r.author_person_id.as_bytes().to_vec().into(),
            r.reason.clone().into(),
            r.created_at.into(),
        ];
        let res = txn
            .execute(Statement::from_sql_and_values(
                DbBackend::MySql,
                INSERT_OBSERVATION,
                params,
            ))
            .await?;
        inserted += res.rows_affected();
    }

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
        [tenant_bytes.into()],
    ))
    .await?;

    txn.commit().await?;
    Ok(inserted)
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
