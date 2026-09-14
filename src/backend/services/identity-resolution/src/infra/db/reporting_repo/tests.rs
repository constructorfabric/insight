use sea_orm::TransactionTrait;

use super::*;
use crate::config::VisibilityPolicy;
use crate::domain::people::{PersonChange, selection::project_selected};
use crate::domain::reporting::ManagerReference;
use crate::domain::seed::{AssignmentKind, IdentityInputRow, RosterMembership, SeedProfile};
use crate::infra::db::test_fixture::{Fixture, SOURCE_TYPE, fixture_or_skip};
use crate::infra::db::{seed_repo, subchart_repo};

fn at(day: u32) -> chrono::NaiveDateTime {
    chrono::DateTime::UNIX_EPOCH.naive_utc() + chrono::Duration::days(20_000 + i64::from(day))
}

async fn seed(
    fixture: &Fixture,
    person: Uuid,
    parent: Uuid,
    observed_at: chrono::NaiveDateTime,
) -> anyhow::Result<()> {
    let profile = SeedProfile {
        account: fixture.account("child"),
        latest_email: None,
        is_closed: false,
        roster_membership: Some(RosterMembership {
            active: true,
            observed_at: at(1),
        }),
        observations: vec![IdentityInputRow {
            source_type: SOURCE_TYPE.to_owned(),
            source_id: fixture.source_id,
            source_account_id: "child".to_owned(),
            value_type: "parent_person_id".to_owned(),
            value: parent.to_string(),
            synced_at: observed_at,
            is_delete: false,
        }],
    };
    let projection = project_selected(person, &profile, None);
    let assignments = [PersonAssignment {
        person_id: person,
        kind: AssignmentKind::ReusedKnown,
        profiles: vec![profile],
    }];
    seed_repo::apply(
        &fixture.db,
        fixture.tenant,
        Uuid::nil(),
        &[],
        &[PersonChange::Upsert(projection)],
        None,
        &assignments,
    )
    .await?;
    Ok(())
}

async fn visible_at(
    fixture: &Fixture,
    parent: Uuid,
    child: Uuid,
    time: chrono::NaiveDateTime,
) -> anyhow::Result<bool> {
    subchart_repo::is_target_in_visible_set(
        &fixture.db,
        fixture.tenant,
        parent,
        child,
        SOURCE_TYPE,
        Some(time),
        VisibilityPolicy::OrgChart,
    )
    .await
}

