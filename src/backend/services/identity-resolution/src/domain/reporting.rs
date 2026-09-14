use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::resolution::EXCLUDED_PERSON;
use super::seed::{IdentityInputRow, KnownBinding, SeedProfile, SourceAccountKey, normalize_email};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SourceEmail {
    pub source_type: String,
    pub source_id: Uuid,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ManagerReference {
    Account {
        account: SourceAccountKey,
    },
    Person {
        person_id: Uuid,
    },
    RedirectedPerson {
        source_person_id: Uuid,
        person_id: Uuid,
    },
    Email {
        source_type: String,
        source_id: Uuid,
        email: String,
    },
    NoManager,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReportingLine {
    pub child: Uuid,
    pub source_type: String,
    pub source_id: Uuid,
    pub parent: Option<Uuid>,
    pub reference: Option<ManagerReference>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReportingError {
    #[error("the manager reference is invalid")]
    InvalidReference,
    #[error("the manager reference cannot be resolved unambiguously")]
    UnresolvedReference,
    #[error("the correction would introduce a reporting cycle")]
    Cycle,
}

pub(crate) fn observed_reference(
    profile: &SeedProfile,
) -> Result<Option<ManagerReference>, ReportingError> {
    if let Some(observation) = manager_observation(profile) {
        if observation.is_delete || observation.value.trim().is_empty() {
            return Ok(Some(ManagerReference::NoManager));
        }
        let reference = match observation.value_type.as_str() {
            "parent_person_id" => ManagerReference::Person {
                person_id: Uuid::parse_str(&observation.value)
                    .map_err(|_| ReportingError::InvalidReference)?,
            },
            "parent_id" => ManagerReference::Account {
                account: SourceAccountKey {
                    account_id: observation.value.clone(),
                    ..profile.account.clone()
                },
            },
            _ => ManagerReference::Email {
                source_type: profile.account.source_type.clone(),
                source_id: profile.account.source_id,
                email: normalize_email(&observation.value),
            },
        };
        return Ok(Some(reference));
    }
    Ok(None)
}

pub(crate) fn manager_observation(profile: &SeedProfile) -> Option<&IdentityInputRow> {
    ["parent_person_id", "parent_id", "parent_email"]
        .into_iter()
        .find_map(|name| {
            profile
                .observations
                .iter()
                .find(|observation| observation.value_type == name)
        })
}

pub(crate) fn resolve_reference(
    reference: &ManagerReference,
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
    profiles: &HashMap<SourceAccountKey, SeedProfile>,
) -> Result<Option<Uuid>, ReportingError> {
    let person = match reference {
        ManagerReference::NoManager => return Ok(None),
        ManagerReference::Person { person_id }
        | ManagerReference::RedirectedPerson { person_id, .. } => *person_id,
        ManagerReference::Account { account } => {
            bindings
                .get(account)
                .ok_or(ReportingError::UnresolvedReference)?
                .person_id
        }
        ManagerReference::Email {
            source_type,
            source_id,
            email,
        } => {
            let people: HashSet<_> = profiles
                .values()
                .filter(|profile| {
                    profile.account.source_type == *source_type
                        && profile.account.source_id == *source_id
                        && profile
                            .latest_email
                            .as_ref()
                            .is_some_and(|value| normalize_email(value) == *email)
                })
                .filter_map(|profile| {
                    bindings
                        .get(&profile.account)
                        .map(|binding| binding.person_id)
                })
                .collect();
            if people.len() != 1 {
                return Err(ReportingError::UnresolvedReference);
            }
            *people
                .iter()
                .next()
                .ok_or(ReportingError::UnresolvedReference)?
        }
    };
    Ok((person != EXCLUDED_PERSON).then_some(person))
}

pub(crate) fn project_reference(
    line: &ReportingLine,
    reference: ManagerReference,
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
    profiles: &HashMap<SourceAccountKey, SeedProfile>,
) -> Result<ReportingLine, ReportingError> {
    Ok(ReportingLine {
        parent: resolve_reference(&reference, bindings, profiles)?,
        reference: Some(reference),
        ..line.clone()
    })
}

pub(crate) fn project_profile(
    person: Uuid,
    account: &SourceAccountKey,
    profile: Option<&SeedProfile>,
    previous: Option<&ReportingLine>,
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
    profiles: &HashMap<SourceAccountKey, SeedProfile>,
) -> Result<ReportingLine, ReportingError> {
    let observed = profile.map(observed_reference).transpose()?.flatten();
    let persisted = previous.and_then(|line| line.reference.as_ref());
    let reference = match (&observed, persisted) {
        (
            Some(ManagerReference::Person { person_id }),
            Some(
                redirected @ ManagerReference::RedirectedPerson {
                    source_person_id, ..
                },
            ),
        ) if person_id == source_person_id => Some(redirected.clone()),
        _ => observed.or_else(|| persisted.cloned()),
    }
    .or_else(|| {
        previous
            .is_none_or(|line| line.parent.is_none())
            .then_some(ManagerReference::NoManager)
    })
    .ok_or(ReportingError::UnresolvedReference)?;
    let line = ReportingLine {
        child: person,
        source_type: account.source_type.clone(),
        source_id: account.source_id,
        parent: None,
        reference: None,
    };
    project_reference(&line, reference, bindings, profiles)
}

pub(crate) fn references_accounts(
    reference: &ManagerReference,
    accounts: &HashSet<SourceAccountKey>,
    profiles: &HashMap<SourceAccountKey, SeedProfile>,
) -> bool {
    match reference {
        ManagerReference::Account { account } => accounts.contains(account),
        ManagerReference::Email {
            source_type,
            source_id,
            email,
        } => accounts.iter().any(|account| {
            account.source_type == *source_type
                && account.source_id == *source_id
                && profiles
                    .get(account)
                    .and_then(|profile| profile.latest_email.as_ref())
                    .is_some_and(|value| normalize_email(value) == *email)
        }),
        ManagerReference::Person { .. }
        | ManagerReference::RedirectedPerson { .. }
        | ManagerReference::NoManager => false,
    }
}

pub(crate) fn reject_cycles(lines: &[ReportingLine]) -> Result<(), ReportingError> {
    let mut by_child: HashMap<(&str, Uuid), Vec<Uuid>> = HashMap::new();
    for line in lines {
        if let Some(parent) = line.parent {
            by_child
                .entry((&line.source_type, line.child))
                .or_default()
                .push(parent);
        }
    }
    let mut complete = HashSet::new();
    for &(source_type, child) in by_child.keys() {
        let mut path = HashSet::new();
        let mut pending = vec![(child, false)];
        while let Some((person, leaving)) = pending.pop() {
            if leaving {
                path.remove(&person);
                complete.insert((source_type, person));
                continue;
            }
            if complete.contains(&(source_type, person)) {
                continue;
            }
            if !path.insert(person) {
                return Err(ReportingError::Cycle);
            }
            pending.push((person, true));
            if let Some(parents) = by_child.get(&(source_type, person)) {
                pending.extend(parents.iter().map(|parent| (*parent, false)));
            }
        }
    }
    Ok(())
}
