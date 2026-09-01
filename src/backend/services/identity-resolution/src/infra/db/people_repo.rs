use std::collections::HashMap;

use chrono::{NaiveDateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, QueryResult, Statement};
use uuid::Uuid;

use crate::domain::people::{PersonChange, PersonProjection};

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
) -> Result<ReconcileCounts, PeopleRepoError> {
    let current = current_people(txn, tenant_id).await?;
    let now = Utc::now().naive_utc();
    let mut counts = ReconcileCounts::default();

    for change in changes {
        match change {
            PersonChange::Upsert(projected) => {
                let Some(existing) = current.get(&projected.person_id) else {
                    insert(txn, tenant_id, projected, projected.valid_from.min(now)).await?;
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

    Ok(counts)
}

fn transition_time(
    current_valid_from: NaiveDateTime,
    requested: NaiveDateTime,
    now: NaiveDateTime,
) -> NaiveDateTime {
    requested.max(current_valid_from).min(now)
}

fn same_state(current: &PersonProjection, projected: &PersonProjection) -> bool {
    current.email == projected.email
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
    const SQL: &str = r"
        SELECT id, person_id, email, username, display_name,
               first_name, last_name, attributes, valid_from
        FROM people
        WHERE insight_tenant_id = ? AND valid_to IS NULL
    ";
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [tenant_id.as_bytes().to_vec().into()],
        ))
        .await?;
    rows.iter()
        .map(decode_current)
        .map(|result| result.map(|person| (person.projection.person_id, person)))
        .collect()
}

fn decode_current(row: &QueryResult) -> Result<CurrentPerson, PeopleRepoError> {
    let person_id = Uuid::from_slice(&row.try_get::<Vec<u8>>("", "person_id")?)?;
    Ok(CurrentPerson {
        id: row.try_get("", "id")?,
        projection: PersonProjection {
            person_id,
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
             first_name, last_name, attributes, valid_from, valid_to)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)
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
        ],
    ))
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn projection(days_after_epoch: i64) -> PersonProjection {
        PersonProjection {
            person_id: Uuid::from_u128(1),
            email: Some("person@example.test".to_owned()),
            username: Some("person".to_owned()),
            display_name: Some("Example Person".to_owned()),
            first_name: Some("Example".to_owned()),
            last_name: Some("Person".to_owned()),
            attributes: BTreeMap::default(),
            valid_from: chrono::DateTime::UNIX_EPOCH.naive_utc()
                + chrono::Duration::days(days_after_epoch),
        }
    }

    #[test]
    fn a_new_observation_time_does_not_create_a_profile_revision() {
        assert!(same_state(&projection(1), &projection(2)));
    }

    #[test]
    fn a_presentation_change_creates_a_profile_revision() {
        let current = projection(1);
        let mut changed = projection(2);
        changed.display_name = Some("Changed Person".to_owned());

        assert!(!same_state(&current, &changed));
    }

    #[test]
    fn an_attribute_change_creates_a_profile_revision() {
        let current = projection(1);
        let mut changed = projection(2);
        changed
            .attributes
            .insert("department".to_owned(), "Engineering".to_owned());

        assert!(!same_state(&current, &changed));
    }
}
