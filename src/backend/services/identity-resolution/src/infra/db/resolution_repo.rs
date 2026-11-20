use std::collections::{HashMap, HashSet};

use sea_orm::{
    AccessMode, ConnectionTrait, DatabaseConnection, DbBackend, IsolationLevel, Statement,
    TransactionTrait, Value,
};
use uuid::Uuid;

use crate::domain::provenance::Provenance;
use crate::domain::resolution::{BINDING_VALUE_TYPE, BindingRow};
use crate::domain::seed::{KnownBinding, SourceAccountKey};

pub const MAX_TENANT_BINDINGS: u64 = 200_000;

#[derive(Debug, Clone, Copy)]
pub enum Ceiling {
    Bounded(u64),
    Unbounded,
}

impl Ceiling {
    const fn rows(self) -> u64 {
        match self {
            Self::Bounded(rows) => rows,
            Self::Unbounded => u64::MAX,
        }
    }
}

pub(crate) const LOOKUP_CHUNK: usize = 100;

#[derive(Debug, Clone, Copy)]
enum Scope {
    Named(usize),
    WholeTenant,
}

fn current_bindings_sql(scope: Scope) -> String {
    let (accounts, page) = match scope {
        Scope::Named(count) => (
            format!(
                "\n              AND (insight_source_type, insight_source_id, value_id) IN ({})",
                vec!["(?, ?, ?)"; count].join(", ")
            ),
            String::new(),
        ),
        Scope::WholeTenant => (
            String::new(),
            "\n        ORDER BY insight_source_type, insight_source_id, source_account_id\
             \n        LIMIT ?"
                .to_owned(),
        ),
    };

    format!(
        r"
        WITH ranked AS (
            SELECT
                insight_source_type,
                insight_source_id,
                value_id AS source_account_id,
                person_id,
                author_person_id,
                reason,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE value_type = 'id'
              AND value_id IS NOT NULL
              AND insight_tenant_id = ?{accounts}
        )
        SELECT insight_source_type, insight_source_id, source_account_id, person_id,
               author_person_id, reason
        FROM ranked
        WHERE rn = 1{page}
    "
    )
}

pub async fn current_bindings(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    accounts: &[SourceAccountKey],
) -> anyhow::Result<HashMap<SourceAccountKey, KnownBinding>> {
    let mut map = HashMap::with_capacity(accounts.len());
    for chunk in accounts.chunks(LOOKUP_CHUNK) {
        let sql = current_bindings_sql(Scope::Named(chunk.len()));

        let mut params: Vec<Value> = Vec::with_capacity(chunk.len() * 3 + 1);
        params.push(tenant_id.as_bytes().to_vec().into());
        for account in chunk {
            params.push(account.source_type.clone().into());
            params.push(account.source_id.as_bytes().to_vec().into());
            params.push(account.account_id.clone().into());
        }

        let rows = db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                &sql,
                params,
            ))
            .await?;

        collect_bindings(rows, &mut map)?;
    }
    Ok(map)
}

#[derive(Debug, Default)]
pub struct BindingSnapshot {
    pub by_account: HashMap<SourceAccountKey, KnownBinding>,
    pub truncated: bool,
}

