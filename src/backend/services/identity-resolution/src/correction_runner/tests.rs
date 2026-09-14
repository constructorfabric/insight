use super::*;
use crate::domain::people::{PersonChange, selection::project_selected};
use crate::domain::reporting::{ManagerReference, ReportingLine};
use crate::domain::resolution::{Target, Verb};
use crate::domain::seed::{AssignmentKind, IdentityInputRow, PersonAssignment, RosterMembership};
use crate::infra::db::test_fixture::{Fixture, SOURCE_TYPE, fixture_or_skip};

fn source_profile(fixture: &Fixture, name: &str, parent: Option<&str>) -> SeedProfile {
    let at = chrono::Utc::now().naive_utc() - chrono::Duration::days(1);
    let observation = IdentityInputRow {
        source_type: SOURCE_TYPE.to_owned(),
        source_id: fixture.source_id,
        source_account_id: name.to_owned(),
        value_type: "person_display_name".to_owned(),
        value: name.to_owned(),
        synced_at: at,
        is_delete: false,
    };
    let mut observations = vec![observation.clone()];
    if let Some(parent) = parent {
        observations.push(IdentityInputRow {
            value_type: "parent_id".to_owned(),
            value: parent.to_owned(),
            ..observation
        });
    }
    SeedProfile {
        account: fixture.account(name),
        latest_email: None,
        is_closed: false,
        roster_membership: Some(RosterMembership {
            active: true,
            observed_at: at,
        }),
        observations,
    }
}

async fn evidence(fixture: &Fixture, person_reference: bool) -> anyhow::Result<Evidence> {
    let mut people = HashMap::new();
    let mut profiles = HashMap::new();
    let mut ids = HashMap::new();
    for (account, parent) in [
        ("target", None),
        ("duplicate", None),
        ("report", Some("duplicate")),
    ] {
        let person = fixture.person(&format!("{account}@example.test")).await?;
        fixture.bound_at(account, person, "fixture", 20).await?;
        let profile = source_profile(fixture, account, parent);
        people.insert(person, project_selected(person, &profile, None));
        profiles.insert(profile.account.clone(), profile);
        ids.insert(account, person);
    }
    let reference = if person_reference {
        let profile = profiles
            .get_mut(&fixture.account("report"))
            .ok_or_else(|| anyhow::anyhow!("missing report"))?;
        profile.observations[1].value_type = "parent_person_id".to_owned();
        profile.observations[1].value = ids["duplicate"].to_string();
        ManagerReference::Person {
            person_id: ids["duplicate"],
        }
    } else {
        ManagerReference::Account {
            account: fixture.account("duplicate"),
        }
    };
    let reporting = vec![ReportingLine {
        child: ids["report"],
        source_type: SOURCE_TYPE.to_owned(),
        source_id: fixture.source_id,
        parent: Some(ids["duplicate"]),
        reference: Some(reference),
    }];
    let txn = fixture.db.begin().await?;
    let changes: Vec<_> = people.values().cloned().map(PersonChange::Upsert).collect();
    people_repo::reconcile(&txn, fixture.tenant, &changes, None).await?;
    reporting_repo::reconcile(&txn, fixture.tenant, Uuid::nil(), &[], &reporting).await?;
    txn.commit().await?;
    let bindings = resolution_repo::current_bindings_in_tenant(
        &fixture.db,
        fixture.tenant,
        resolution_repo::Ceiling::Bounded(100),
    )
    .await?
    .by_account;
    Ok(Evidence {
        bindings,
        people,
        profiles,
        reporting,
    })
}

fn correction(fixture: &Fixture, evidence: &Evidence) -> Vec<BindingRow> {
    let account = fixture.account("duplicate");
    let target = Target {
        account: account.clone(),
        person_id: evidence.bindings[&fixture.account("target")].person_id,
    };
    resolution::build_rows(
        [(&target, evidence.bindings.get(&account).copied())],
        Uuid::from_u128(1000),
        Verb::Bind,
        chrono::Utc::now().naive_utc(),
    )
}

#[tokio::test]
async fn correction_commits_bindings_roster_and_reporting_before_returning_and_survives_seed()
-> anyhow::Result<()> {
    correction_survives_seed(false, false).await
}

#[tokio::test]
async fn person_reference_correction_survives_seed_with_unchanged_source_evidence()
-> anyhow::Result<()> {
    correction_survives_seed(true, false).await
}

#[tokio::test]
async fn rebinding_an_excluded_manager_restores_visibility_and_survives_seed() -> anyhow::Result<()>
{
    correction_survives_seed(false, true).await
}

