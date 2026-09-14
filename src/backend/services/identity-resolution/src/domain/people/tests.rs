use super::*;
use crate::domain::roster::RosterSource;
use crate::domain::seed::PersonAssignment;
use crate::domain::seed::{AssignmentKind, IdentityInputRow, RosterMembership, SourceAccountKey};
use std::collections::HashMap;

fn profile(account: &str, active: bool, name: &str) -> SeedProfile {
    let observed_at = chrono::DateTime::UNIX_EPOCH.naive_utc();
    SeedProfile {
        account: SourceAccountKey {
            source_type: "directory".to_owned(),
            source_id: Uuid::from_u128(1),
            account_id: account.to_owned(),
        },
        latest_email: None,
        is_closed: !active,
        roster_membership: Some(RosterMembership {
            active,
            observed_at,
        }),
        observations: vec![IdentityInputRow {
            source_type: "directory".to_owned(),
            source_id: Uuid::from_u128(1),
            source_account_id: account.to_owned(),
            value_type: "display_name".to_owned(),
            value: name.to_owned(),
            synced_at: observed_at,
            is_delete: false,
        }],
    }
}

fn configured_roster() -> RosterSource {
    let Some(roster) = RosterSource::parse("directory") else {
        panic!("directory is a roster source");
    };
    roster
}

#[test]
fn attaching_a_newer_account_preserves_the_complete_target_profile() {
    let person_id = Uuid::from_u128(7);
    let mut original = profile("original", true, "Original Person");
    original.observations[0].value_type = "person_display_name".to_owned();
    for (field, value) in [
        ("email", "original@example.com"),
        ("department", "Research"),
    ] {
        original.observations.push(IdentityInputRow {
            value_type: format!("person_{field}"),
            value: value.to_owned(),
            ..original.observations[0].clone()
        });
    }
    let assignment = PersonAssignment {
        person_id,
        kind: AssignmentKind::ReusedKnown,
        profiles: vec![original.clone()],
    };
    let initial = changes(&[assignment], Some(&configured_roster()));
    let PersonChange::Upsert(existing) = &initial[0] else {
        panic!("expected roster person")
    };
    let mut duplicate = original;
    duplicate.account.account_id = "duplicate".to_owned();
    for observation in &mut duplicate.observations {
        observation.value = "Replacement".to_owned();
        observation.synced_at += chrono::Duration::days(1);
    }
    let assignments = [PersonAssignment {
        person_id,
        kind: AssignmentKind::ReusedKnown,
        profiles: vec![profile_with_original_fields(), duplicate],
    }];
    let updated = changes_preserving_profiles(
        &assignments,
        Some(&configured_roster()),
        &HashMap::from([(person_id, existing.clone())]),
    );
    let PersonChange::Upsert(updated) = &updated[0] else {
        panic!("expected roster person")
    };
    assert_eq!(updated.display_name, existing.display_name);
    assert_eq!(updated.email, existing.email);
    assert_eq!(updated.attributes, existing.attributes);
}

fn profile_with_original_fields() -> SeedProfile {
    let mut original = profile("original", true, "Original Person");
    original.observations[0].value_type = "person_display_name".to_owned();
    for (field, value) in [
        ("email", "original@example.com"),
        ("department", "Research"),
    ] {
        original.observations.push(IdentityInputRow {
            value_type: format!("person_{field}"),
            value: value.to_owned(),
            ..original.observations[0].clone()
        });
    }
    original
}

#[test]
fn only_person_profile_claims_control_presentation() {
    let mut roster = profile("member", true, "Roster Name");
    roster.observations[0].value_type = "person_display_name".to_owned();
    roster.observations.push(IdentityInputRow {
        value_type: "display_name".to_owned(),
        value: "Later Activity Name".to_owned(),
        synced_at: roster.observations[0].synced_at + chrono::Duration::days(1),
        ..roster.observations[0].clone()
    });
    let mut activity = profile("activity", true, "Activity Name");
    activity.roster_membership = None;
    let assignments = vec![PersonAssignment {
        person_id: Uuid::from_u128(7),
        kind: AssignmentKind::Minted,
        profiles: vec![activity, roster],
    }];

    let changes = changes(&assignments, Some(&configured_roster()));

    let PersonChange::Upsert(projected) = &changes[0] else {
        panic!("active membership should upsert a person");
    };
    assert_eq!(projected.display_name.as_deref(), Some("Roster Name"));
}

#[test]
fn competing_accounts_without_a_chosen_source_are_not_blended() {
    let timestamp = chrono::DateTime::UNIX_EPOCH.naive_utc();
    let mut older_richer = profile("older", true, "Older Name");
    older_richer.observations[0].value_type = "person_display_name".to_owned();
    older_richer.observations.push(IdentityInputRow {
        value_type: "person_username".to_owned(),
        value: "stable-handle".to_owned(),
        ..older_richer.observations[0].clone()
    });
    let mut newer = profile("newer", true, "Newer Name");
    newer.observations[0].value_type = "person_display_name".to_owned();
    newer.observations[0].synced_at = timestamp + chrono::Duration::days(1);
    let assignments = vec![PersonAssignment {
        person_id: Uuid::from_u128(7),
        kind: AssignmentKind::ReusedKnown,
        profiles: vec![older_richer, newer],
    }];

    let changes = changes(&assignments, Some(&configured_roster()));

    assert!(changes.is_empty());
}