pub async fn current_bindings_in_tenant(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    ceiling: Ceiling,
) -> anyhow::Result<BindingSnapshot> {
    let probe = ceiling.rows().saturating_add(1);
    let mut rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            current_bindings_sql(Scope::WholeTenant),
            [tenant_id.as_bytes().to_vec().into(), probe.into()],
        ))
        .await?;

    let truncated = rows.len() as u64 >= probe;
    if truncated {
        rows.truncate(usize::try_from(ceiling.rows()).unwrap_or(usize::MAX));
        tracing::warn!(
            cap = ceiling.rows(),
            "tenant bindings: read cap reached; anything derived from these \
             describes only this many accounts, not the whole tenant"
        );
    }

    let mut by_account = HashMap::with_capacity(rows.len());
    collect_bindings(rows, &mut by_account)?;
    Ok(BindingSnapshot {
        by_account,
        truncated,
    })
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
        let reason: Option<String> = row.try_get("", "reason")?;
        map.insert(
            SourceAccountKey {
                source_type,
                source_id: Uuid::from_slice(&source_id)?,
                account_id,
            },
            KnownBinding {
                person_id: Uuid::from_slice(&person_id)?,
                author_person_id: Uuid::from_slice(&author_person_id)?,
                provenance: Provenance::of_reason(reason.as_deref()),
            },
        );
    }
    Ok(())
}

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
        .query_all_raw(Statement::from_sql_and_values(
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

pub async fn person_exists(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<bool> {
    const SQL: &str =
        "SELECT 1 AS hit FROM persons WHERE insight_tenant_id = ? AND person_id = ? LIMIT 1";

    let row = db
        .query_one_raw(Statement::from_sql_and_values(
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

/// MariaDB refuses a bare subquery on the INSERT's own target; the inner derived
/// table is what makes the self-reference legal.
pub async fn append_binding_if_unbound(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    row: &BindingRow,
) -> anyhow::Result<bool> {
    const SQL: &str = r"
        INSERT INTO persons
            (value_type, insight_source_type, insight_source_id, insight_tenant_id,
             value_id, value_full_text, value, person_id, author_person_id, reason,
             created_at)
        SELECT * FROM (
            SELECT 'id' AS c1, ? AS c2, ? AS c3, ? AS c4, ? AS c5,
                   NULL AS c6, NULL AS c7, ? AS c8, ? AS c9, ? AS c10, ? AS c11
        ) AS incoming
        WHERE NOT EXISTS (
            SELECT 1 FROM (
                SELECT 1 FROM persons
                WHERE value_type = 'id'
                  AND insight_source_type = ?
                  AND value_id = ?
                LIMIT 1
            ) AS decided
        )
    ";

    // InnoDB's `INSERT ... SELECT` takes a locking read under REPEATABLE READ but not
    // under READ COMMITTED, where an operator decision could slip between the guard's
    // self-read and the insert, so the level is stated rather than inherited.
    let txn = db
        .begin_with_config(
            Some(IsolationLevel::RepeatableRead),
            Some(AccessMode::ReadWrite),
        )
        .await?;

    let statement = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            row.account.source_type.clone().into(),
            row.account.source_id.as_bytes().to_vec().into(),
            tenant_id.as_bytes().to_vec().into(),
            row.account.account_id.clone().into(),
            row.person_id.as_bytes().to_vec().into(),
            row.author_person_id.as_bytes().to_vec().into(),
            row.reason.clone().into(),
            row.created_at.into(),
            row.account.source_type.clone().into(),
            row.account.account_id.clone().into(),
        ],
    );

    let result = match txn.execute_raw(statement).await {
        Ok(result) => result,
        Err(e) if is_lock_conflict(&e) => {
            tracing::info!(
                source_type = %row.account.source_type,
                account_id = %row.account.account_id,
                "login bootstrap: another writer holds this account; deferring to it"
            );
            let _ = txn.rollback().await;
            return Ok(false);
        }
        Err(e) => return Err(e.into()),
    };

    txn.commit().await?;

    Ok(result.rows_affected() > 0)
}

fn is_lock_conflict(error: &sea_orm::DbErr) -> bool {
    let message = error.to_string();
    message.contains("1213")
        || message.contains("1205")
        || message.contains("Deadlock found")
        || message.contains("Lock wait timeout exceeded")
}

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
            .execute_raw(Statement::from_sql_and_values(
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

#[derive(Debug, Clone)]
pub struct BindingHistoryRow {
    pub person_id: Uuid,
    pub author_person_id: Uuid,
    pub reason: Option<String>,
    pub created_at: sea_orm::prelude::DateTime,
}

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
        .query_all_raw(Statement::from_sql_and_values(
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
    const PRESENCE_CHUNK: usize = 200;

    let mut found: HashSet<(String, Uuid, String, Uuid, Uuid, sea_orm::prelude::DateTime)> =
        HashSet::new();

    for chunk in rows.chunks(PRESENCE_CHUNK) {
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
            .query_all_raw(Statement::from_sql_and_values(
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

#[cfg(test)]
mod tests {
    use sea_orm::{DbErr, RuntimeErr};

    use super::*;

    #[test]
    fn a_lock_conflict_is_told_apart_from_a_real_failure() {
        for (case, message, expected) in [
            (
                "deadlock, by code",
                "Execution Error: error returned from database: 1213 (40001): \
                 Deadlock found when trying to get lock; try restarting transaction",
                true,
            ),
            (
                "lock-wait timeout, by code",
                "Execution Error: error returned from database: 1205 (HY000): \
                 Lock wait timeout exceeded; try restarting transaction",
                true,
            ),
            (
                "deadlock, wording only",
                "Deadlock found when trying to get lock",
                true,
            ),
            (
                "lock-wait timeout, wording only",
                "Lock wait timeout exceeded",
                true,
            ),
            (
                "a duplicate key is NOT a lock conflict",
                "error returned from database: 1062 (23000): Duplicate entry for key 'PRIMARY'",
                false,
            ),
            (
                "a syntax error is NOT a lock conflict",
                "error returned from database: 1064 (42000): You have an error in your SQL syntax",
                false,
            ),
            (
                "a dead connection is NOT a lock conflict",
                "Connection Error: closed",
                false,
            ),
        ] {
            let error = DbErr::Exec(RuntimeErr::Internal(message.to_owned()));
            assert_eq!(is_lock_conflict(&error), expected, "misclassified: {case}");
        }
    }
}