async fn correction_survives_seed(
    person_reference: bool,
    exclude_first: bool,
) -> anyhow::Result<()> {
    let Some(fixture) = fixture_or_skip().await? else {
        return Ok(());
    };
    let mut evidence = evidence(&fixture, person_reference).await?;
    let config = GearConfig {
        roster_source_type: SOURCE_TYPE.to_owned(),
        ..GearConfig::default()
    };
    let source = evidence.bindings[&fixture.account("duplicate")].person_id;
    if exclude_first {
        let account = fixture.account("duplicate");
        let target = Target {
            account: account.clone(),
            person_id: resolution::EXCLUDED_PERSON,
        };
        let rows = resolution::build_rows(
            [(&target, evidence.bindings.get(&account).copied())],
            Uuid::from_u128(1000),
            Verb::Exclude,
            chrono::Utc::now().naive_utc(),
        );
        apply_evidence(&fixture.db, &config, fixture.tenant, &rows, &evidence).await?;
        assert!(!fixture.can_see(source, evidence.reporting[0].child).await?);
        evidence.bindings.extend(
            resolution_repo::current_bindings(&fixture.db, fixture.tenant, &[account]).await?,
        );
        evidence.people = people_repo::current_projections(&fixture.db, fixture.tenant).await?;
        evidence.reporting = reporting_repo::current(&fixture.db, fixture.tenant).await?;
        assert_eq!(evidence.reporting[0].parent, None);
    }
    let rows = correction(&fixture, &evidence);
    let target = rows[0].person_id;
    let landed = apply_evidence(&fixture.db, &config, fixture.tenant, &rows, &evidence).await?;
    assert_eq!(landed, vec![true]);
    let report = evidence.reporting[0].child;
    assert!(fixture.can_see(target, report).await?);
    assert!(!fixture.can_see(source, report).await?);
    assert!(!fixture.can_see(report, target).await?);
    let current = people_repo::current_projections(&fixture.db, fixture.tenant).await?;
    assert!(!current.contains_key(&source));
    assert!(crate::domain::people::selection::same_profile(
        &current[&target],
        &evidence.people[&target]
    ));
    assert_eq!(
        reporting_repo::current(&fixture.db, fixture.tenant).await?[0].parent,
        Some(target)
    );

    let bindings = resolution_repo::current_bindings_in_tenant(
        &fixture.db,
        fixture.tenant,
        resolution_repo::Ceiling::Bounded(100),
    )
    .await?
    .by_account;
    let assignments: Vec<_> = evidence
        .profiles
        .values()
        .map(|profile| PersonAssignment {
            person_id: bindings[&profile.account].person_id,
            kind: AssignmentKind::ReusedKnown,
            profiles: vec![profile.clone()],
        })
        .collect();
    let projected = crate::domain::people::changes_preserving_profiles(
        &assignments,
        RosterSource::parse(SOURCE_TYPE).as_ref(),
        &current,
    );
    crate::infra::db::seed_repo::apply(
        &fixture.db,
        fixture.tenant,
        Uuid::nil(),
        &[],
        &projected,
        None,
        &assignments,
    )
    .await?;
    let seeded = people_repo::current_projections(&fixture.db, fixture.tenant).await?;
    assert!(crate::domain::people::selection::same_profile(
        &seeded[&target],
        &current[&target]
    ));
    assert!(!seeded.contains_key(&source));
    assert_eq!(
        reporting_repo::current(&fixture.db, fixture.tenant)
            .await?
            .iter()
            .find(|line| line.child == evidence.reporting[0].child)
            .ok_or_else(|| anyhow::anyhow!("reporting line missing"))?
            .parent,
        Some(target)
    );
    Ok(())
}

#[tokio::test]
async fn rejected_projection_rolls_back_the_binding_write() -> anyhow::Result<()> {
    let Some(fixture) = fixture_or_skip().await? else {
        return Ok(());
    };
    let mut evidence = evidence(&fixture, false).await?;
    let config = GearConfig {
        roster_source_type: SOURCE_TYPE.to_owned(),
        ..GearConfig::default()
    };
    let rows = correction(&fixture, &evidence);
    let source = evidence.bindings[&rows[0].account].person_id;
    evidence.profiles.remove(&fixture.account("duplicate"));
    assert!(
        apply_evidence(&fixture.db, &config, fixture.tenant, &rows, &evidence)
            .await
            .is_err()
    );
    let bindings =
        resolution_repo::current_bindings(&fixture.db, fixture.tenant, &[rows[0].account.clone()])
            .await?;
    assert_eq!(bindings[&rows[0].account].person_id, source);
    assert!(
        people_repo::current_projections(&fixture.db, fixture.tenant)
            .await?
            .contains_key(&source)
    );
    Ok(())
}

#[tokio::test]
async fn a_single_account_correction_and_profile_selection_ignore_unrelated_roster_accounts()
-> anyhow::Result<()> {
    let Some(fixture) = fixture_or_skip().await? else {
        return Ok(());
    };
    let Some(reader) = crate::infra::identity_inputs::tests::reader_or_skip().await? else {
        return Ok(());
    };
    let evidence = evidence(&fixture, false).await?;
    for profile in evidence.profiles.values() {
        crate::infra::identity_inputs::tests::insert_profile(&reader, profile).await?;
    }
    let unrelated: Vec<_> = (0..10_001)
        .map(|index| BindingRow {
            account: fixture.account(&format!("unrelated-{index}")),
            person_id: Uuid::now_v7(),
            author_person_id: Uuid::nil(),
            reason: "fixture".to_owned(),
            created_at: chrono::Utc::now().naive_utc(),
        })
        .collect();
    resolution_repo::append_bindings_in(&fixture.db, fixture.tenant, &unrelated).await?;
    let config = GearConfig {
        roster_source_type: SOURCE_TYPE.to_owned(),
        clickhouse_url: std::env::var("INTEGRATION_TESTS_IDENTITY_INPUTS_URL")?,
        ..GearConfig::default()
    };
    let rows = correction(&fixture, &evidence);
    let target = rows[0].person_id;
    assert_eq!(
        apply(&fixture.db, &config, fixture.tenant, rows).await?,
        vec![true]
    );
    select_profile(
        &fixture.db,
        &config,
        fixture.tenant,
        Uuid::nil(),
        target,
        &fixture.account("target"),
    )
    .await?;
    let report = evidence.reporting[0].child;
    assert!(fixture.can_see(target, report).await?);
    let people = people_repo::current_projections(&fixture.db, fixture.tenant).await?;
    assert!(crate::domain::people::selection::same_profile(
        &people[&target],
        &evidence.people[&target]
    ));
    Ok(())
}
