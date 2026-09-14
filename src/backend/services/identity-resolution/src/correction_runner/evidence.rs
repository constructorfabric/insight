use std::collections::{HashMap, HashSet};

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::config::GearConfig;
use crate::domain::reporting::{
    ManagerReference, SourceEmail, observed_reference, references_accounts,
};
use crate::domain::resolution::EXCLUDED_PERSON;
use crate::domain::roster::RosterSource;
use crate::domain::roster_correction::RosterCorrectionError;
use crate::domain::seed::{KnownBinding, SeedProfile, SourceAccountKey, build_profiles};
use crate::infra::db::{people_repo, reporting_repo, resolution_repo};
use crate::infra::identity_inputs::{ClickHouseIdentityInputsReader, MAX_CORRECTION_ACCOUNTS};

use super::{CorrectionRunError, Evidence};

#[cfg(test)]
mod tests;

pub(super) async fn read_evidence(
    db: &DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
    named: &[SourceAccountKey],
    mut affected: HashSet<Uuid>,
) -> Result<Evidence, CorrectionRunError> {
    let bindings = resolution_repo::current_bindings_in_tenant(
        db,
        tenant,
        resolution_repo::Ceiling::Bounded(resolution_repo::MAX_TENANT_BINDINGS),
    )
    .await?;
    if bindings.truncated {
        return Err(anyhow::anyhow!("correction exceeds the binding limit").into());
    }
    let people = people_repo::current_projections(db, tenant)
        .await
        .map_err(anyhow::Error::from)?;
    let reporting = reporting_repo::current(db, tenant).await?;
    affected.extend(named.iter().filter_map(|account| {
        bindings
            .by_account
            .get(account)
            .map(|binding| binding.person_id)
    }));
    affected.remove(&EXCLUDED_PERSON);
    let roster = RosterSource::parse(&config.roster_source_type);
    let reader = ClickHouseIdentityInputsReader::connect(
        &config.clickhouse_url,
        &config.clickhouse_database,
        &config.clickhouse_user,
        &config.clickhouse_password,
    );
    let mut accounts = roster_accounts(&bindings.by_account, &affected, roster.as_ref());
    accounts.extend(named.iter().cloned());
    let mut read = HashSet::new();
    let mut profiles = HashMap::new();
    load_profiles(&reader, &accounts, &mut read, &mut profiles).await?;

    let named = named.iter().cloned().collect();
    let relevant: Vec<_> = reporting
        .iter()
        .filter(|line| {
            affected.contains(&line.child)
                || line.parent.is_some_and(|parent| affected.contains(&parent))
                || line
                    .reference
                    .as_ref()
                    .is_some_and(|reference| references_accounts(reference, &named, &profiles))
        })
        .collect();
    let children = relevant
        .iter()
        .filter(|line| line.reference.is_none())
        .map(|line| line.child)
        .collect();
    accounts.extend(roster_accounts(
        &bindings.by_account,
        &children,
        roster.as_ref(),
    ));
    load_profiles(&reader, &accounts, &mut read, &mut profiles).await?;

    let mut emails = HashSet::new();
    for line in relevant {
        if let Some(reference) = &line.reference {
            include_reference(reference, &mut accounts, &mut emails);
        }
    }
    for profile in profiles.values() {
        if profile
            .roster_membership
            .is_some_and(|membership| membership.active)
            && let Some(reference) =
                observed_reference(profile).map_err(RosterCorrectionError::from)?
        {
            include_reference(&reference, &mut accounts, &mut emails);
        }
    }
    let email_accounts = reader
        .email_accounts(&emails.into_iter().collect::<Vec<_>>())
        .await?;
    accounts.extend(
        email_accounts
            .into_iter()
            .filter(|account| bindings.by_account.contains_key(account)),
    );
    load_profiles(&reader, &accounts, &mut read, &mut profiles).await?;
    Ok(Evidence {
        bindings: bindings.by_account,
        people,
        profiles,
        reporting,
    })
}

fn roster_accounts(
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
    people: &HashSet<Uuid>,
    roster: Option<&RosterSource>,
) -> HashSet<SourceAccountKey> {
    bindings
        .iter()
        .filter(|(account, binding)| {
            people.contains(&binding.person_id)
                && roster.is_some_and(|roster| roster.speaks_for(&account.source_type))
        })
        .map(|(account, _)| account.clone())
        .collect()
}

fn include_reference(
    reference: &ManagerReference,
    accounts: &mut HashSet<SourceAccountKey>,
    emails: &mut HashSet<SourceEmail>,
) {
    match reference {
        ManagerReference::Account { account } => {
            accounts.insert(account.clone());
        }
        ManagerReference::Email {
            source_type,
            source_id,
            email,
        } => {
            emails.insert(SourceEmail {
                source_type: source_type.clone(),
                source_id: *source_id,
                email: email.clone(),
            });
        }
        ManagerReference::Person { .. }
        | ManagerReference::RedirectedPerson { .. }
        | ManagerReference::NoManager => {}
    }
}

async fn load_profiles(
    reader: &ClickHouseIdentityInputsReader,
    accounts: &HashSet<SourceAccountKey>,
    read: &mut HashSet<SourceAccountKey>,
    profiles: &mut HashMap<SourceAccountKey, SeedProfile>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        accounts.len() <= MAX_CORRECTION_ACCOUNTS,
        "correction exceeds the account evidence limit"
    );
    let missing: Vec<_> = accounts.difference(read).cloned().collect();
    let rows = reader.accounts(&missing).await?;
    for profile in build_profiles(rows) {
        profiles.insert(profile.account.clone(), profile);
    }
    read.extend(missing);
    Ok(())
}
