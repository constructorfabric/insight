use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::people::selection::{project_selected, same_profile, selected_profile};
use super::people::{PersonChange, PersonProjection};
use super::reporting::{self, ManagerReference, ReportingLine};
use super::resolution::EXCLUDED_PERSON;
use super::roster::RosterSource;
use super::seed::{KnownBinding, SeedProfile, SourceAccountKey};

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub(crate) enum RosterCorrectionError {
    #[error("roster evidence is unavailable; no correction was applied")]
    MissingEvidence,
    #[error("choose a profile account before changing these bindings")]
    ProfileChoiceRequired,
    #[error(transparent)]
    Reporting(#[from] reporting::ReportingError),
}

#[derive(Debug)]
pub(crate) struct CorrectionProjection {
    pub people: Vec<PersonChange>,
    pub reporting: Vec<ReportingLine>,
}

#[derive(Debug)]
pub(crate) struct Snapshot<'a> {
    pub people: &'a HashMap<Uuid, PersonProjection>,
    pub bindings: &'a HashMap<SourceAccountKey, KnownBinding>,
    pub profiles: &'a HashMap<SourceAccountKey, SeedProfile>,
    pub reporting: &'a [ReportingLine],
    pub roster: Option<&'a RosterSource>,
}

pub(crate) fn infer_profile_sources(snapshot: &Snapshot<'_>) -> HashMap<Uuid, PersonProjection> {
    let mut people = snapshot.people.clone();
    for person in people
        .values_mut()
        .filter(|person| person.profile_account.is_none())
    {
        let profiles = eligible(snapshot, snapshot.bindings, person.person_id);
        if let Ok(profile) = choose_existing(snapshot, person, &profiles) {
            person.profile_account = Some(Box::new(profile.account.clone()));
        }
    }
    people
}

pub(crate) fn project(
    snapshot: &Snapshot<'_>,
    after: &HashMap<SourceAccountKey, KnownBinding>,
    affected: &HashSet<Uuid>,
) -> Result<CorrectionProjection, RosterCorrectionError> {
    let mut people = Vec::new();
    let mut selected = HashMap::new();
    let mut closed = HashSet::new();
    let at = chrono::Utc::now().naive_utc();
    for person_id in affected
        .iter()
        .copied()
        .filter(|person| *person != EXCLUDED_PERSON)
    {
        let before_profiles = eligible(snapshot, snapshot.bindings, person_id);
        let remaining = eligible(snapshot, after, person_id);
        let existing = snapshot.people.get(&person_id);
        let chosen = if let Some(existing) = existing {
            let profile = choose_existing(snapshot, existing, &before_profiles)?;
            if after
                .get(&profile.account)
                .is_some_and(|binding| binding.person_id == person_id)
            {
                let mut preserved = existing.clone();
                preserved.profile_account = Some(Box::new(profile.account.clone()));
                preserved.valid_from = at;
                people.push(PersonChange::Upsert(preserved));
                Some(profile)
            } else if remaining.is_empty() {
                ensure_membership_known(snapshot, after, person_id)?;
                closed.insert(person_id);
                people.push(PersonChange::Close {
                    person_id,
                    valid_to: at,
                });
                None
            } else {
                return Err(RosterCorrectionError::ProfileChoiceRequired);
            }
        } else if remaining.is_empty() {
            None
        } else {
            let profile = selected_profile(&remaining, None)
                .ok_or(RosterCorrectionError::ProfileChoiceRequired)?;
            let mut projection = project_selected(person_id, profile, None);
            projection.valid_from = at;
            people.push(PersonChange::Upsert(projection));
            Some(profile)
        };
        if let Some(profile) = chosen {
            selected.insert(person_id, profile);
        }
    }

    let mut reporting = Vec::new();
    let moved: HashSet<_> = after
        .iter()
        .filter(|(account, binding)| {
            snapshot
                .bindings
                .get(*account)
                .is_none_or(|before| before.person_id != binding.person_id)
        })
        .map(|(account, _)| account.clone())
        .collect();
    for line in snapshot.reporting {
        if closed.contains(&line.child)
            && snapshot
                .roster
                .is_some_and(|roster| roster.speaks_for(&line.source_type))
        {
            continue;
        }
        let mut projected = line.clone();
        if line.parent.is_some_and(|parent| affected.contains(&parent))
            || line.reference.as_ref().is_some_and(|reference| {
                reporting::references_accounts(reference, &moved, snapshot.profiles)
            })
        {
            let reference = relationship_reference(snapshot, line)?;
            let reference = corrected_reference(reference, snapshot, after)?;
            projected = reporting::project_reference(line, reference, after, snapshot.profiles)?;
        }
        reporting.push(projected);
    }
    for (person_id, profile) in selected {
        if snapshot.people.contains_key(&person_id) {
            continue;
        }
        reporting.push(reporting::project_profile(
            person_id,
            &profile.account,
            Some(profile),
            None,
            after,
            snapshot.profiles,
        )?);
    }
    reporting::reject_cycles(&reporting)?;
    Ok(CorrectionProjection { people, reporting })
}

