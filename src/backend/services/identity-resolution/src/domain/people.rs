use std::collections::{BTreeMap, HashMap};

use sea_orm::prelude::DateTime;
use uuid::Uuid;

use super::seed::{IdentityInputRow, PersonAssignment, SeedProfile};

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
pub fn changes(assignments: &[PersonAssignment]) -> Vec<PersonChange> {
    let mut profiles_by_person: HashMap<Uuid, Vec<&SeedProfile>> = HashMap::new();
    for assignment in assignments {
        profiles_by_person
            .entry(assignment.person_id)
            .or_default()
            .extend(&assignment.profiles);
    }

    let mut changes = profiles_by_person
        .into_iter()
        .filter_map(|(person_id, profiles)| change(person_id, &profiles))
        .collect::<Vec<_>>();
    changes.sort_by_key(person_id);
    changes
}

fn change(person_id: Uuid, profiles: &[&SeedProfile]) -> Option<PersonChange> {
    if let Some(profile) = preferred_active_profile(profiles) {
        return Some(PersonChange::Upsert(project(person_id, profile)));
    }

    profiles
        .iter()
        .filter_map(|profile| profile.roster_membership)
        .max_by_key(|membership| membership.observed_at)
        .map(|membership| PersonChange::Close {
            person_id,
            valid_to: membership.observed_at,
        })
}

fn preferred_active_profile<'a>(profiles: &[&'a SeedProfile]) -> Option<&'a SeedProfile> {
    profiles
        .iter()
        .copied()
        .filter(|profile| {
            profile
                .roster_membership
                .is_some_and(|membership| membership.active)
        })
        .max_by_key(|profile| {
            let populated = [
                "display_name",
                "first_name",
                "last_name",
                "username",
                "email",
            ]
            .into_iter()
            .filter(|value_type| profile_value(profile, value_type).is_some())
            .count();
            (
                populated,
                profile.account.source_type.as_str(),
                profile.account.source_id,
                profile.account.account_id.as_str(),
            )
        })
}

fn project(person_id: Uuid, profile: &SeedProfile) -> PersonProjection {
    let membership_at = profile.roster_membership.map_or_else(
        || chrono::DateTime::UNIX_EPOCH.naive_utc(),
        |membership| membership.observed_at,
    );
    PersonProjection {
        person_id,
        email: profile_value(profile, "email"),
        username: profile_value(profile, "username"),
        display_name: profile_value(profile, "display_name"),
        first_name: profile_value(profile, "first_name"),
        last_name: profile_value(profile, "last_name"),
        attributes: profile_attributes(profile),
        valid_from: profile_claims(profile)
            .map(|observation| observation.synced_at)
            .max()
            .map_or(membership_at, |claim_at| claim_at.max(membership_at)),
    }
}

fn profile_value(profile: &SeedProfile, value_type: &str) -> Option<String> {
    let person_value_type = format!("person_{value_type}");
    profile
        .observations
        .iter()
        .filter(|observation| observation.value_type == person_value_type)
        .max_by_key(|observation| observation.synced_at)
        .map(|observation| observation.value.clone())
        .filter(|value| !value.trim().is_empty())
}

fn profile_attributes(profile: &SeedProfile) -> BTreeMap<String, String> {
    let mut attributes: BTreeMap<String, &IdentityInputRow> = BTreeMap::new();
    for observation in profile_claims(profile) {
        let Some(name) = observation.value_type.strip_prefix("person_") else {
            continue;
        };
        if is_core_profile_field(name) || observation.value.trim().is_empty() {
            continue;
        }
        let replace = attributes
            .get(name)
            .is_none_or(|current| current.synced_at <= observation.synced_at);
        if replace {
            attributes.insert(name.to_owned(), observation);
        }
    }
    attributes
        .into_iter()
        .map(|(name, observation)| (name, observation.value.clone()))
        .collect()
}

fn profile_claims(profile: &SeedProfile) -> impl Iterator<Item = &IdentityInputRow> {
    profile.observations.iter().filter(|observation| {
        observation
            .value_type
            .strip_prefix("person_")
            .is_some_and(|name| !is_hierarchy_field(name))
    })
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

        let changes = changes(&assignments);

        let PersonChange::Upsert(projected) = &changes[0] else {
            panic!("active membership should upsert a person");
        };
        assert_eq!(projected.display_name.as_deref(), Some("Roster Name"));
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

        let changes = changes(&assignments);

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

        let changes = changes(&assignments);

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
            changes(&assignments).as_slice(),
            [PersonChange::Close { person_id, .. }] if *person_id == Uuid::from_u128(7)
        ));
    }
}
