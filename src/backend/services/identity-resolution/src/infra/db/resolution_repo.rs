//! Operator-correction write store (MariaDB).
//!
//! Corrections append binding observations to `persons`. Nothing here updates
//! or deletes a journal row, and nothing here rebuilds the derived caches:
//! those stay the persons-seed's to own, on its own schedule, the way the
//! ClickHouse mirror already works. The journal is the source of truth and
//! every read path — the correction verbs, the review queue, the history —
//! reads it directly, so a correction is visible the moment it commits.

use std::collections::{HashMap, HashSet};

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait, Value};
use uuid::Uuid;

use crate::domain::resolution::{BINDING_VALUE_TYPE, BindingRow};
use crate::domain::seed::{KnownBinding, SourceAccountKey};

/// Current binding of each requested account — the latest `value_type='id'`
/// observation, with its author so the caller can tell an operator decision
/// from an automatic one. Accounts never observed are simply absent.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
pub async fn current_bindings(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    accounts: &[SourceAccountKey],
) -> anyhow::Result<HashMap<SourceAccountKey, KnownBinding>> {
    const SQL_PREFIX: &str = r"
        WITH ranked AS (
            SELECT
                insight_source_type,
                insight_source_id,
                value_id AS source_account_id,
                person_id,
                author_person_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE value_type = 'id'
              AND value_id IS NOT NULL
              AND insight_tenant_id = ?
              AND (insight_source_type, insight_source_id, value_id) IN (";
    const SQL_SUFFIX: &str = r")
        )
        SELECT insight_source_type, insight_source_id, source_account_id, person_id, author_person_id
        FROM ranked
        WHERE rn = 1
    ";

    // The review surface asks about every observed account, so the list is as
    // long as the tenant is wide: chunk it rather than build one statement
    // whose placeholder count grows without bound.
    const LOOKUP_CHUNK: usize = 500;

    let mut map = HashMap::with_capacity(accounts.len());
    for chunk in accounts.chunks(LOOKUP_CHUNK) {
        let tuples = vec!["(?, ?, ?)"; chunk.len()].join(", ");
        let sql = format!("{SQL_PREFIX}{tuples}{SQL_SUFFIX}");

        let mut params: Vec<Value> = Vec::with_capacity(chunk.len() * 3 + 1);
        params.push(tenant_id.as_bytes().to_vec().into());
        for account in chunk {
            params.push(account.source_type.clone().into());
            params.push(account.source_id.as_bytes().to_vec().into());
            params.push(account.account_id.clone().into());
        }

        let rows = db
            .query_all(Statement::from_sql_and_values(
                DbBackend::MySql,
                &sql,
                params,
            ))
            .await?;

        collect_bindings(rows, &mut map)?;
    }
    Ok(map)
}

fn collect_bindings(
    rows: Vec<sea_orm::QueryResult>,
    map: &mut HashMap<SourceAccountKey, KnownBinding>,
) -> anyhow::Result<()> {
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let source_id: Vec<u8> = row.try_get("", "insight_source_id")?;
        let account_id: String = row.try_get("", "source_account_id")?;
        let person_id: Vec<u8> = row.try_get("", "person_id")?;
        let author_person_id: Vec<u8> = row.try_get("", "author_person_id")?;
        map.insert(
            SourceAccountKey {
                source_type,
                source_id: Uuid::from_slice(&source_id)?,
                account_id,
            },
            KnownBinding {
                person_id: Uuid::from_slice(&person_id)?,
                author_person_id: Uuid::from_slice(&author_person_id)?,
            },
        );
    }
    Ok(())
}