fn eligible<'a>(
    snapshot: &'a Snapshot<'_>,
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
    person: Uuid,
) -> Vec<&'a SeedProfile> {
    snapshot
        .profiles
        .values()
        .filter(|profile| {
            snapshot
                .roster
                .is_some_and(|roster| roster.speaks_for(&profile.account.source_type))
                && profile
                    .roster_membership
                    .is_some_and(|membership| membership.active)
                && bindings
                    .get(&profile.account)
                    .is_some_and(|binding| binding.person_id == person)
        })
        .collect()
}

fn ensure_membership_known(
    snapshot: &Snapshot<'_>,
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
    person: Uuid,
) -> Result<(), RosterCorrectionError> {
    for (account, binding) in bindings {
        if binding.person_id != person
            || !snapshot
                .roster
                .is_some_and(|roster| roster.speaks_for(&account.source_type))
        {
            continue;
        }
        if snapshot
            .profiles
            .get(account)
            .and_then(|profile| profile.roster_membership)
            .is_none()
        {
            return Err(RosterCorrectionError::MissingEvidence);
        }
    }
    Ok(())
}

fn choose_existing<'a>(
    snapshot: &Snapshot<'_>,
    existing: &PersonProjection,
    eligible: &[&'a SeedProfile],
) -> Result<&'a SeedProfile, RosterCorrectionError> {
    if let Some(account) = existing.profile_account.as_deref() {
        return eligible
            .iter()
            .copied()
            .find(|profile| profile.account == *account)
            .ok_or(RosterCorrectionError::MissingEvidence);
    }
    let candidates: Vec<_> = eligible
        .iter()
        .copied()
        .filter(|profile| {
            if eligible.len() == 1 {
                return true;
            }
            if !same_profile(
                existing,
                &project_selected(existing.person_id, profile, None),
            ) {
                return false;
            }
            let line = snapshot.reporting.iter().find(|line| {
                line.child == existing.person_id
                    && line.source_type == profile.account.source_type
                    && line.source_id == profile.account.source_id
            });
            let Some(line) = line else {
                return true;
            };
            reporting::observed_reference(profile)
                .ok()
                .flatten()
                .and_then(|reference| {
                    reporting::resolve_reference(&reference, snapshot.bindings, snapshot.profiles)
                        .ok()
                })
                .is_some_and(|parent| parent == line.parent)
        })
        .collect();
    selected_profile(&candidates, Some(existing))
        .ok_or(RosterCorrectionError::ProfileChoiceRequired)
}

fn relationship_reference(
    snapshot: &Snapshot<'_>,
    line: &ReportingLine,
) -> Result<ManagerReference, RosterCorrectionError> {
    if let Some(reference) = &line.reference {
        return Ok(reference.clone());
    }
    let Some(person) = snapshot.people.get(&line.child) else {
        return Ok(ManagerReference::Person {
            person_id: line.parent.ok_or(RosterCorrectionError::MissingEvidence)?,
        });
    };
    if !snapshot
        .roster
        .is_some_and(|roster| roster.speaks_for(&line.source_type))
    {
        return Ok(ManagerReference::Person {
            person_id: line.parent.ok_or(RosterCorrectionError::MissingEvidence)?,
        });
    }
    let profiles = eligible(snapshot, snapshot.bindings, line.child);
    let selected = choose_existing(snapshot, person, &profiles)?;
    let reference =
        reporting::observed_reference(selected)?.ok_or(RosterCorrectionError::MissingEvidence)?;
    if reporting::resolve_reference(&reference, snapshot.bindings, snapshot.profiles)?
        != line.parent
    {
        return Err(RosterCorrectionError::MissingEvidence);
    }
    Ok(reference)
}

fn corrected_reference(
    reference: ManagerReference,
    snapshot: &Snapshot<'_>,
    after: &HashMap<SourceAccountKey, KnownBinding>,
) -> Result<ManagerReference, RosterCorrectionError> {
    let (source_person_id, person_id) = match &reference {
        ManagerReference::Person { person_id } => (*person_id, *person_id),
        ManagerReference::RedirectedPerson {
            source_person_id,
            person_id,
        } => (*source_person_id, *person_id),
        ManagerReference::Account { .. }
        | ManagerReference::Email { .. }
        | ManagerReference::NoManager => return Ok(reference),
    };
    let destinations: HashSet<_> = snapshot
        .bindings
        .iter()
        .filter(|(_, binding)| binding.person_id == person_id)
        .filter_map(|(account, _)| after.get(account).map(|binding| binding.person_id))
        .collect();
    if destinations.len() == 1 {
        let destination = *destinations
            .iter()
            .next()
            .ok_or(RosterCorrectionError::MissingEvidence)?;
        if destination != person_id {
            return Ok(ManagerReference::RedirectedPerson {
                source_person_id,
                person_id: destination,
            });
        }
    }
    if !destinations.contains(&person_id) {
        return Err(RosterCorrectionError::MissingEvidence);
    }
    Ok(reference)
}
