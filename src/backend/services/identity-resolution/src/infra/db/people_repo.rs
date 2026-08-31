use std::collections::HashMap;

use chrono::{NaiveDateTime, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, QueryResult, Statement,
};
use uuid::Uuid;

use crate::domain::people::{PersonChange, PersonProjection};
use crate::domain::person_card::{PersonCard, compose_name};

use super::persons_repo;

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
) -> anyhow::Result<ReconcileCounts> {
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
}

async fn current_people<C>(db: &C, tenant_id: Uuid) -> anyhow::Result<HashMap<Uuid, CurrentPerson>>
where
    C: ConnectionTrait,
{
    const SQL: &str = r"
        SELECT id, person_id, email, username, display_name,
               first_name, last_name, valid_from
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

fn decode_current(row: &QueryResult) -> anyhow::Result<CurrentPerson> {
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
            valid_from: row.try_get("", "valid_from")?,
        },
    })
}

async fn close(txn: &DatabaseTransaction, id: u64, valid_to: NaiveDateTime) -> anyhow::Result<()> {
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
) -> anyhow::Result<()> {
    const SQL: &str = r"
        INSERT INTO people
            (insight_tenant_id, person_id, email, username, display_name,
             first_name, last_name, valid_from, valid_to)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)
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
            valid_from.into(),
        ],
    ))
    .await?;
    Ok(())
}

pub async fn person_cards(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, PersonCard>> {
    if person_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; person_ids.len()].join(", ");
    let sql = format!(
        "SELECT person_id, email, username, display_name, first_name, last_name FROM people \
         WHERE insight_tenant_id = ? AND valid_to IS NULL \
         AND person_id IN ({placeholders})"
    );
    let mut values = vec![tenant_id.as_bytes().to_vec().into()];
    values.extend(
        person_ids
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

    let mut cards = rows
        .iter()
        .map(decode_card)
        .collect::<anyhow::Result<HashMap<_, _>>>()?;
    let attributes = persons_repo::person_cards(db, tenant_id, person_ids).await?;
    for (person_id, card) in &mut cards {
        let Some(attribute_card) = attributes.get(person_id) else {
            continue;
        };
        card.job_title.clone_from(&attribute_card.job_title);
        card.status.clone_from(&attribute_card.status);
    }

    Ok(cards)
}

fn decode_card(row: &QueryResult) -> anyhow::Result<(Uuid, PersonCard)> {
    let person_id = Uuid::from_slice(&row.try_get::<Vec<u8>>("", "person_id")?)?;
    let first_name: Option<String> = row.try_get("", "first_name")?;
    let last_name: Option<String> = row.try_get("", "last_name")?;
    let stored_display_name: Option<String> = row.try_get("", "display_name")?;
    let display_name =
        stored_display_name.or_else(|| compose_name(first_name.clone(), last_name.clone()));
    Ok((
        person_id,
        PersonCard {
            person_id,
            email: row.try_get("", "email")?,
            username: row.try_get("", "username")?,
            display_name,
            first_name,
            last_name,
            job_title: None,
            status: None,
        },
    ))
}

pub async fn people_in_tenant(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    requested: &[Uuid],
) -> anyhow::Result<Vec<Uuid>> {
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; requested.len()].join(", ");
    let sql = format!(
        "SELECT person_id FROM people WHERE insight_tenant_id = ? AND valid_to IS NULL \
         AND person_id IN ({placeholders})"
    );
    let mut values = vec![tenant_id.as_bytes().to_vec().into()];
    values.extend(
        requested
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
    let current = rows
        .iter()
        .map(|row| {
            let raw: Vec<u8> = row.try_get("", "person_id")?;
            Ok(Uuid::from_slice(&raw)?)
        })
        .collect::<anyhow::Result<std::collections::HashSet<_>>>()?;

    Ok(requested
        .iter()
        .copied()
        .filter(|person_id| current.contains(person_id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection(days_after_epoch: i64) -> PersonProjection {
        PersonProjection {
            person_id: Uuid::from_u128(1),
            email: Some("person@example.test".to_owned()),
            username: Some("person".to_owned()),
            display_name: Some("Example Person".to_owned()),
            first_name: Some("Example".to_owned()),
            last_name: Some("Person".to_owned()),
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
}