/// Accounts currently bound to a person (latest binding wins). The merge verb
/// reads this to know what to rebind.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
pub async fn accounts_of_person(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<Vec<SourceAccountKey>> {
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
        SELECT insight_source_type, insight_source_id, source_account_id
        FROM ranked
        WHERE rn = 1 AND person_id = ?
    ";

    let rows = db
        .query_all(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [
                tenant_id.as_bytes().to_vec().into(),
                person_id.as_bytes().to_vec().into(),
            ],
        ))
        .await?;

    let mut accounts = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let source_id: Vec<u8> = row.try_get("", "insight_source_id")?;
        let account_id: String = row.try_get("", "source_account_id")?;
        accounts.push(SourceAccountKey {
            source_type,
            source_id: Uuid::from_slice(&source_id)?,
            account_id,
        });
    }
    Ok(accounts)
}

/// Whether the tenant's journal knows this person at all — a correction may not
/// invent a target out of thin air.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn person_exists(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<bool> {
    const SQL: &str =
        "SELECT 1 AS hit FROM persons WHERE insight_tenant_id = ? AND person_id = ? LIMIT 1";

    let row = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [
                tenant_id.as_bytes().to_vec().into(),
                person_id.as_bytes().to_vec().into(),
            ],
        ))
        .await?;
    Ok(row.is_some())
}

/// Append binding observations in one transaction. Returns the number of rows
/// actually appended (a re-emitted identical observation is ignored by the
/// natural key).
///
/// The tenant's derived caches are deliberately NOT rebuilt here. The rebuild
/// is whole-tenant — it deletes and re-derives `account_person_map` and
/// `org_chart` from the entire journal — and it grows far worse than linearly,
/// so putting it in a request path makes one operator decision cost minutes on
/// a large tenant. It also buys nothing: a correction appends only
/// `value_type='id'` rows, while `org_chart` is derived from the parent, status
/// and e-mail observations a correction never touches. The caches stay the
/// seed's to refresh on its own schedule, like the ClickHouse mirror.
///
/// # Errors
///
/// Returns an error if any statement fails; the transaction is rolled back.
pub async fn append_bindings(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    rows: &[BindingRow],
) -> anyhow::Result<u64> {
    const INSERT_PREFIX: &str = "INSERT IGNORE INTO persons \
        (value_type, insight_source_type, insight_source_id, insight_tenant_id, \
         value_id, value_full_text, value, person_id, author_person_id, reason, \
         created_at) VALUES ";
    const ROW_TUPLE: &str = "(?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?)";
    const INSERT_CHUNK: usize = 500;

    let txn = db.begin().await?;

    let mut appended = 0u64;
    for chunk in rows.chunks(INSERT_CHUNK) {
        let values = vec![ROW_TUPLE; chunk.len()].join(", ");
        let sql = format!("{INSERT_PREFIX}{values}");

        let mut params: Vec<Value> = Vec::with_capacity(chunk.len() * 9);
        for row in chunk {
            params.push(BINDING_VALUE_TYPE.into());
            params.push(row.account.source_type.clone().into());
            params.push(row.account.source_id.as_bytes().to_vec().into());
            params.push(tenant_id.as_bytes().to_vec().into());
            params.push(row.account.account_id.clone().into());
            params.push(row.person_id.as_bytes().to_vec().into());
            params.push(row.author_person_id.as_bytes().to_vec().into());
            params.push(row.reason.clone().into());
            params.push(row.created_at.into());
        }

        let res = txn
            .execute(Statement::from_sql_and_values(
                DbBackend::MySql,
                &sql,
                params,
            ))
            .await?;
        appended += res.rows_affected();
    }

    txn.commit().await?;

    tracing::info!(appended, "identity correction: bindings appended");
    Ok(appended)
}

/// One appended observation in an account's history — what the explain surface
/// shows: who bound it where, when, and why.
#[derive(Debug, Clone)]
pub struct BindingHistoryRow {
    pub person_id: Uuid,
    pub author_person_id: Uuid,
    pub reason: Option<String>,
    pub created_at: sea_orm::prelude::DateTime,
}

