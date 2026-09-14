use std::collections::{HashMap, HashSet};

use chrono::{NaiveDateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, QueryResult, Statement};
use uuid::Uuid;

use crate::domain::people::{PersonChange, PersonProjection};
use crate::domain::seed::SourceAccountKey;

#[derive(Debug, thiserror::Error)]
pub(crate) enum PeopleRepoError {
    #[error("people repository query failed")]
    Database(#[from] sea_orm::DbErr),
    #[error("people repository row decoding failed: {0}")]
    RowDecode(String),
    #[error("people repository row contains an invalid person id")]
    InvalidPersonId(#[from] uuid::Error),
    #[error("people repository attributes are invalid")]
    InvalidAttributes(#[from] serde_json::Error),
}

impl From<sea_orm::TryGetError> for PeopleRepoError {
    fn from(error: sea_orm::TryGetError) -> Self {
        match error {
            sea_orm::TryGetError::DbErr(error) => Self::Database(error),
            sea_orm::TryGetError::Null(column) => Self::RowDecode(column),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentPerson {
    id: u64,
    projection: PersonProjection,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileCounts {
    pub opened: u64,
    pub closed: u64,
    pub unchanged: u64,
}

pub async fn reconcile(
    txn: &DatabaseTransaction,
    tenant_id: Uuid,
    changes: &[PersonChange],
    retained_people: Option<&HashSet<Uuid>>,
) -> Result<ReconcileCounts, PeopleRepoError> {
    let current = current_people(txn, tenant_id).await?;
    let previously_closed = previously_closed_people(txn, tenant_id).await?;
    let now = Utc::now().naive_utc();
    let mut counts = ReconcileCounts::default();

    for change in changes {
        match change {
            PersonChange::Upsert(projected) => {
                let Some(existing) = current.get(&projected.person_id) else {
                    let valid_from = opening_time(
                        previously_closed.contains(&projected.person_id),
                        projected.valid_from,
                        now,
                    );
                    insert(txn, tenant_id, projected, valid_from).await?;
                    counts.opened += 1;
                    continue;
                };
                if same_state(&existing.projection, projected) {
                    counts.unchanged += 1;
                    continue;
                }

                let valid_from =
                    transition_time(existing.projection.valid_from, projected.valid_from, now);
                close(txn, existing.id, valid_from).await?;
                insert(txn, tenant_id, projected, valid_from).await?;
                counts.closed += 1;
                counts.opened += 1;
            }
            PersonChange::Close {
                person_id,
                valid_to,
            } => {
                let Some(existing) = current.get(person_id) else {
                    counts.unchanged += 1;
                    continue;
                };
                let valid_to = transition_time(existing.projection.valid_from, *valid_to, now);
                close(txn, existing.id, valid_to).await?;
                counts.closed += 1;
            }
        }
    }

    if let Some(retained_people) = retained_people {
        for existing in unretained_current(&current, changes, retained_people) {
            close(txn, existing.id, now).await?;
            counts.closed += 1;
        }
    }

    Ok(counts)
}

fn unretained_current<'a>(
    current: &'a HashMap<Uuid, CurrentPerson>,
    changes: &[PersonChange],
    retained_people: &HashSet<Uuid>,
) -> Vec<&'a CurrentPerson> {
    let mentioned = changes
        .iter()
        .map(|change| match change {
            PersonChange::Upsert(projected) => projected.person_id,
            PersonChange::Close { person_id, .. } => *person_id,
        })
        .collect::<HashSet<_>>();
    let mut unretained = current
        .iter()
        .filter(|(person_id, _)| {
            !mentioned.contains(person_id) && !retained_people.contains(person_id)
        })
        .map(|(_, person)| person)
        .collect::<Vec<_>>();
    unretained.sort_by_key(|person| person.id);
    unretained
}

fn transition_time(
    current_valid_from: NaiveDateTime,
    requested: NaiveDateTime,
    now: NaiveDateTime,
) -> NaiveDateTime {
    requested.max(current_valid_from).min(now)
}

fn opening_time(
    was_previously_closed: bool,
    projected_valid_from: NaiveDateTime,
    now: NaiveDateTime,
) -> NaiveDateTime {
    if was_previously_closed {
        return now;
    }
    projected_valid_from.min(now)
}

fn same_state(current: &PersonProjection, projected: &PersonProjection) -> bool {
    current.profile_account == projected.profile_account
        && current.email == projected.email
        && current.username == projected.username
        && current.display_name == projected.display_name
        && current.first_name == projected.first_name
        && current.last_name == projected.last_name
        && current.attributes == projected.attributes
}

async fn current_people<C>(
    db: &C,
    tenant_id: Uuid,
) -> Result<HashMap<Uuid, CurrentPerson>, PeopleRepoError>
where
    C: ConnectionTrait,
{
    read_current_people(db, tenant_id, None).await
}

async fn read_current_people<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    person: Option<Uuid>,
) -> Result<HashMap<Uuid, CurrentPerson>, PeopleRepoError> {
    const SQL: &str = r"
        SELECT id, person_id, email, username, display_name,
               first_name, last_name, attributes, valid_from,
               profile_source_type, profile_source_id, profile_account_id
        FROM people
        WHERE insight_tenant_id = ? AND valid_to IS NULL
    ";
    let mut sql = SQL.to_owned();
    let mut params = vec![tenant_id.as_bytes().to_vec().into()];
    if let Some(person) = person {
        sql.push_str(" AND person_id = ?");
        params.push(person.as_bytes().to_vec().into());
    }
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql,
            params,
        ))
        .await?;
    rows.iter()
        .map(decode_current)
        .map(|result| result.map(|person| (person.projection.person_id, person)))
        .collect()
}

pub(crate) async fn current_projection<C: ConnectionTrait>(
    db: &C,
    tenant: Uuid,
    person: Uuid,
) -> Result<Option<PersonProjection>, PeopleRepoError> {
    Ok(read_current_people(db, tenant, Some(person))
        .await?
        .remove(&person)
        .map(|row| row.projection))
}

async fn previously_closed_people<C>(
    db: &C,
    tenant_id: Uuid,
) -> Result<HashSet<Uuid>, PeopleRepoError>
where
    C: ConnectionTrait,
{
    const SQL: &str = r"
        SELECT DISTINCT person_id
        FROM people
        WHERE insight_tenant_id = ? AND valid_to IS NOT NULL
    ";
    db.query_all_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [tenant_id.as_bytes().to_vec().into()],
    ))
    .await?
    .iter()
    .map(|row| {
        let person_id = row.try_get::<Vec<u8>>("", "person_id")?;
        Uuid::from_slice(&person_id).map_err(PeopleRepoError::from)
    })
    .collect()
}

