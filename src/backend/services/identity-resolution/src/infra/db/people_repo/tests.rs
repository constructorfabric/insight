use std::collections::BTreeMap;

use sea_orm::TransactionTrait as _;

use super::*;
use crate::infra::db::test_fixture::fixture_or_skip;

fn projection(days_after_epoch: i64) -> PersonProjection {
    PersonProjection {
        person_id: Uuid::from_u128(1),
        profile_account: None,
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
fn a_first_projection_starts_at_its_source_time() {
    let source_time = projection(1).valid_from;
    let now = projection(3).valid_from;

    assert_eq!(opening_time(false, source_time, now), source_time);
}

#[test]
fn a_reopened_person_starts_a_new_interval_now() {
    let old_source_time = projection(1).valid_from;
    let now = projection(3).valid_from;

    assert_eq!(opening_time(true, old_source_time, now), now);
}

#[test]
fn a_future_source_time_is_capped_at_now() {
    let now = projection(1).valid_from;
    let future_source_time = projection(3).valid_from;

    assert_eq!(opening_time(false, future_source_time, now), now);
}

#[tokio::test]
async fn reopening_after_close_does_not_overlap_the_previous_interval() -> anyhow::Result<()> {
    let Some(fixture) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person_id = Uuid::now_v7();
    let projected = PersonProjection {
        person_id,
        ..projection(1)
    };

    let txn = fixture.db.begin().await?;
    reconcile(
        &txn,
        fixture.tenant,
        &[PersonChange::Upsert(projected.clone())],
        None,
    )
    .await?;
    txn.commit().await?;

    let txn = fixture.db.begin().await?;
    reconcile(
        &txn,
        fixture.tenant,
        &[PersonChange::Close {
            person_id,
            valid_to: Utc::now().naive_utc(),
        }],
        None,
    )
    .await?;
    txn.commit().await?;

    let txn = fixture.db.begin().await?;
    reconcile(
        &txn,
        fixture.tenant,
        &[PersonChange::Upsert(projected)],
        None,
    )
    .await?;
    txn.commit().await?;

    let rows = fixture
        .db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            r"
                SELECT valid_from, valid_to
                FROM people
                WHERE insight_tenant_id = ? AND person_id = ?
                ORDER BY id
            ",
            [
                fixture.tenant.as_bytes().to_vec().into(),
                person_id.as_bytes().to_vec().into(),
            ],
        ))
        .await?;

    assert_eq!(rows.len(), 2);
    let closed_at = rows[0]
        .try_get::<Option<NaiveDateTime>>("", "valid_to")?
        .ok_or_else(|| anyhow::anyhow!("first interval was not closed"))?;
    let reopened_at = rows[1].try_get::<NaiveDateTime>("", "valid_from")?;
    assert!(closed_at <= reopened_at);
    assert!(
        rows[1]
            .try_get::<Option<NaiveDateTime>>("", "valid_to")?
            .is_none()
    );
    Ok(())
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

#[test]
fn a_current_person_with_no_retained_roster_binding_needs_closure() {
    let current_person = CurrentPerson {
        id: 1,
        projection: projection(1),
    };
    let current = HashMap::from([(current_person.projection.person_id, current_person)]);
    let desired = [PersonChange::Upsert(PersonProjection {
        person_id: Uuid::from_u128(2),
        ..projection(2)
    })];
    let retained = HashSet::from([Uuid::from_u128(2)]);

    assert_eq!(
        unretained_current(&current, &desired, &retained)
            .iter()
            .map(|person| person.id)
            .collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn a_current_person_with_a_retained_roster_binding_stays_open() {
    let current_person = CurrentPerson {
        id: 1,
        projection: projection(1),
    };
    let current = HashMap::from([(current_person.projection.person_id, current_person)]);
    let retained = HashSet::from([Uuid::from_u128(1)]);

    assert!(unretained_current(&current, &[], &retained).is_empty());
}

#[test]
fn an_explicit_closure_is_not_closed_twice() {
    let current_person = CurrentPerson {
        id: 1,
        projection: projection(1),
    };
    let current = HashMap::from([(current_person.projection.person_id, current_person)]);
    let desired = [PersonChange::Close {
        person_id: Uuid::from_u128(1),
        valid_to: chrono::DateTime::UNIX_EPOCH.naive_utc(),
    }];

    assert!(unretained_current(&current, &desired, &HashSet::new()).is_empty());
}
