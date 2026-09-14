use std::collections::{HashMap, HashSet};

use sea_orm::{DatabaseConnection, DatabaseTransaction, TransactionTrait};
use uuid::Uuid;

use crate::config::GearConfig;
use crate::domain::people::PersonProjection;
use crate::domain::resolution::{self, BindingRow};
use crate::domain::roster::RosterSource;
use crate::domain::roster_correction::{self, RosterCorrectionError, Snapshot};
use crate::domain::seed::{KnownBinding, SeedProfile, SourceAccountKey};
use crate::infra::db::{SeedLockGuard, people_repo, reporting_repo, resolution_repo};

mod evidence;
use evidence::read_evidence;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CorrectionRunError {
    #[error("another identity operation is running; retry the correction")]
    Busy,
    #[error(transparent)]
    Projection(#[from] RosterCorrectionError),
    #[error("the correction could not be completed")]
    Failed(#[from] anyhow::Error),
}

pub(crate) async fn lock(
    config: &GearConfig,
    tenant: Uuid,
) -> Result<SeedLockGuard, CorrectionRunError> {
    // INVARIANT: The caller retains this guard across evidence reads and transaction commit.
    SeedLockGuard::acquire(&config.database_url, tenant, 0)
        .await?
        .ok_or(CorrectionRunError::Busy)
}

#[derive(Debug)]
pub(crate) struct Evidence {
    pub bindings: HashMap<SourceAccountKey, KnownBinding>,
    pub people: HashMap<Uuid, PersonProjection>,
    pub profiles: HashMap<SourceAccountKey, SeedProfile>,
    pub reporting: Vec<crate::domain::reporting::ReportingLine>,
}

pub(crate) async fn apply(
    db: &DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
    rows: Vec<BindingRow>,
) -> Result<Vec<bool>, CorrectionRunError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    if RosterSource::parse(&config.roster_source_type).is_none() {
        let txn = db.begin().await.map_err(anyhow::Error::from)?;
        let landed = write_rows(&txn, tenant, &rows).await?;
        txn.commit().await.map_err(anyhow::Error::from)?;
        return Ok(landed);
    }
    let accounts: Vec<_> = rows.iter().map(|row| row.account.clone()).collect();
    let targets = rows.iter().map(|row| row.person_id).collect();
    let evidence = read_evidence(db, config, tenant, &accounts, targets).await?;
    apply_evidence(db, config, tenant, &rows, &evidence).await
}

async fn apply_evidence(
    db: &DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
    rows: &[BindingRow],
    evidence: &Evidence,
) -> Result<Vec<bool>, CorrectionRunError> {
    let accounts: Vec<_> = rows.iter().map(|row| row.account.clone()).collect();
    let mut affected: HashSet<_> = rows.iter().map(|row| row.person_id).collect();
    affected.extend(accounts.iter().filter_map(|account| {
        evidence
            .bindings
            .get(account)
            .map(|binding| binding.person_id)
    }));
    let txn = db.begin().await.map_err(anyhow::Error::from)?;
    let landed = write_rows(&txn, tenant, rows).await?;
    let current = resolution_repo::current_bindings(&txn, tenant, &accounts).await?;
    for (row, landed) in rows.iter().zip(&landed) {
        if *landed
            && !current.get(&row.account).is_some_and(|binding| {
                binding.person_id == row.person_id && binding.is_operator_authored()
            })
        {
            return Err(CorrectionRunError::Busy);
        }
    }
    let mut after = evidence.bindings.clone();
    after.extend(current);
    let roster = RosterSource::parse(&config.roster_source_type);
    let snapshot = Snapshot {
        people: &evidence.people,
        bindings: &evidence.bindings,
        profiles: &evidence.profiles,
        reporting: &evidence.reporting,
        roster: roster.as_ref(),
    };
    let projected = roster_correction::project(&snapshot, &after, &affected)?;
    people_repo::reconcile(&txn, tenant, &projected.people, None)
        .await
        .map_err(anyhow::Error::from)?;
    reporting_repo::reconcile(
        &txn,
        tenant,
        rows[0].author_person_id,
        &evidence.reporting,
        &projected.reporting,
    )
    .await?;
    txn.commit().await.map_err(anyhow::Error::from)?;
    Ok(landed)
}

async fn write_rows(
    txn: &DatabaseTransaction,
    tenant: Uuid,
    rows: &[BindingRow],
) -> anyhow::Result<Vec<bool>> {
    let appended = resolution_repo::append_bindings_in(txn, tenant, rows).await?;
    if appended == rows.len() as u64 {
        return Ok(vec![true; rows.len()]);
    }
    let mut present = resolution_repo::present_rows(txn, tenant, rows).await?;
    let missing = resolution::missing(rows, &present);
    if missing.is_empty() {
        return Ok(present);
    }
    let retry = resolution::restamp(&missing, chrono::Utc::now().naive_utc());
    resolution_repo::append_bindings_in(txn, tenant, &retry).await?;
    let recovered = resolution_repo::present_rows(txn, tenant, &retry).await?;
    resolution::apply_recovery(&mut present, &recovered);
    Ok(present)
}

pub(crate) async fn select_profile(
    db: &DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
    author: Uuid,
    person: Uuid,
    account: &SourceAccountKey,
) -> Result<(), CorrectionRunError> {
    if person == resolution::EXCLUDED_PERSON {
        return Err(RosterCorrectionError::ProfileChoiceRequired.into());
    }
    let evidence = read_evidence(
        db,
        config,
        tenant,
        std::slice::from_ref(account),
        HashSet::from([person]),
    )
    .await?;
    let profile = evidence
        .profiles
        .get(account)
        .filter(|profile| {
            profile.account.source_type == config.roster_source_type.trim()
                && profile
                    .roster_membership
                    .is_some_and(|membership| membership.active)
                && evidence
                    .bindings
                    .get(account)
                    .is_some_and(|binding| binding.person_id == person)
        })
        .ok_or(RosterCorrectionError::ProfileChoiceRequired)?;
    let existing = evidence
        .people
        .get(&person)
        .filter(|person| person.profile_account.as_deref() == Some(account));
    let mut projected =
        crate::domain::people::selection::project_selected(person, profile, existing);
    projected.valid_from = chrono::Utc::now().naive_utc();
    let reference = crate::domain::reporting::observed_reference(profile)
        .map_err(RosterCorrectionError::from)?;
    let previous_line = evidence
        .reporting
        .iter()
        .find(|line| line.child == person && line.source_type == account.source_type);
    if reference.is_none()
        && let Some(line) = previous_line
    {
        if existing.is_none() && line.parent.is_some() {
            return Err(RosterCorrectionError::MissingEvidence.into());
        }
        if line.reference.is_none() && line.parent.is_some() {
            return Err(RosterCorrectionError::MissingEvidence.into());
        }
    }
    let line = crate::domain::reporting::project_profile(
        person,
        account,
        Some(profile),
        previous_line.filter(|_| existing.is_some()),
        &evidence.bindings,
        &evidence.profiles,
    )
    .map_err(RosterCorrectionError::from)?;
    let mut reporting: Vec<_> = evidence
        .reporting
        .iter()
        .filter(|line| !(line.child == person && line.source_type == account.source_type))
        .cloned()
        .collect();
    reporting.push(line);
    crate::domain::reporting::reject_cycles(&reporting).map_err(RosterCorrectionError::from)?;
    let txn = db.begin().await.map_err(anyhow::Error::from)?;
    people_repo::reconcile(
        &txn,
        tenant,
        &[crate::domain::people::PersonChange::Upsert(projected)],
        None,
    )
    .await
    .map_err(anyhow::Error::from)?;
    reporting_repo::reconcile(&txn, tenant, author, &evidence.reporting, &reporting).await?;
    journal_profile_source(&txn, tenant, author, person, account).await?;
    txn.commit().await.map_err(anyhow::Error::from)?;
    Ok(())
}

async fn journal_profile_source(
    txn: &DatabaseTransaction,
    tenant: Uuid,
    author: Uuid,
    person: Uuid,
    account: &SourceAccountKey,
) -> anyhow::Result<()> {
    use crate::infra::db::ops_repo;
    let id = Uuid::now_v7();
    let request = serde_json::json!({ "verb": resolution::Verb::ProfileSource.reason_code(), "target_person_id": person, "accounts": [{ "source": account.source_type, "source_id": account.source_id, "account_id": account.account_id, "outcome": "applied" }] });
    ops_repo::enqueue(
        txn,
        id,
        resolution::OPERATION_TYPE,
        tenant,
        author,
        Some(&request.to_string()),
    )
    .await?;
    ops_repo::try_start(txn, id).await?;
    ops_repo::complete(
        txn,
        id,
        "{\"applied\":1,\"already_decided\":0,\"refused\":0}",
    )
    .await
}
