use std::collections::{HashMap, HashSet};

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, QueryFilter,
    QueryResult, QuerySelect, Statement,
};
use uuid::Uuid;

use super::entities::persons;
use crate::domain::person_card::{self, CARD_VALUE_TYPES, PersonCard};
use crate::domain::profile::SAFE_PROFILE_ATTRIBUTE_TYPES;
use crate::domain::provenance::UNCONFIRMED_MINT_REASONS;
use crate::domain::resolution::EXCLUDED_PERSON;

#[derive(Debug, thiserror::Error)]
pub enum BatchProfileReadError {
    #[error("batch profile query failed")]
    Database(#[from] sea_orm::DbErr),
    #[error("batch profile row decoding failed: {0}")]
    RowDecode(String),
    #[error("batch profile row contains an invalid person id")]
    InvalidPersonId(#[from] uuid::Error),
}

impl From<sea_orm::TryGetError> for BatchProfileReadError {
    fn from(error: sea_orm::TryGetError) -> Self {
        match error {
            sea_orm::TryGetError::DbErr(error) => Self::Database(error),
            sea_orm::TryGetError::Null(column) => Self::RowDecode(column),
        }
    }
}

pub async fn resolve_person_ids_by_email(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    email: &str,
) -> anyhow::Result<Vec<Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id = ?
              AND value_type = 'email'
        )
        SELECT DISTINCT person_id
        FROM ranked
        WHERE rn = 1
          AND value_id = ?
          AND person_id != ?
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            email.trim().to_owned().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all_raw(stmt).await?;
    person_ids_from_rows(rows)
}

pub async fn resolve_person_id_by_email_any_tenant(
    db: &DatabaseConnection,
    email: &str,
) -> anyhow::Result<Option<Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_type, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn,
                created_at
            FROM persons
            WHERE value_type = 'email'
              AND value_id = ?
        )
        SELECT person_id
        FROM ranked
        WHERE rn = 1
          AND person_id != ?
        ORDER BY created_at DESC, id DESC
        LIMIT 1
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            email.trim().to_owned().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    match db.query_one_raw(stmt).await? {
        Some(row) => {
            let bytes: Vec<u8> = row.try_get("", "person_id")?;
            Ok(Some(Uuid::from_slice(&bytes)?))
        }
        None => Ok(None),
    }
}

pub async fn resolve_person_ids_by_roster_email(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    roster_source_type: &str,
    email: &str,
) -> anyhow::Result<Vec<Uuid>> {
    const SQL: &str = r"
        WITH addressed AS (
            SELECT DISTINCT insight_source_id, person_id
            FROM persons
            WHERE value_type = 'email'
              AND insight_tenant_id = ?
              AND insight_source_type = ?
              AND value_id = ?
        ),
        current_address AS (
            SELECT
                p.person_id,
                p.value_id,
                p.created_at,
                p.id,
                ROW_NUMBER() OVER (
                    PARTITION BY p.insight_source_id, p.person_id
                    ORDER BY p.created_at DESC, p.id DESC
                ) AS rn
            FROM persons p
            JOIN addressed a
              ON a.insight_source_id = p.insight_source_id
             AND a.person_id = p.person_id
            WHERE p.value_type = 'email'
              AND p.insight_tenant_id = ?
              AND p.insight_source_type = ?
        ),
        bindings AS (
            SELECT
                person_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_source_id, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE value_type = 'id'
              AND insight_tenant_id = ?
              AND insight_source_type = ?
        )
        SELECT c.person_id
        FROM current_address c
        WHERE c.rn = 1
          AND c.value_id = ?
          AND c.person_id != ?
          AND EXISTS (
              SELECT 1 FROM bindings b
              WHERE b.rn = 1 AND b.person_id = c.person_id
          )
        GROUP BY c.person_id
        ORDER BY MAX(c.created_at) DESC, MAX(c.id) DESC
    ";

    let tenant_bytes = tenant_id.as_bytes().to_vec();
    let source = roster_source_type.trim().to_owned();
    let address = email.trim().to_owned();
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_bytes.clone().into(),
            source.clone().into(),
            address.clone().into(),
            tenant_bytes.clone().into(),
            source.clone().into(),
            tenant_bytes.into(),
            source.into(),
            address.into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    person_ids_from_rows(db.query_all_raw(stmt).await?)
}

