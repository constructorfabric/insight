use super::*;
use crate::domain::provenance::Provenance;
use crate::domain::seed::{IdentityInputRow, RosterMembership};

fn account(id: &str) -> SourceAccountKey {
    SourceAccountKey {
        source_type: "directory".to_owned(),
        source_id: Uuid::from_u128(100),
        account_id: id.to_owned(),
    }
}

fn binding(person: u128) -> KnownBinding {
    KnownBinding {
        person_id: Uuid::from_u128(person),
        author_person_id: Uuid::nil(),
        provenance: Provenance::Resolved,
    }
}

fn profile(id: &str) -> SeedProfile {
    let at = chrono::DateTime::UNIX_EPOCH.naive_utc();
    SeedProfile {
        account: account(id),
        latest_email: Some(format!("{id}@example.test")),
        is_closed: false,
        roster_membership: Some(RosterMembership {
            active: true,
            observed_at: at,
        }),
        observations: [
            "display_name",
            "first_name",
            "last_name",
            "username",
            "email",
            "department",
            "division",
            "job_title",
        ]
        .into_iter()
        .map(|field| IdentityInputRow {
            source_type: "directory".to_owned(),
            source_id: account(id).source_id,
            source_account_id: id.to_owned(),
            value_type: format!("person_{field}"),
            value: format!("{id}-{field}"),
            synced_at: at,
            is_delete: false,
        })
        .collect(),
    }
}

fn line(child: u128, parent: u128, native: &str) -> ReportingLine {
    ReportingLine {
        child: Uuid::from_u128(child),
        source_type: "directory".to_owned(),
        source_id: account(native).source_id,
        parent: Some(Uuid::from_u128(parent)),
        reference: Some(ManagerReference::Account {
            account: account(native),
        }),
    }
}

struct Fixture {
    people: HashMap<Uuid, PersonProjection>,
    bindings: HashMap<SourceAccountKey, KnownBinding>,
    profiles: HashMap<SourceAccountKey, SeedProfile>,
    reporting: Vec<ReportingLine>,
    roster: RosterSource,
}

impl Fixture {
    fn new() -> Self {
        let profiles: HashMap<_, _> = ["a", "b", "manager", "report-a", "report-b"]
            .into_iter()
            .map(|id| (account(id), profile(id)))
            .collect();
        let bindings: HashMap<_, _> = [
            ("a", 1),
            ("b", 2),
            ("manager", 3),
            ("report-a", 4),
            ("report-b", 5),
        ]
        .into_iter()
        .map(|(id, person)| (account(id), binding(person)))
        .collect();
        let people = profiles
            .iter()
            .map(|(account, profile)| {
                let person = bindings[account].person_id;
                (person, project_selected(person, profile, None))
            })
            .collect();
        let Some(roster) = RosterSource::parse("directory") else {
            panic!("synthetic roster must parse");
        };
        Self {
            people,
            bindings,
            profiles,
            reporting: vec![
                line(1, 3, "manager"),
                line(2, 3, "manager"),
                line(4, 1, "a"),
                line(5, 2, "b"),
            ],
            roster,
        }
    }

    fn snapshot(&self) -> Snapshot<'_> {
        Snapshot {
            people: &self.people,
            bindings: &self.bindings,
            profiles: &self.profiles,
            reporting: &self.reporting,
            roster: Some(&self.roster),
        }
    }

    fn move_account(&self, moved: &str) -> Result<CorrectionProjection, RosterCorrectionError> {
        let mut after = self.bindings.clone();
        after.insert(account(moved), binding(1));
        project(
            &self.snapshot(),
            &after,
            &HashSet::from([Uuid::from_u128(1), Uuid::from_u128(2)]),
        )
    }
}

