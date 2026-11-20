use sea_orm::prelude::DateTime;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, IsolationLevel, Statement, TransactionTrait,
    Value,
};
use uuid::Uuid;

const COLUMNS: &str = "person_role_id, insight_tenant_id, person_id, role_id, \
     valid_from, valid_to, author_person_id, reason, created_at";

#[expect(clippy::struct_field_names)] // columns are ids by nature (`*_id`)
#[derive(Debug, Clone)]
pub struct PersonRole {
    pub person_role_id: Uuid,
    pub insight_tenant_id: Uuid,
    pub person_id: Uuid,
    pub role_id: Uuid,
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

fn row_to_person_role(r: &sea_orm::QueryResult) -> anyhow::Result<PersonRole> {
    Ok(PersonRole {
        person_role_id: id(r, "person_role_id")?,
        insight_tenant_id: id(r, "insight_tenant_id")?,
        person_id: id(r, "person_id")?,
        role_id: id(r, "role_id")?,
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
    person_role_id: Uuid,
) -> anyhow::Result<Option<PersonRole>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM person_roles \
         WHERE insight_tenant_id = ? AND person_role_id = ? LIMIT 1"
    );
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        &sql,
        [
            tenant_id.as_bytes().to_vec().into(),
            person_role_id.as_bytes().to_vec().into(),
        ],
    );
    db.query_one_raw(stmt)
        .await?
        .as_ref()
        .map(row_to_person_role)
        .transpose()
}

pub async fn list(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    filter_person: Option<Uuid>,
    filter_role: Option<Uuid>,
    active_only: bool,
    limit: u64,
) -> anyhow::Result<Vec<PersonRole>> {
    let mut sql = format!("SELECT {COLUMNS} FROM person_roles WHERE insight_tenant_id = ?");
    let mut params: Vec<Value> = vec![tenant_id.as_bytes().to_vec().into()];
    if let Some(p) = filter_person {
        sql.push_str(" AND person_id = ?");
        params.push(p.as_bytes().to_vec().into());
    }
    if let Some(r) = filter_role {
        sql.push_str(" AND role_id = ?");
        params.push(r.as_bytes().to_vec().into());
    }
    if active_only {
        sql.push_str(" AND valid_to IS NULL");
    }
    sql.push_str(" ORDER BY created_at DESC, person_role_id DESC LIMIT ?");
    params.push(limit.into());

    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            params,
        ))
        .await?;
    rows.iter().map(row_to_person_role).collect()
}

#[expect(clippy::too_many_arguments)] // mirrors the columns of one assignment row
pub async fn insert(
    db: &DatabaseConnection,
    person_role_id: Uuid,
    tenant_id: Uuid,
    person_id: Uuid,
    role_id: Uuid,
    valid_from: Option<DateTime>,
    author_person_id: Uuid,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    const SQL: &str = r"
        INSERT INTO person_roles
            (person_role_id, insight_tenant_id, person_id, role_id,
             valid_from, valid_to, author_person_id, reason)
        VALUES
            (?, ?, ?, ?, IFNULL(?, UTC_TIMESTAMP(6)), NULL, ?, ?)
    ";
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            person_role_id.as_bytes().to_vec().into(),
            tenant_id.as_bytes().to_vec().into(),
            person_id.as_bytes().to_vec().into(),
            role_id.as_bytes().to_vec().into(),
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
    person_role_id: Uuid,
    reason: Option<&str>,
) -> anyhow::Result<u64> {
    const SQL: &str = r"
        UPDATE person_roles
        SET valid_to = UTC_TIMESTAMP(6),
            reason   = COALESCE(?, reason)
        WHERE insight_tenant_id = ?
          AND person_role_id    = ?
          AND valid_to IS NULL
    ";
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            reason.into(),
            tenant_id.as_bytes().to_vec().into(),
            person_role_id.as_bytes().to_vec().into(),
        ],
    );
    Ok(db.execute_raw(stmt).await?.rows_affected())
}

pub async fn try_soft_delete_protecting_last_admin(
    db: &DatabaseConnection,
    person_role_id: Uuid,
    admin_role_id: Uuid,
    reason: Option<&str>,
) -> anyhow::Result<u64> {
    const LOCK_SQL: &str = r"
        SELECT COUNT(*)
        FROM person_roles
        WHERE insight_tenant_id = (
                SELECT insight_tenant_id FROM person_roles
                WHERE person_role_id = ?
              )
          AND role_id  = ?
          AND valid_to IS NULL
        FOR UPDATE
    ";
    const SQL: &str = r"
        UPDATE person_roles AS target
        JOIN (
            SELECT
                pr.person_role_id,
                pr.role_id,
                (
                    SELECT COUNT(*)
                    FROM person_roles AS adm
                    WHERE adm.insight_tenant_id = pr.insight_tenant_id
                      AND adm.role_id           = ?
                      AND adm.valid_to IS NULL
                ) AS active_admin_cnt
            FROM person_roles AS pr
            WHERE pr.person_role_id = ?
              AND pr.valid_to IS NULL
        ) AS row_with_count
          ON row_with_count.person_role_id = target.person_role_id
        SET target.valid_to = UTC_TIMESTAMP(6),
            target.reason   = COALESCE(?, target.reason)
        WHERE target.valid_to IS NULL
          AND (
              row_with_count.role_id <> ?
              OR row_with_count.active_admin_cnt > 1
          )
    ";
    let admin = admin_role_id.as_bytes().to_vec();
    let pr = person_role_id.as_bytes().to_vec();

    let txn = db
        .begin_with_config(Some(IsolationLevel::RepeatableRead), None)
        .await?;

    txn.query_one_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        LOCK_SQL,
        [pr.clone().into(), admin.clone().into()],
    ))
    .await?;

    let rows = txn
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [admin.clone().into(), pr.into(), reason.into(), admin.into()],
        ))
        .await?
        .rows_affected();

    txn.commit().await?;
    Ok(rows)
}