/// Every binding observation ever recorded for one account, newest first.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
pub async fn binding_history(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    account: &SourceAccountKey,
) -> anyhow::Result<Vec<BindingHistoryRow>> {
    const SQL: &str = r"
        SELECT person_id, author_person_id, reason, created_at
        FROM persons
        WHERE value_type = 'id'
          AND insight_tenant_id = ?
          AND insight_source_type = ?
          AND insight_source_id = ?
          AND value_id = ?
        ORDER BY created_at DESC, id DESC
    ";

    let rows = db
        .query_all(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [
                tenant_id.as_bytes().to_vec().into(),
                account.source_type.clone().into(),
                account.source_id.as_bytes().to_vec().into(),
                account.account_id.clone().into(),
            ],
        ))
        .await?;

    let mut history = Vec::with_capacity(rows.len());
    for row in rows {
        let person_id: Vec<u8> = row.try_get("", "person_id")?;
        let author_person_id: Vec<u8> = row.try_get("", "author_person_id")?;
        history.push(BindingHistoryRow {
            person_id: Uuid::from_slice(&person_id)?,
            author_person_id: Uuid::from_slice(&author_person_id)?,
            reason: row.try_get("", "reason")?,
            created_at: row.try_get("", "created_at")?,
        });
    }
    Ok(history)
}

/// Which of these exact observations the journal holds.
///
/// Identity is the whole natural key including the author and the instant, not
/// just "the account points at this person": a confirmation appends an
/// operator-authored row over an automatic binding to the SAME person, so
/// asking about the person alone cannot tell a landed row from a refused one.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn present_rows(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    rows: &[BindingRow],
) -> anyhow::Result<Vec<bool>> {
    const SQL_PREFIX: &str = "SELECT insight_source_type, insight_source_id, value_id, \
         person_id, author_person_id, created_at \
         FROM persons \
         WHERE value_type = 'id' AND insight_tenant_id = ? \
           AND (insight_source_type, insight_source_id, value_id, person_id, \
                author_person_id, created_at) IN (";
    const ROW_TUPLE: &str = "(?, ?, ?, ?, ?, ?)";
    const LOOKUP_CHUNK: usize = 200;

    let mut found: HashSet<(String, Uuid, String, Uuid, Uuid, sea_orm::prelude::DateTime)> =
        HashSet::new();

    for chunk in rows.chunks(LOOKUP_CHUNK) {
        let tuples = vec![ROW_TUPLE; chunk.len()].join(", ");
        let sql = format!("{SQL_PREFIX}{tuples})");

        let mut params: Vec<Value> = Vec::with_capacity(chunk.len() * 6 + 1);
        params.push(tenant_id.as_bytes().to_vec().into());
        for row in chunk {
            params.push(row.account.source_type.clone().into());
            params.push(row.account.source_id.as_bytes().to_vec().into());
            params.push(row.account.account_id.clone().into());
            params.push(row.person_id.as_bytes().to_vec().into());
            params.push(row.author_person_id.as_bytes().to_vec().into());
            params.push(row.created_at.into());
        }

        let hits = db
            .query_all(Statement::from_sql_and_values(
                DbBackend::MySql,
                &sql,
                params,
            ))
            .await?;

        for hit in hits {
            let source_type: String = hit.try_get("", "insight_source_type")?;
            let source_id: Vec<u8> = hit.try_get("", "insight_source_id")?;
            let account_id: String = hit.try_get("", "value_id")?;
            let person_id: Vec<u8> = hit.try_get("", "person_id")?;
            let author_person_id: Vec<u8> = hit.try_get("", "author_person_id")?;
            found.insert((
                source_type,
                Uuid::from_slice(&source_id)?,
                account_id,
                Uuid::from_slice(&person_id)?,
                Uuid::from_slice(&author_person_id)?,
                hit.try_get("", "created_at")?,
            ));
        }
    }

    Ok(rows
        .iter()
        .map(|row| {
            found.contains(&(
                row.account.source_type.clone(),
                row.account.source_id,
                row.account.account_id.clone(),
                row.person_id,
                row.author_person_id,
                row.created_at,
            ))
        })
        .collect())
}