pub async fn resolve_person_id_by_source_any_tenant(
    db: &DatabaseConnection,
    source_type: &str,
    external_id: &str,
) -> anyhow::Result<Option<Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_type, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn,
                created_at
            FROM persons
            WHERE value_type = 'id'
              AND insight_source_type = ?
              AND value_id = ?
        )
        SELECT person_id
        FROM ranked
        WHERE rn = 1
          AND person_id != ?
        ORDER BY created_at DESC, id DESC
        LIMIT 1
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            source_type.to_owned().into(),
            external_id.to_owned().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    match db.query_one_raw(stmt).await? {
        Some(row) => {
            let bytes: Vec<u8> = row.try_get("", "person_id")?;
            Ok(Some(Uuid::from_slice(&bytes)?))
        }
        None => Ok(None),
    }
}

pub async fn resolve_person_ids_by_source_id(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    source_type: &str,
    source_id: Uuid,
    value: &str,
) -> anyhow::Result<Vec<Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id   = ?
              AND insight_source_type = ?
              AND insight_source_id   = ?
              AND value_type          = 'id'
        )
        SELECT DISTINCT person_id
        FROM ranked
        WHERE rn = 1
          AND value_id = ?
          AND person_id != ?
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            source_type.to_owned().into(),
            source_id.as_bytes().to_vec().into(),
            value.to_owned().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all_raw(stmt).await?;
    person_ids_from_rows(rows)
}

pub async fn persons_in_tenant(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> Result<Vec<Uuid>, BatchProfileReadError> {
    if person_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rows = persons::Entity::find()
        .select_only()
        .column(persons::Column::PersonId)
        .filter(persons::Column::InsightTenantId.eq(tenant_id.as_bytes().to_vec()))
        .filter(persons::Column::PersonId.is_in(person_ids.iter().map(|id| id.as_bytes().to_vec())))
        .filter(persons::Column::PersonId.ne(EXCLUDED_PERSON.as_bytes().to_vec()))
        .distinct()
        .into_tuple::<Vec<u8>>()
        .all(db)
        .await?;

    rows.iter().map(|raw| Ok(Uuid::from_slice(raw)?)).collect()
}

pub async fn person_exists(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<bool> {
    if person_id == EXCLUDED_PERSON {
        return Ok(false);
    }

    let found = persons::Entity::find()
        .filter(persons::Column::InsightTenantId.eq(tenant_id.as_bytes().to_vec()))
        .filter(persons::Column::PersonId.eq(person_id.as_bytes().to_vec()))
        .one(db)
        .await?;
    Ok(found.is_some())
}

pub async fn provisional_persons(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> anyhow::Result<HashSet<Uuid>> {
    if person_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let placeholders = vec!["?"; person_ids.len()].join(", ");
    // SAFETY: `<=>`, not `=`: `reason` is nullable, and `=` would make the CASE and the
    // SUM NULL for such a row and drop the person from the answer.
    let mint_reasons = vec!["reason <=> ?"; UNCONFIRMED_MINT_REASONS.len()].join(" OR ");
    let sql = format!(
        "SELECT person_id          FROM persons          WHERE insight_tenant_id = ? AND person_id IN ({placeholders})          GROUP BY person_id          HAVING SUM(CASE WHEN {mint_reasons} THEN 0 ELSE 1 END) = 0"
    );

    let mut params: Vec<sea_orm::Value> =
        Vec::with_capacity(person_ids.len() + 1 + UNCONFIRMED_MINT_REASONS.len());
    params.push(tenant_id.as_bytes().to_vec().into());
    for id in person_ids {
        params.push(id.as_bytes().to_vec().into());
    }
    for reason in UNCONFIRMED_MINT_REASONS {
        params.push(reason.into());
    }

    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            params,
        ))
        .await?;

    let mut provisional = HashSet::with_capacity(rows.len());
    for row in rows {
        let person_id: Vec<u8> = row.try_get("", "person_id")?;
        provisional.insert(Uuid::from_slice(&person_id)?);
    }
    Ok(provisional)
}

pub async fn person_cards(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, PersonCard>> {
    if person_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let id_placeholders = vec!["?"; person_ids.len()].join(", ");
    let type_list = CARD_VALUE_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        r"
        SELECT id, value_type, insight_source_type, insight_source_id,
               insight_tenant_id, value_id, value_full_text, value,
               value_effective, person_id, author_person_id,
               reason, created_at
        FROM (
            SELECT p.*,
                   ROW_NUMBER() OVER (
                       PARTITION BY person_id, insight_source_type, insight_source_id, value_type
                       ORDER BY created_at DESC, id DESC
                   ) AS rn
            FROM persons p
            WHERE insight_tenant_id = ?
              AND person_id IN ({id_placeholders})
              AND value_type IN ({type_list})
        ) current_rows
        WHERE rn = 1
    "
    );

    let mut values: Vec<sea_orm::Value> = vec![tenant_id.as_bytes().to_vec().into()];
    values.extend(person_ids.iter().map(|id| id.as_bytes().to_vec().into()));

    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);
    let rows = persons::Entity::find().from_raw_sql(stmt).all(db).await?;

    Ok(person_card::assemble_cards(rows))
}

