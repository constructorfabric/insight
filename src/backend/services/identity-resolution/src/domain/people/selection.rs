use std::collections::{BTreeMap, HashMap};

use uuid::Uuid;

use super::{
    PersonChange, PersonProjection, is_core_profile_field, is_hierarchy_field, profile_claims,
    project, roster_profile,
};
use crate::domain::roster::RosterSource;
use crate::domain::seed::{PersonAssignment, SeedProfile};

pub(crate) fn changes_preserving_profiles(
    assignments: &[PersonAssignment],
    roster: Option<&RosterSource>,
    current: &HashMap<Uuid, PersonProjection>,
) -> Vec<PersonChange> {
    let mut profiles_by_person: HashMap<Uuid, Vec<&SeedProfile>> = HashMap::new();
    for assignment in assignments {
        profiles_by_person
            .entry(assignment.person_id)
            .or_default()
            .extend(&assignment.profiles);
    }

    let mut result = Vec::new();
    for (person_id, profiles) in profiles_by_person {
        let eligible: Vec<_> = profiles
            .into_iter()
            .filter(|profile| roster_profile(profile, roster))
            .collect();
        let active: Vec<_> = eligible
            .iter()
            .copied()
            .filter(|profile| {
                profile
                    .roster_membership
                    .is_some_and(|membership| membership.active)
            })
            .collect();
        let existing = current.get(&person_id);
        if active.is_empty() {
            let missing = eligible
                .iter()
                .any(|profile| profile.roster_membership.is_none());
            if !missing
                && let Some(at) = eligible
                    .iter()
                    .filter_map(|profile| profile.roster_membership)
                    .map(|membership| membership.observed_at)
                    .max()
            {
                result.push(PersonChange::Close {
                    person_id,
                    valid_to: at,
                });
            }
            continue;
        }

        let Some(selected) = selected_profile(&active, existing) else {
            if let Some(existing) = existing {
                result.push(PersonChange::Upsert(existing.clone()));
            }
            continue;
        };
        result.push(PersonChange::Upsert(project_selected(
            person_id, selected, existing,
        )));
    }
    result.sort_by_key(super::person_id);
    result
}

pub(crate) fn selected_profile<'a>(
    profiles: &[&'a SeedProfile],
    current: Option<&PersonProjection>,
) -> Option<&'a SeedProfile> {
    if let Some(account) = current.and_then(|person| person.profile_account.as_deref()) {
        return profiles
            .iter()
            .copied()
            .find(|profile| profile.account == *account);
    }
    if profiles.len() == 1 {
        return profiles.first().copied();
    }
    None
}

pub(crate) fn same_profile(a: &PersonProjection, b: &PersonProjection) -> bool {
    a.email == b.email
        && a.username == b.username
        && a.display_name == b.display_name
        && a.first_name == b.first_name
        && a.last_name == b.last_name
        && a.attributes == b.attributes
}

pub(crate) fn project_selected(
    person_id: Uuid,
    profile: &SeedProfile,
    existing: Option<&PersonProjection>,
) -> PersonProjection {
    let profiles = [profile];
    let at = profile.roster_membership.map_or_else(
        || chrono::DateTime::UNIX_EPOCH.naive_utc(),
        |membership| membership.observed_at,
    );
    let mut result = project(person_id, &profiles, at);
    result.profile_account = Some(Box::new(profile.account.clone()));
    for (name, field) in [
        ("email", &mut result.email),
        ("username", &mut result.username),
        ("display_name", &mut result.display_name),
        ("first_name", &mut result.first_name),
        ("last_name", &mut result.last_name),
    ] {
        let value_type = format!("person_{name}");
        match profile
            .observations
            .iter()
            .find(|observation| observation.value_type == value_type)
        {
            Some(observation) if observation.is_delete || observation.value.trim().is_empty() => {
                *field = None;
            }
            Some(_) if field.is_some() => {}
            Some(_) | None => {
                *field = existing
                    .and_then(|current| field_value(current, name))
                    .cloned();
            }
        }
    }
    result.attributes = existing.map_or_else(BTreeMap::new, |current| current.attributes.clone());
    for observation in profile_claims(&profiles) {
        let Some(name) = observation.value_type.strip_prefix("person_") else {
            continue;
        };
        if is_core_profile_field(name) || is_hierarchy_field(name) {
            continue;
        }
        if observation.is_delete || observation.value.trim().is_empty() {
            result.attributes.remove(name);
        } else {
            result
                .attributes
                .insert(name.to_owned(), observation.value.clone());
        }
    }
    result
}

fn field_value<'a>(person: &'a PersonProjection, name: &str) -> Option<&'a String> {
    match name {
        "email" => person.email.as_ref(),
        "username" => person.username.as_ref(),
        "display_name" => person.display_name.as_ref(),
        "first_name" => person.first_name.as_ref(),
        "last_name" => person.last_name.as_ref(),
        _ => None,
    }
}
