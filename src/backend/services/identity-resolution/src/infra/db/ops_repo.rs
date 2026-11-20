use sea_orm::prelude::DateTime;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use uuid::Uuid;

pub const PERSONS_SEED_OP: &str = "persons-seed";
pub const PERSONS_SYNC_OP: &str = "persons-sync";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl OperationStatus {
    #[must_use]
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    fn from_db(s: &str) -> anyhow::Result<Self> {
        match s {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown operation status '{other}'"),
        }
    }
}

// Field names mirror the DB columns (`operation_id` / `operation_type`).
#[expect(clippy::struct_field_names)]
#[derive(Debug, Clone)]
pub struct Operation {
    pub operation_id: Uuid,
    pub operation_type: String,
    pub status: OperationStatus,
    pub insight_tenant_id: Uuid,
    pub author_person_id: Uuid,
    pub request_json: Option<String>,
    pub summary_json: Option<String>,
    pub error_message: Option<String>,
    pub started_at: DateTime,
    pub completed_at: Option<DateTime>,
}

const COLUMNS: &str = "operation_id, operation_type, status, insight_tenant_id, author_person_id, \
     request_json, summary_json, error_message, started_at, completed_at";

pub async fn enqueue(
    db: &DatabaseConnection,
    operation_id: Uuid,
    operation_type: &str,
    tenant_id: Uuid,
    author_person_id: Uuid,
    request_json: Option<&str>,
) -> anyhow::Result<()> {
    const SQL: &str = r"
        INSERT INTO operations
            (operation_id, operation_type, status,
             insight_tenant_id, author_person_id, request_json)
        VALUES (?, ?, 'queued', ?, ?, ?)
    ";
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            operation_id.as_bytes().to_vec().into(),
            operation_type.into(),
            tenant_id.as_bytes().to_vec().into(),
            author_person_id.as_bytes().to_vec().into(),
            request_json.into(),
        ],
    ))
    .await?;
    Ok(())
}

pub async fn try_start(db: &DatabaseConnection, operation_id: Uuid) -> anyhow::Result<bool> {
    const SQL: &str =
        "UPDATE operations SET status = 'running' WHERE operation_id = ? AND status = 'queued'";
    let res = db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [operation_id.as_bytes().to_vec().into()],
        ))
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn complete(
    db: &DatabaseConnection,
    operation_id: Uuid,
    summary_json: &str,
) -> anyhow::Result<()> {
    const SQL: &str = r"
        UPDATE operations
        SET status = 'completed', summary_json = ?, completed_at = UTC_TIMESTAMP(6)
        WHERE operation_id = ?
    ";
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [summary_json.into(), operation_id.as_bytes().to_vec().into()],
    ))
    .await?;
    Ok(())
}

pub async fn fail(
    db: &DatabaseConnection,
    operation_id: Uuid,
    error_message: &str,
) -> anyhow::Result<()> {
    const SQL: &str = r"
        UPDATE operations
        SET status = 'failed', error_message = ?, completed_at = UTC_TIMESTAMP(6)
        WHERE operation_id = ?
    ";
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            error_message.into(),
            operation_id.as_bytes().to_vec().into(),
        ],
    ))
    .await?;
    Ok(())
}

pub async fn get_by_id(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    operation_id: Uuid,
) -> anyhow::Result<Option<Operation>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM operations WHERE insight_tenant_id = ? AND operation_id = ? LIMIT 1"
    );
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            [
                tenant_id.as_bytes().to_vec().into(),
                operation_id.as_bytes().to_vec().into(),
            ],
        ))
        .await?;
    row.map(|r| row_to_operation(&r)).transpose()
}

pub async fn list(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    operation_type: Option<&str>,
    status: Option<OperationStatus>,
    limit: u64,
) -> anyhow::Result<Vec<Operation>> {
    let mut sql = format!("SELECT {COLUMNS} FROM operations WHERE insight_tenant_id = ?");
    let mut params: Vec<sea_orm::Value> = vec![tenant_id.as_bytes().to_vec().into()];
    if let Some(t) = operation_type {
        sql.push_str(" AND operation_type = ?");
        params.push(t.into());
    }
    if let Some(s) = status {
        sql.push_str(" AND status = ?");
        params.push(s.as_db().into());
    }
    sql.push_str(" ORDER BY started_at DESC, operation_id DESC LIMIT ?");
    params.push(limit.into());

    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            params,
        ))
        .await?;
    rows.iter().map(row_to_operation).collect()
}

pub async fn corrections_for_account(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    operation_type: &str,
    source_type: &str,
    source_id: Uuid,
    account_id: &str,
    limit: u64,
) -> anyhow::Result<Vec<Operation>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM operations          WHERE insight_tenant_id = ?            AND operation_type = ?            AND JSON_CONTAINS(                 request_json,                  JSON_OBJECT('source', ?, 'source_id', ?, 'account_id', ?),                  '$.accounts')          ORDER BY started_at DESC, operation_id DESC LIMIT ?"
    );

    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            [
                tenant_id.as_bytes().to_vec().into(),
                operation_type.into(),
                source_type.into(),
                source_id.to_string().into(),
                account_id.into(),
                limit.into(),
            ],
        ))
        .await?;
    rows.iter().map(row_to_operation).collect()
}

pub async fn sweep_zombies(db: &DatabaseConnection, older_than: DateTime) -> anyhow::Result<u64> {
    const SQL: &str = r"
        UPDATE operations
        SET status        = 'failed',
            error_message = 'aborted by pod restart',
            completed_at  = UTC_TIMESTAMP(6)
        WHERE status IN ('queued', 'running')
          AND started_at < ?
    ";
    let res = db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [older_than.into()],
        ))
        .await?;
    Ok(res.rows_affected())
}

fn row_to_operation(r: &sea_orm::QueryResult) -> anyhow::Result<Operation> {
    let operation_id: Vec<u8> = r.try_get("", "operation_id")?;
    let tenant: Vec<u8> = r.try_get("", "insight_tenant_id")?;
    let author: Vec<u8> = r.try_get("", "author_person_id")?;
    let status: String = r.try_get("", "status")?;
    Ok(Operation {
        operation_id: Uuid::from_slice(&operation_id)?,
        operation_type: r.try_get("", "operation_type")?,
        status: OperationStatus::from_db(&status)?,
        insight_tenant_id: Uuid::from_slice(&tenant)?,
        author_person_id: Uuid::from_slice(&author)?,
        request_json: r.try_get("", "request_json")?,
        summary_json: r.try_get("", "summary_json")?,
        error_message: r.try_get("", "error_message")?,
        started_at: r.try_get("", "started_at")?,
        completed_at: r.try_get("", "completed_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_status_db_round_trip() -> anyhow::Result<()> {
        for s in [
            OperationStatus::Queued,
            OperationStatus::Running,
            OperationStatus::Completed,
            OperationStatus::Failed,
        ] {
            assert_eq!(OperationStatus::from_db(s.as_db())?, s);
        }
        assert!(OperationStatus::from_db("bogus").is_err());
        Ok(())
    }
}
