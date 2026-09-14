use std::collections::BTreeMap;

use sea_orm::prelude::DateTime;
use uuid::Uuid;

use super::roster::RosterSource;
use super::seed::{IdentityInputRow, SeedProfile, route_value};

pub(crate) mod selection;
pub(crate) use selection::changes_preserving_profiles;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonProjection {
    pub person_id: Uuid,
    pub profile_account: Option<Box<super::seed::SourceAccountKey>>,
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

#[cfg(test)]
fn changes(
    assignments: &[super::seed::PersonAssignment],
    roster: Option<&RosterSource>,
) -> Vec<PersonChange> {
    changes_preserving_profiles(assignments, roster, &std::collections::HashMap::new())
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
        profile_account: (profiles.len() == 1).then(|| Box::new(profiles[0].account.clone())),
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
mod tests;