pub async fn current_profile_observations(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<persons::Model>>, BatchProfileReadError> {
    if person_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let id_placeholders = vec!["?"; person_ids.len()].join(", ");
    let type_list = SAFE_PROFILE_ATTRIBUTE_TYPES
        .iter()
        .map(|value_type| format!("'{value_type}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        r"
        SELECT id, value_type, insight_source_type, insight_source_id,
               insight_tenant_id, value_id, value_full_text, value,
               value_effective, person_id, author_person_id,
               reason, created_at
        FROM (
            SELECT p.*,
                   ROW_NUMBER() OVER (
                       PARTITION BY person_id, insight_source_type, insight_source_id, value_type
                       ORDER BY created_at DESC, id DESC
                   ) AS rn
            FROM persons p
            WHERE insight_tenant_id = ?
              AND person_id IN ({id_placeholders})
              AND value_type IN ({type_list})
        ) current_rows
        WHERE rn = 1
    "
    );

    let mut values: Vec<sea_orm::Value> = vec![tenant_id.as_bytes().to_vec().into()];
    values.extend(person_ids.iter().map(|id| id.as_bytes().to_vec().into()));

    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);
    let rows = persons::Entity::find().from_raw_sql(stmt).all(db).await?;
    let mut observations = HashMap::new();
    for row in rows {
        observations
            .entry(Uuid::from_slice(&row.person_id)?)
            .or_insert_with(Vec::new)
            .push(row);
    }
    Ok(observations)
}

pub async fn fetch_person_observations(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<Vec<persons::Model>> {
    let rows = persons::Entity::find()
        .filter(persons::Column::InsightTenantId.eq(tenant_id.as_bytes().to_vec()))
        .filter(persons::Column::PersonId.eq(person_id.as_bytes().to_vec()))
        .all(db)
        .await?;
    Ok(rows)
}

pub struct SourceIdRow {
    pub source_type: String,
    pub source_id: Uuid,
    pub value: String,
}

pub async fn current_source_ids_for_person(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<Vec<SourceIdRow>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                insight_source_type,
                insight_source_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id = ?
              AND person_id         = ?
              AND value_type        = 'id'
        )
        SELECT insight_source_type, insight_source_id, value_id AS value
        FROM ranked
        WHERE rn = 1
        ORDER BY insight_source_type, insight_source_id
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            person_id.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all_raw(stmt).await?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let source_id_bytes: Vec<u8> = row.try_get("", "insight_source_id")?;
        let value: Option<String> = row.try_get("", "value")?;
        ids.push(SourceIdRow {
            source_type,
            source_id: Uuid::from_slice(&source_id_bytes)?,
            value: value.unwrap_or_default(),
        });
    }
    Ok(ids)
}

