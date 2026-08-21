//! Operator-correction write store (MariaDB).
//!
//! Corrections append binding observations to `persons`. Nothing here updates
//! or deletes a journal row, and nothing here rebuilds the derived caches:
//! those stay the persons-seed's to own, on its own schedule, the way the
//! ClickHouse mirror already works. The journal is the source of truth and
//! every read path — the correction verbs, the review queue, the history —
//! reads it directly, so a correction is visible the moment it commits.

use std::collections::{HashMap, HashSet};

use sea_orm::{
    AccessMode, ConnectionTrait, DatabaseConnection, DbBackend, IsolationLevel, Statement,
    TransactionTrait, Value,
};
use uuid::Uuid;

use crate::domain::provenance::Provenance;
use crate::domain::resolution::{BINDING_VALUE_TYPE, BindingRow};
use crate::domain::seed::{KnownBinding, SourceAccountKey};

/// Ceiling for a request handler's whole-tenant read, which holds every row it
/// gets in memory. Paired with the evidence fold's own cap: the review can only
/// look up accounts the fold reported, so bindings past that many are unusable
/// anyway. Reaching it truncates what the operator sees and is reported, not
/// passed off as a complete answer.
pub const MAX_TENANT_BINDINGS: u64 = 200_000;

/// How much of the tenant a whole-tenant read may return.
#[derive(Debug, Clone, Copy)]
pub enum Ceiling {
    /// Stop after this many and say so. For a reader that can honestly show a
    /// prefix — a console page, a rate with a "partial" flag beside it.
    Bounded(u64),
    /// Every binding, whatever it costs. For a batch whose decisions are wrong
    /// without them: one missed binding and it mints a second person for an
    /// account that already has one.
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

/// How many accounts one statement may name.
///
/// INVARIANT: keep this small. A statement's cost grows faster than the number
/// of accounts it names, so raising it to save round trips trades a cheap cost
/// for an expensive one.
pub(crate) const LOOKUP_CHUNK: usize = 100;

/// Which accounts a binding read covers — the one difference between the two
/// readers below, kept a value so a caller cannot hand the query a fragment.
#[derive(Debug, Clone, Copy)]
enum Scope {
    /// These many accounts, named as a bound tuple list.
    Named(usize),
    /// Every account of the tenant, up to a bound [`Ceiling`].
    WholeTenant,
}

/// The latest `value_type='id'` observation per account, with its author so the
/// caller can tell an operator decision from an automatic one.
fn current_bindings_sql(scope: Scope) -> String {
    // Ordering only where the ceiling can cut: a bounded read has to return the
    // same prefix twice, or the queue's counts would move with no data change.
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

/// Current binding of each requested account. Accounts never observed are simply
/// absent.
///
/// For a handful of accounts — a verb's targets, a page of the listing. A caller
/// after most of the tenant reads it whole instead
/// ([`current_bindings_in_tenant`]), because a statement's cost grows faster
/// than the number of accounts it names.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
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

/// Every account's current binding in the tenant, and whether the ceiling cut
/// the read.
#[derive(Debug, Default)]
pub struct BindingSnapshot {
    pub by_account: HashMap<SourceAccountKey, KnownBinding>,
    /// [`MAX_TENANT_BINDINGS`] was reached: these are a prefix of the tenant's
    /// bindings, so anything derived from them describes a prefix too.
    pub truncated: bool,
}

/// Current binding of every account in the tenant — for the callers that ask
/// about all of them: the review surface, and the persons-seed.
///
/// INVARIANT: this answers exactly what naming every account would, plus
/// accounts nobody asked about. The filter it drops sits on the ranking's own
/// partition key, so removing it cannot change which observation wins for an
/// account — only which accounts appear. A caller that must not see beyond its
/// own keys looks them up rather than iterating the map.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
pub async fn current_bindings_in_tenant(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    ceiling: Ceiling,
) -> anyhow::Result<BindingSnapshot> {
    // Ask for one row past the ceiling: its presence is what says another row
    // exists. Counting the rows against the ceiling instead calls a read that
    // ended exactly on it truncated, and a caller that refuses to answer on
    // truncation would then refuse a complete answer.
    let probe = ceiling.rows().saturating_add(1);
    let mut rows = db
        .query_all(Statement::from_sql_and_values(
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
/// Append one binding ONLY IF the account has none yet, in a single statement.
///
/// The login bootstrap cannot check-then-write: between the two an operator's
/// exclusion or the seed's own link can land, and because the binding in force
/// is the LATEST row, an automation row written after it would silently
/// override a human's decision. Making "nobody has decided this account" part
/// of the same statement as the insert removes the window — the condition is
/// evaluated against the same snapshot that writes.
///
/// "Nobody has decided it" is scoped exactly as the login lookup scopes it —
/// by (`source_type`, `value_id`), across every tenant and connector instance.
/// Narrowing the guard to the instance the evidence names would leave the
/// decisions it is meant to protect invisible: an exclusion recorded before a
/// connector was re-registered lives under the OLD instance id, while the
/// lookup that answers "who is in force" ignores the instance entirely — so a
/// narrow guard would write, and the fresh row would win.
///
/// The inner derived table is not decoration: MariaDB refuses a bare subquery
/// on the INSERT's own target, and materialising it is what makes the
/// self-reference legal.
///
/// Returns whether the row was written. `false` means somebody else had
/// already decided the account, and the caller must read what they decided
/// rather than assume its own row is in force.
///
/// # Errors
///
/// Returns an error if the statement fails.
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

    // The isolation level is stated, not inherited: the guard's protection rests
    // on the self-read seeing the same snapshot the insert writes into, and
    // under READ COMMITTED that read does not lock, so an operator decision can
    // slip between them. MariaDB defaults to REPEATABLE READ, which is what
    // makes `INSERT ... SELECT` take a locking read — a deployment that changed
    // the default must not silently weaken this.
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

    let result = match txn.execute(statement).await {
        Ok(result) => result,
        // A lock conflict here means another login is writing THIS account
        // right now. "We did not write" is the truthful answer, and the
        // caller's read-back then reports whatever the winner decided —
        // whereas surfacing it would put a 500 on the login screen for a
        // condition the design already handles.
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

/// Whether MariaDB refused the statement because another transaction held the
/// rows: deadlock (1213) or lock-wait timeout (1205).
///
/// Classified from the message for the same reason `is_missing_relation` does
/// it for ClickHouse — the driver surfaces the server's code in the text and
/// exposes no typed variant for either condition.
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

#[cfg(test)]
mod tests {
    use sea_orm::{DbErr, RuntimeErr};

    use super::*;

    /// A lock conflict is not a fault to surface: it means another login is
    /// writing this very account, and the caller's read-back is what resolves
    /// it. Misclassifying either code puts a 500 on the login screen instead.
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
