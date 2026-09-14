use std::collections::{BTreeMap, HashMap};

use sea_orm::prelude::DateTime;
use uuid::Uuid;

use super::roster::RosterSource;
use super::seed::{IdentityInputRow, PersonAssignment, SeedProfile, route_value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonProjection {
    pub person_id: Uuid,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub valid_from: DateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonChange {
    Upsert(PersonProjection),
    Close { person_id: Uuid, valid_to: DateTime },
}

#[must_use]
pub fn changes(
    assignments: &[PersonAssignment],
    roster: Option<&RosterSource>,
) -> Vec<PersonChange> {
    let mut profiles_by_person: HashMap<Uuid, Vec<&SeedProfile>> = HashMap::new();
    for assignment in assignments {
        profiles_by_person
            .entry(assignment.person_id)
            .or_default()
            .extend(&assignment.profiles);
    }

    let mut changes = profiles_by_person
        .into_iter()
        .filter_map(|(person_id, profiles)| change(person_id, &profiles, roster))
        .collect::<Vec<_>>();
    changes.sort_by_key(person_id);
    changes
}

fn change(
    person_id: Uuid,
    profiles: &[&SeedProfile],
    roster: Option<&RosterSource>,
) -> Option<PersonChange> {
    if let Some((active_profiles, membership_at)) = active_roster_profiles(profiles, roster) {
        return Some(PersonChange::Upsert(project(
            person_id,
            &active_profiles,
            membership_at,
        )));
    }

    profiles
        .iter()
        .filter(|profile| roster_profile(profile, roster))
        .filter_map(|profile| profile.roster_membership)
        .max_by_key(|membership| membership.observed_at)
        .map(|membership| PersonChange::Close {
            person_id,
            valid_to: membership.observed_at,
        })
}

fn active_roster_profiles<'a>(
    profiles: &[&'a SeedProfile],
    roster: Option<&RosterSource>,
) -> Option<(Vec<&'a SeedProfile>, DateTime)> {
    let profiles = profiles
        .iter()
        .copied()
        .filter_map(|profile| {
            let membership = profile.roster_membership?;
            (roster_profile(profile, roster) && membership.active).then_some(profile)
        })
        .collect::<Vec<_>>();
    let membership_at = profiles
        .iter()
        .filter_map(|profile| profile.roster_membership)
        .map(|membership| membership.observed_at)
        .max()?;
    Some((profiles, membership_at))
}

fn roster_profile(profile: &SeedProfile, roster: Option<&RosterSource>) -> bool {
    roster.is_some_and(|roster| roster.speaks_for(&profile.account.source_type))
}

fn project(
    person_id: Uuid,
    profiles: &[&SeedProfile],
    membership_at: DateTime,
) -> PersonProjection {
    PersonProjection {
        person_id,
        email: profile_value(profiles, "email"),
        username: profile_value(profiles, "username"),
        display_name: profile_value(profiles, "display_name"),
        first_name: profile_value(profiles, "first_name"),
        last_name: profile_value(profiles, "last_name"),
        attributes: profile_attributes(profiles),
        valid_from: profile_claims(profiles)
            .map(|observation| observation.synced_at)
            .max()
            .map_or(membership_at, |claim_at| claim_at.max(membership_at)),
    }
}

fn profile_value(profiles: &[&SeedProfile], value_type: &str) -> Option<String> {
    let person_value_type = format!("person_{value_type}");
    profile_claims(profiles)
        .filter(|observation| observation.value_type == person_value_type)
        .filter(|observation| {
            let (value_id, value_full_text, value) =
                route_value(&observation.value_type, &observation.value);
            value_id.is_some() || value_full_text.is_some() || value.is_some()
        })
        .max_by_key(|observation| claim_order(observation))
        .map(|observation| observation.value.clone())
        .filter(|value| !value.trim().is_empty())
}

fn profile_attributes(profiles: &[&SeedProfile]) -> BTreeMap<String, String> {
    let mut attributes: BTreeMap<String, &IdentityInputRow> = BTreeMap::new();
    for observation in profile_claims(profiles) {
        let Some(name) = observation.value_type.strip_prefix("person_") else {
            continue;
        };
        if is_core_profile_field(name) || observation.value.trim().is_empty() {
            continue;
        }
        let replace = attributes
            .get(name)
            .is_none_or(|current| claim_order(current) <= claim_order(observation));
        if replace {
            attributes.insert(name.to_owned(), observation);
        }
    }
    attributes
        .into_iter()
        .map(|(name, observation)| (name, observation.value.clone()))
        .collect()
}

fn profile_claims<'a>(
    profiles: &'a [&'a SeedProfile],
) -> impl Iterator<Item = &'a IdentityInputRow> {
    profiles.iter().flat_map(|profile| {
        profile.observations.iter().filter(|observation| {
            observation
                .value_type
                .strip_prefix("person_")
                .is_some_and(|name| !is_hierarchy_field(name))
        })
    })
}

fn claim_order(observation: &IdentityInputRow) -> (DateTime, &str, Uuid, &str, &str) {
    (
        observation.synced_at,
        observation.source_type.as_str(),
        observation.source_id,
        observation.source_account_id.as_str(),
        observation.value.as_str(),
    )
}

fn is_core_profile_field(name: &str) -> bool {
    matches!(
        name,
        "email" | "username" | "display_name" | "first_name" | "last_name"
    )
}

fn is_hierarchy_field(name: &str) -> bool {
    matches!(
        name,
        "id" | "parent_email" | "parent_id" | "parent_person_id"
    )
}

fn person_id(change: &PersonChange) -> Uuid {
    match change {
        PersonChange::Upsert(projection) => projection.person_id,
        PersonChange::Close { person_id, .. } => *person_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::roster::RosterSource;
    use crate::domain::seed::{
        AssignmentKind, IdentityInputRow, RosterMembership, SourceAccountKey,
    };

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
    fn each_profile_field_uses_the_latest_active_roster_claim() {
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

        let PersonChange::Upsert(projected) = &changes[0] else {
            panic!("active membership should upsert a person");
        };
        assert_eq!(projected.display_name.as_deref(), Some("Newer Name"));
        assert_eq!(projected.username.as_deref(), Some("stable-handle"));
        assert_eq!(projected.valid_from, timestamp + chrono::Duration::days(1));
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
}
