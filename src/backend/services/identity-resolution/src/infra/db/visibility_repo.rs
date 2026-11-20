// `viewer_person_id` / `viewed_person_id` are the domain's own field names.
#![expect(clippy::similar_names)]

use sea_orm::prelude::DateTime;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, Value};
use uuid::Uuid;

const COLUMNS: &str = "visibility_id, insight_tenant_id, viewer_person_id, viewed_person_id, \
     valid_from, valid_to, author_person_id, reason, created_at";

#[expect(clippy::struct_field_names)] // columns are ids by nature (`*_id`)
#[derive(Debug, Clone)]
pub struct Visibility {
    pub visibility_id: Uuid,
    pub insight_tenant_id: Uuid,
    pub viewer_person_id: Uuid,
    pub viewed_person_id: Option<Uuid>,
    pub valid_from: DateTime,
    pub valid_to: Option<DateTime>,
    pub author_person_id: Uuid,
    pub reason: Option<String>,
    pub created_at: DateTime,
}

fn id(r: &sea_orm::QueryResult, col: &str) -> anyhow::Result<Uuid> {
    let bytes: Vec<u8> = r.try_get("", col)?;
    Ok(Uuid::from_slice(&bytes)?)
}

fn opt_id(r: &sea_orm::QueryResult, col: &str) -> anyhow::Result<Option<Uuid>> {
    let bytes: Option<Vec<u8>> = r.try_get("", col)?;
    bytes
        .map(|b| Uuid::from_slice(&b))
        .transpose()
        .map_err(Into::into)
}

fn row_to_visibility(r: &sea_orm::QueryResult) -> anyhow::Result<Visibility> {
    Ok(Visibility {
        visibility_id: id(r, "visibility_id")?,
        insight_tenant_id: id(r, "insight_tenant_id")?,
        viewer_person_id: id(r, "viewer_person_id")?,
        viewed_person_id: opt_id(r, "viewed_person_id")?,
        valid_from: r.try_get("", "valid_from")?,
        valid_to: r.try_get("", "valid_to")?,
        author_person_id: id(r, "author_person_id")?,
        reason: r.try_get("", "reason")?,
        created_at: r.try_get("", "created_at")?,
    })
}

pub async fn get_by_id(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    visibility_id: Uuid,
) -> anyhow::Result<Option<Visibility>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM visibility \
         WHERE insight_tenant_id = ? AND visibility_id = ? LIMIT 1"
    );
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        &sql,
        [
            tenant_id.as_bytes().to_vec().into(),
            visibility_id.as_bytes().to_vec().into(),
        ],
    );
    db.query_one_raw(stmt)
        .await?
        .as_ref()
        .map(row_to_visibility)
        .transpose()
}

pub async fn list(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    filter_viewer: Option<Uuid>,
    filter_viewed: Option<Uuid>,
    active_only: bool,
    limit: u64,
) -> anyhow::Result<Vec<Visibility>> {
    let mut sql = format!("SELECT {COLUMNS} FROM visibility WHERE insight_tenant_id = ?");
    let mut params: Vec<Value> = vec![tenant_id.as_bytes().to_vec().into()];
    if let Some(v) = filter_viewer {
        sql.push_str(" AND viewer_person_id = ?");
        params.push(v.as_bytes().to_vec().into());
    }
    if let Some(v) = filter_viewed {
        sql.push_str(" AND viewed_person_id = ?");
        params.push(v.as_bytes().to_vec().into());
    }
    if active_only {
        sql.push_str(" AND valid_to IS NULL");
    }
    sql.push_str(" ORDER BY created_at DESC, visibility_id DESC LIMIT ?");
    params.push(limit.into());

    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            params,
        ))
        .await?;
    rows.iter().map(row_to_visibility).collect()
}

#[expect(clippy::too_many_arguments)] // mirrors the columns of one grant row
pub async fn insert(
    db: &DatabaseConnection,
    visibility_id: Uuid,
    tenant_id: Uuid,
    viewer_person_id: Uuid,
    viewed_person_id: Option<Uuid>,
    valid_from: Option<DateTime>,
    author_person_id: Uuid,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    const SQL: &str = r"
        INSERT INTO visibility
            (visibility_id, insight_tenant_id, viewer_person_id, viewed_person_id,
             valid_from, valid_to, author_person_id, reason)
        VALUES
            (?, ?, ?, ?, IFNULL(?, UTC_TIMESTAMP(6)), NULL, ?, ?)
    ";
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            visibility_id.as_bytes().to_vec().into(),
            tenant_id.as_bytes().to_vec().into(),
            viewer_person_id.as_bytes().to_vec().into(),
            viewed_person_id.map(|u| u.as_bytes().to_vec()).into(),
            valid_from.into(),
            author_person_id.as_bytes().to_vec().into(),
            reason.into(),
        ],
    );
    db.execute_raw(stmt).await?;
    Ok(())
}

pub async fn soft_delete(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    visibility_id: Uuid,
    reason: Option<&str>,
) -> anyhow::Result<u64> {
    const SQL: &str = r"
        UPDATE visibility
        SET valid_to = UTC_TIMESTAMP(6),
            reason   = COALESCE(?, reason)
        WHERE insight_tenant_id = ?
          AND visibility_id     = ?
          AND valid_to IS NULL
    ";
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            reason.into(),
            tenant_id.as_bytes().to_vec().into(),
            visibility_id.as_bytes().to_vec().into(),
        ],
    );
    Ok(db.execute_raw(stmt).await?.rows_affected())
}