#[test]
fn typed_profile_values_accept_their_column_limits() {
    for (value_type, limit) in [
        ("email", 320),
        ("username", 320),
        ("display_name", 512),
        ("first_name", 512),
        ("last_name", 512),
    ] {
        let mut roster = profile("member", true, "value");
        roster.observations[0].value_type = format!("person_{value_type}");
        roster.observations[0].value = "x".repeat(limit);

        assert_eq!(
            profile_value(&[&roster], value_type),
            Some("x".repeat(limit)),
            "should accept {value_type} at its column limit"
        );
    }
}

#[test]
fn typed_profile_values_reject_values_over_their_column_limits() {
    for (value_type, limit) in [
        ("email", 320),
        ("username", 320),
        ("display_name", 512),
        ("first_name", 512),
        ("last_name", 512),
    ] {
        let mut roster = profile("member", true, "value");
        roster.observations[0].value_type = format!("person_{value_type}");
        roster.observations[0].value = "x".repeat(limit + 1);

        assert_eq!(
            profile_value(&[&roster], value_type),
            None,
            "should reject {value_type} above its column limit"
        );
    }
}

#[test]
fn roster_profile_claims_project_attributes_without_hierarchy_or_activity_metadata() {
    let timestamp = chrono::DateTime::UNIX_EPOCH.naive_utc();
    let mut roster = profile("member", true, "Roster Name");
    roster.observations[0].value_type = "person_display_name".to_owned();
    roster.observations.extend([
        IdentityInputRow {
            value_type: "person_department".to_owned(),
            value: "Engineering".to_owned(),
            synced_at: timestamp + chrono::Duration::days(1),
            ..roster.observations[0].clone()
        },
        IdentityInputRow {
            value_type: "person_job_title".to_owned(),
            value: "Engineer".to_owned(),
            synced_at: timestamp + chrono::Duration::days(2),
            ..roster.observations[0].clone()
        },
        IdentityInputRow {
            value_type: "person_parent_email".to_owned(),
            value: "manager@example.test".to_owned(),
            synced_at: timestamp + chrono::Duration::days(3),
            ..roster.observations[0].clone()
        },
        IdentityInputRow {
            value_type: "department".to_owned(),
            value: "Activity Department".to_owned(),
            synced_at: timestamp + chrono::Duration::days(4),
            ..roster.observations[0].clone()
        },
    ]);
    let assignments = vec![PersonAssignment {
        person_id: Uuid::from_u128(7),
        kind: AssignmentKind::Minted,
        profiles: vec![roster],
    }];

    let changes = changes(&assignments, Some(&configured_roster()));

    let PersonChange::Upsert(projected) = &changes[0] else {
        panic!("active membership should upsert a person");
    };
    assert_eq!(
        projected.attributes,
        BTreeMap::from([
            ("department".to_owned(), "Engineering".to_owned()),
            ("job_title".to_owned(), "Engineer".to_owned()),
        ])
    );
    assert_eq!(projected.valid_from, timestamp + chrono::Duration::days(2));
}

#[test]
fn roster_activation_bounds_the_profile_revision_start() {
    let timestamp = chrono::DateTime::UNIX_EPOCH.naive_utc();
    let mut roster = profile("member", true, "Roster Name");
    roster.observations[0].value_type = "person_display_name".to_owned();
    roster.observations[0].synced_at = timestamp + chrono::Duration::days(2);
    roster.roster_membership = Some(RosterMembership {
        active: true,
        observed_at: timestamp + chrono::Duration::days(5),
    });
    roster.observations.push(IdentityInputRow {
        value_type: "display_name".to_owned(),
        value: "Later Activity Name".to_owned(),
        synced_at: timestamp + chrono::Duration::days(7),
        ..roster.observations[0].clone()
    });
    let assignments = vec![PersonAssignment {
        person_id: Uuid::from_u128(7),
        kind: AssignmentKind::Minted,
        profiles: vec![roster],
    }];

    let changes = changes(&assignments, Some(&configured_roster()));

    let PersonChange::Upsert(projected) = &changes[0] else {
        panic!("active membership should upsert a person");
    };
    assert_eq!(projected.valid_from, timestamp + chrono::Duration::days(5));
}

#[test]
fn inactive_membership_closes_the_person() {
    let assignments = vec![PersonAssignment {
        person_id: Uuid::from_u128(7),
        kind: AssignmentKind::ReusedKnown,
        profiles: vec![profile("member", false, "Former Member")],
    }];

    assert!(matches!(
        changes(&assignments, Some(&configured_roster())).as_slice(),
        [PersonChange::Close { person_id, .. }] if *person_id == Uuid::from_u128(7)
    ));
}

#[test]
fn membership_from_another_source_does_not_project_a_person() {
    let assignments = vec![PersonAssignment {
        person_id: Uuid::from_u128(7),
        kind: AssignmentKind::ReusedKnown,
        profiles: vec![profile("member", true, "Other Directory Member")],
    }];
    let Some(other_roster) = RosterSource::parse("other-directory") else {
        panic!("other-directory is a roster source");
    };

    assert!(changes(&assignments, Some(&other_roster)).is_empty());
}

#[test]
fn no_source_is_a_roster_when_none_is_configured() {
    let assignments = vec![PersonAssignment {
        person_id: Uuid::from_u128(7),
        kind: AssignmentKind::ReusedKnown,
        profiles: vec![profile("member", true, "Directory Member")],
    }];

    assert!(changes(&assignments, None).is_empty());
}