#[test]
fn a_merge_keeps_the_targets_complete_profile_manager_and_both_report_groups() -> anyhow::Result<()>
{
    let mut fixture = Fixture::new();
    fixture
        .profiles
        .get_mut(&account("a"))
        .ok_or_else(|| anyhow::anyhow!("required fixture value is missing"))?
        .observations[0]
        .value = "upstream edit not yet seeded".to_owned();
    let projected = fixture.move_account("b")?;
    let target = projected
        .people
        .iter()
        .find_map(|change| match change {
            PersonChange::Upsert(person) if person.person_id == Uuid::from_u128(1) => Some(person),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("required fixture value is missing"))?;
    assert!(same_profile(target, &fixture.people[&Uuid::from_u128(1)]));
    assert_eq!(target.profile_account.as_deref(), Some(&account("a")));
    assert!(projected.people.iter().any(|change| matches!(change, PersonChange::Close { person_id, .. } if *person_id == Uuid::from_u128(2))));
    for (child, parent) in [(1, 3), (4, 1), (5, 1)] {
        assert_eq!(
            projected
                .reporting
                .iter()
                .find(|line| line.child == Uuid::from_u128(child))
                .ok_or_else(|| anyhow::anyhow!("required fixture value is missing"))?
                .parent,
            Some(Uuid::from_u128(parent))
        );
    }
    Ok(())
}

#[test]
fn a_partial_move_redirects_only_reports_of_the_moved_account() -> anyhow::Result<()> {
    let mut fixture = Fixture::new();
    fixture
        .profiles
        .insert(account("b-other"), profile("b-other"));
    fixture.bindings.insert(account("b-other"), binding(2));
    fixture.reporting.push(line(6, 2, "b-other"));
    let projected = fixture.move_account("b-other")?;
    assert_eq!(
        projected
            .reporting
            .iter()
            .find(|line| line.child == Uuid::from_u128(5))
            .ok_or_else(|| anyhow::anyhow!("required fixture value is missing"))?
            .parent,
        Some(Uuid::from_u128(2))
    );
    assert_eq!(
        projected
            .reporting
            .iter()
            .find(|line| line.child == Uuid::from_u128(6))
            .ok_or_else(|| anyhow::anyhow!("required fixture value is missing"))?
            .parent,
        Some(Uuid::from_u128(1))
    );
    assert!(
        !projected
            .people
            .iter()
            .any(|change| matches!(change, PersonChange::Close { .. }))
    );
    Ok(())
}

#[test]
fn moving_the_selected_account_requires_a_replacement_when_another_roster_account_remains() {
    let mut fixture = Fixture::new();
    fixture
        .profiles
        .insert(account("b-other"), profile("b-other"));
    fixture.bindings.insert(account("b-other"), binding(2));
    assert!(matches!(
        fixture.move_account("b"),
        Err(RosterCorrectionError::ProfileChoiceRequired)
    ));
}

#[test]
fn a_merge_cannot_make_the_target_its_own_manager() {
    let mut fixture = Fixture::new();
    fixture.reporting[0] = line(1, 2, "b");
    assert!(matches!(
        fixture.move_account("b"),
        Err(RosterCorrectionError::Reporting(
            reporting::ReportingError::Cycle
        ))
    ));
}

#[test]
fn missing_selected_account_evidence_is_not_inactivity() {
    let mut fixture = Fixture::new();
    fixture.profiles.remove(&account("b"));
    assert!(matches!(
        fixture.move_account("b"),
        Err(RosterCorrectionError::MissingEvidence)
    ));
}

#[test]
fn a_corrected_person_reference_resolves_to_the_surviving_manager() -> anyhow::Result<()> {
    let mut fixture = Fixture::new();
    fixture.reporting[3].reference = Some(ManagerReference::Person {
        person_id: Uuid::from_u128(2),
    });
    let projected = fixture.move_account("b")?;
    let report = projected
        .reporting
        .iter()
        .find(|line| line.child == Uuid::from_u128(5))
        .ok_or_else(|| anyhow::anyhow!("missing report"))?;
    let mut after = fixture.bindings.clone();
    after.insert(account("b"), binding(1));
    let reference = report
        .reference
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing reference"))?;
    assert_eq!(
        reporting::resolve_reference(reference, &after, &fixture.profiles)?,
        Some(Uuid::from_u128(1))
    );
    Ok(())
}

#[test]
fn rebinding_an_excluded_manager_reconnects_account_backed_reports() -> anyhow::Result<()> {
    let mut fixture = Fixture::new();
    let mut excluded = fixture.bindings.clone();
    excluded.insert(
        account("b"),
        KnownBinding {
            person_id: EXCLUDED_PERSON,
            ..binding(2)
        },
    );
    let projected = project(
        &fixture.snapshot(),
        &excluded,
        &HashSet::from([Uuid::from_u128(2), EXCLUDED_PERSON]),
    )?;
    assert_eq!(
        projected
            .reporting
            .iter()
            .find(|line| line.child == Uuid::from_u128(5))
            .and_then(|line| line.parent),
        None
    );
    fixture.reporting = projected.reporting;
    fixture.people.remove(&Uuid::from_u128(2));
    fixture.bindings = excluded;
    let projected = fixture.move_account("b")?;
    assert_eq!(
        projected
            .reporting
            .iter()
            .find(|line| line.child == Uuid::from_u128(5))
            .and_then(|line| line.parent),
        Some(Uuid::from_u128(1))
    );
    Ok(())
}

#[test]
fn remaining_account_without_membership_evidence_prevents_roster_closure() {
    let mut fixture = Fixture::new();
    let mut unknown = profile("b-other");
    unknown.roster_membership = None;
    fixture.profiles.insert(account("b-other"), unknown);
    fixture.bindings.insert(account("b-other"), binding(2));
    assert!(matches!(
        fixture.move_account("b"),
        Err(RosterCorrectionError::MissingEvidence)
    ));
}

#[test]
fn a_later_seed_uses_only_the_preserved_account_and_respects_explicit_clears() {
    let fixture = Fixture::new();
    let existing = &fixture.people[&Uuid::from_u128(1)];
    let mut changed = profile("a");
    changed
        .observations
        .retain(|row| row.value_type != "person_email");
    changed
        .observations
        .iter_mut()
        .filter(|row| {
            matches!(
                row.value_type.as_str(),
                "person_display_name" | "person_department"
            )
        })
        .for_each(|row| {
            row.is_delete = true;
            row.value.clear();
        });
    let projected = project_selected(existing.person_id, &changed, Some(existing));
    assert_eq!(projected.display_name, None);
    assert!(!projected.attributes.contains_key("department"));
    assert_eq!(projected.email, existing.email);
    assert_eq!(
        projected.attributes.get("division"),
        existing.attributes.get("division")
    );
}