pub struct OrgChartEdge {
    pub source_type: String,
    pub source_id: Uuid,
    pub parent_person_id: Uuid,
}

pub struct OrgChartParentEdge {
    pub child_person_id: Uuid,
    pub source_type: String,
    pub parent_person_id: Uuid,
}

pub async fn current_parents_for_child(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    child_person_id: Uuid,
) -> anyhow::Result<Vec<OrgChartEdge>> {
    const SQL: &str = r"
        SELECT insight_source_type, insight_source_id, parent_person_id
        FROM org_chart
        WHERE insight_tenant_id = ?
          AND child_person_id   = ?
          AND valid_to IS NULL
          AND parent_person_id IS NOT NULL
        ORDER BY insight_source_type, insight_source_id
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            child_person_id.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all_raw(stmt).await?;
    let mut edges = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let source_id: Vec<u8> = row.try_get("", "insight_source_id")?;
        let parent_person_id: Vec<u8> = row.try_get("", "parent_person_id")?;
        edges.push(OrgChartEdge {
            source_type,
            source_id: Uuid::from_slice(&source_id)?,
            parent_person_id: Uuid::from_slice(&parent_person_id)?,
        });
    }
    Ok(edges)
}

pub async fn current_parents_for_children(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    child_person_ids: &[Uuid],
) -> Result<Vec<OrgChartParentEdge>, BatchProfileReadError> {
    if child_person_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = vec!["?"; child_person_ids.len()].join(", ");
    let sql = format!(
        r"
        SELECT child_person_id, insight_source_type, parent_person_id
        FROM org_chart
        WHERE insight_tenant_id = ?
          AND child_person_id IN ({placeholders})
          AND valid_to IS NULL
          AND parent_person_id IS NOT NULL
        ORDER BY child_person_id, insight_source_type, insight_source_id
    "
    );

    let mut values: Vec<sea_orm::Value> = vec![tenant_id.as_bytes().to_vec().into()];
    values.extend(
        child_person_ids
            .iter()
            .map(|person_id| person_id.as_bytes().to_vec().into()),
    );

    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            values,
        ))
        .await?;
    let mut edges = Vec::with_capacity(rows.len());
    for row in rows {
        let child_person_id: Vec<u8> = row.try_get("", "child_person_id")?;
        let source_type: String = row.try_get("", "insight_source_type")?;
        let parent_person_id: Vec<u8> = row.try_get("", "parent_person_id")?;
        edges.push(OrgChartParentEdge {
            child_person_id: Uuid::from_slice(&child_person_id)?,
            source_type,
            parent_person_id: Uuid::from_slice(&parent_person_id)?,
        });
    }
    Ok(edges)
}

pub struct OrgChartChildEdge {
    pub source_type: String,
    pub child_person_id: Uuid,
}

pub async fn current_children_for_parent(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    parent_person_id: Uuid,
) -> anyhow::Result<Vec<OrgChartChildEdge>> {
    const SQL: &str = r"
        SELECT insight_source_type, child_person_id
        FROM org_chart
        WHERE insight_tenant_id  = ?
          AND parent_person_id   = ?
          AND valid_to IS NULL
        ORDER BY insight_source_type, insight_source_id, child_person_id
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            parent_person_id.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all_raw(stmt).await?;
    let mut edges = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let child_person_id: Vec<u8> = row.try_get("", "child_person_id")?;
        edges.push(OrgChartChildEdge {
            source_type,
            child_person_id: Uuid::from_slice(&child_person_id)?,
        });
    }
    Ok(edges)
}

fn person_ids_from_rows(rows: Vec<QueryResult>) -> anyhow::Result<Vec<Uuid>> {
    let mut person_ids = Vec::with_capacity(rows.len());
    for row in rows {
        let bytes: Vec<u8> = row.try_get("", "person_id")?;
        person_ids.push(Uuid::from_slice(&bytes)?);
    }
    Ok(person_ids)
}