fn decode_current(row: &QueryResult) -> Result<CurrentPerson, PeopleRepoError> {
    let person_id = Uuid::from_slice(&row.try_get::<Vec<u8>>("", "person_id")?)?;
    Ok(CurrentPerson {
        id: row.try_get("", "id")?,
        projection: PersonProjection {
            person_id,
            profile_account: decode_profile_account(row)?.map(Box::new),
            email: row.try_get("", "email")?,
            username: row.try_get("", "username")?,
            display_name: row.try_get("", "display_name")?,
            first_name: row.try_get("", "first_name")?,
            last_name: row.try_get("", "last_name")?,
            attributes: serde_json::from_str(&row.try_get::<String>("", "attributes")?)?,
            valid_from: row.try_get("", "valid_from")?,
        },
    })
}

fn decode_profile_account(row: &QueryResult) -> Result<Option<SourceAccountKey>, PeopleRepoError> {
    let source_type: Option<String> = row.try_get("", "profile_source_type")?;
    let Some(source_type) = source_type else {
        return Ok(None);
    };
    Ok(Some(SourceAccountKey {
        source_type,
        source_id: Uuid::from_slice(&row.try_get::<Vec<u8>>("", "profile_source_id")?)?,
        account_id: row.try_get("", "profile_account_id")?,
    }))
}

pub(crate) async fn current_projections<C: ConnectionTrait>(
    db: &C,
    tenant: Uuid,
) -> Result<HashMap<Uuid, PersonProjection>, PeopleRepoError> {
    Ok(current_people(db, tenant)
        .await?
        .into_iter()
        .map(|(id, person)| (id, person.projection))
        .collect())
}

async fn close(
    txn: &DatabaseTransaction,
    id: u64,
    valid_to: NaiveDateTime,
) -> Result<(), PeopleRepoError> {
    txn.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        "UPDATE people SET valid_to = ? WHERE id = ? AND valid_to IS NULL",
        [valid_to.into(), id.into()],
    ))
    .await?;
    Ok(())
}

async fn insert(
    txn: &DatabaseTransaction,
    tenant_id: Uuid,
    person: &PersonProjection,
    valid_from: NaiveDateTime,
) -> Result<(), PeopleRepoError> {
    const SQL: &str = r"
        INSERT INTO people
            (insight_tenant_id, person_id, email, username, display_name,
             first_name, last_name, attributes, valid_from, valid_to,
             profile_source_type, profile_source_id, profile_account_id)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)
    ";
    txn.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            person.person_id.as_bytes().to_vec().into(),
            person.email.clone().into(),
            person.username.clone().into(),
            person.display_name.clone().into(),
            person.first_name.clone().into(),
            person.last_name.clone().into(),
            serde_json::to_string(&person.attributes)?.into(),
            valid_from.into(),
            person
                .profile_account
                .as_ref()
                .map(|account| account.source_type.clone())
                .into(),
            person
                .profile_account
                .as_ref()
                .map(|account| account.source_id.as_bytes().to_vec())
                .into(),
            person
                .profile_account
                .as_ref()
                .map(|account| account.account_id.clone())
                .into(),
        ],
    ))
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