#[tokio::test]
async fn seed_preserves_source_effective_reporting_intervals() -> anyhow::Result<()> {
    let Some(fixture) = fixture_or_skip().await? else {
        return Ok(());
    };
    let child = Uuid::now_v7();
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    seed(&fixture, child, first, at(1)).await?;
    assert!(visible_at(&fixture, first, child, at(2)).await?);
    seed(&fixture, child, second, at(3)).await?;
    assert!(visible_at(&fixture, first, child, at(2)).await?);
    assert!(!visible_at(&fixture, first, child, at(3)).await?);
    assert!(visible_at(&fixture, second, child, at(3)).await?);
    seed(&fixture, child, second, at(4)).await?;
    assert!(visible_at(&fixture, second, child, at(3)).await?);
    assert_eq!(history(&fixture, child).await?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn corrections_use_operation_time_and_seed_does_not_overlap_closed_history()
-> anyhow::Result<()> {
    let Some(fixture) = fixture_or_skip().await? else {
        return Ok(());
    };
    let child = Uuid::now_v7();
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    seed(&fixture, child, first, at(1)).await?;
    let before = current(&fixture.db, fixture.tenant).await?;
    let corrected = ReportingLine {
        parent: Some(second),
        reference: Some(ManagerReference::RedirectedPerson {
            source_person_id: first,
            person_id: second,
        }),
        ..before[0].clone()
    };
    let operation_start = chrono::Utc::now().naive_utc() - chrono::Duration::seconds(1);
    let txn = fixture.db.begin().await?;
    reconcile(
        &txn,
        fixture.tenant,
        Uuid::nil(),
        &before,
        std::slice::from_ref(&corrected),
    )
    .await?;
    txn.commit().await?;
    let corrected_history = history(&fixture, child).await?;
    assert!(corrected_history[1].valid_from >= operation_start);
    assert_eq!(
        corrected_history[0].valid_to,
        Some(corrected_history[1].valid_from)
    );
    assert!(visible_at(&fixture, first, child, at(2)).await?);
    assert!(!visible_at(&fixture, second, child, at(2)).await?);
    seed(&fixture, child, first, at(1)).await?;
    assert_eq!(history(&fixture, child).await?, corrected_history);

    let txn = fixture.db.begin().await?;
    reconcile(&txn, fixture.tenant, Uuid::nil(), &[corrected], &[]).await?;
    txn.commit().await?;
    let closed_history = history(&fixture, child).await?;
    seed(&fixture, child, second, at(2)).await?;
    let reopened = history(&fixture, child).await?;
    assert_eq!(reopened.len(), 3);
    assert_eq!(&reopened[..2], closed_history.as_slice());
    let closed_at = closed_history[1]
        .valid_to
        .ok_or_else(|| anyhow::anyhow!("reporting interval must be closed"))?;
    assert!(reopened[2].valid_from >= closed_at);
    assert!(visible_at(&fixture, first, child, at(2)).await?);
    assert!(!visible_at(&fixture, second, child, at(2)).await?);
    Ok(())
}

#[tokio::test]
async fn seed_preserves_corrected_lines_and_history_outside_profile_source() -> anyhow::Result<()> {
    let Some(fixture) = fixture_or_skip().await? else {
        return Ok(());
    };
    let child = fixture.person("child@example.test").await?;
    let first = fixture.person("first@example.test").await?;
    let second = fixture.person("second@example.test").await?;
    fixture
        .observed(child, "parent_person_id", &first.to_string())
        .await?;
    let txn = fixture.db.begin().await?;
    txn.execute_raw(Statement::from_sql_and_values(DbBackend::MySql,
        "UPDATE people SET profile_source_type = 'other-directory', profile_source_id = ?, profile_account_id = 'child' WHERE insight_tenant_id = ? AND person_id = ?",
        [fixture.source_id.as_bytes().to_vec().into(), fixture.tenant.as_bytes().to_vec().into(), child.as_bytes().to_vec().into()])).await?;
    let original = ReportingLine {
        child,
        source_type: SOURCE_TYPE.to_owned(),
        source_id: fixture.source_id,
        parent: Some(first),
        reference: None,
    };
    reconcile(
        &txn,
        fixture.tenant,
        Uuid::nil(),
        &[],
        std::slice::from_ref(&original),
    )
    .await?;
    let corrected = ReportingLine {
        parent: Some(second),
        reference: Some(ManagerReference::RedirectedPerson {
            source_person_id: first,
            person_id: second,
        }),
        ..original.clone()
    };
    reconcile(
        &txn,
        fixture.tenant,
        Uuid::nil(),
        &[original],
        std::slice::from_ref(&corrected),
    )
    .await?;
    txn.commit().await?;
    let original_history = history(&fixture, child).await?;
    for _ in 0..2 {
        seed_repo::apply(
            &fixture.db,
            fixture.tenant,
            Uuid::nil(),
            &[],
            &[],
            None,
            &[],
        )
        .await?;
        assert_eq!(
            current(&fixture.db, fixture.tenant)
                .await?
                .into_iter()
                .find(|line| line.child == child && line.source_type == SOURCE_TYPE),
            Some(corrected.clone())
        );
        assert_eq!(history(&fixture, child).await?, original_history);
    }
    let before = current(&fixture.db, fixture.tenant).await?;
    let after: Vec<_> = before
        .iter()
        .filter(|line| line.source_type != SOURCE_TYPE)
        .cloned()
        .collect();
    let txn = fixture.db.begin().await?;
    reconcile(&txn, fixture.tenant, Uuid::nil(), &before, &after).await?;
    txn.commit().await?;
    let closed_history = history(&fixture, child).await?;
    seed_repo::apply(
        &fixture.db,
        fixture.tenant,
        Uuid::nil(),
        &[],
        &[],
        None,
        &[],
    )
    .await?;
    assert_eq!(history(&fixture, child).await?, closed_history);
    assert!(
        !current(&fixture.db, fixture.tenant)
            .await?
            .iter()
            .any(|line| line.child == child && line.source_type == SOURCE_TYPE)
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct Revision {
    valid_from: chrono::NaiveDateTime,
    valid_to: Option<chrono::NaiveDateTime>,
    reference: Option<String>,
}

async fn history(fixture: &Fixture, child: Uuid) -> anyhow::Result<Vec<Revision>> {
    fixture.db.query_all_raw(Statement::from_sql_and_values(DbBackend::MySql,
        "SELECT valid_from, valid_to, parent_reference FROM org_chart WHERE insight_tenant_id = ? AND child_person_id = ? AND insight_source_type = ? ORDER BY valid_from",
        [fixture.tenant.as_bytes().to_vec().into(), child.as_bytes().to_vec().into(), SOURCE_TYPE.into()])).await?.iter().map(|row| Ok(Revision {
            valid_from: row.try_get("", "valid_from")?, valid_to: row.try_get("", "valid_to")?, reference: row.try_get("", "parent_reference")?,
        })).collect()
}
