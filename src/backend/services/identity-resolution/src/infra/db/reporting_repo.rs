use std::collections::{HashMap, HashSet};

use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement};
use uuid::Uuid;

use crate::domain::reporting::{ReportingError, ReportingLine, project_profile, reject_cycles};
use crate::domain::seed::PersonAssignment;

mod timing;
use timing::{ReportingTimes, key};

#[cfg(test)]
mod tests;

pub(crate) async fn current<C: ConnectionTrait>(
    db: &C,
    tenant: Uuid,
) -> anyhow::Result<Vec<ReportingLine>> {
    let rows = db.query_all_raw(Statement::from_sql_and_values(DbBackend::MySql,
        "SELECT child_person_id, parent_person_id, insight_source_type, insight_source_id, parent_reference FROM org_chart WHERE insight_tenant_id = ? AND valid_to IS NULL LIMIT 100001",
        [tenant.as_bytes().to_vec().into()])).await?;
    anyhow::ensure!(
        rows.len() <= 100_000,
        "reporting graph exceeds the correction limit"
    );
    rows.iter()
        .map(|row| {
            Ok(ReportingLine {
                child: Uuid::from_slice(&row.try_get::<Vec<u8>>("", "child_person_id")?)?,
                parent: row
                    .try_get::<Option<Vec<u8>>>("", "parent_person_id")?
                    .map(|bytes| Uuid::from_slice(&bytes))
                    .transpose()?,
                source_type: row.try_get("", "insight_source_type")?,
                source_id: Uuid::from_slice(&row.try_get::<Vec<u8>>("", "insight_source_id")?)?,
                reference: row
                    .try_get::<Option<String>>("", "parent_reference")?
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            })
        })
        .collect()
}

pub(crate) async fn reconcile(
    txn: &DatabaseTransaction,
    tenant: Uuid,
    author: Uuid,
    before: &[ReportingLine],
    after: &[ReportingLine],
) -> anyhow::Result<u64> {
    reconcile_at(txn, tenant, author, before, after, &HashMap::new()).await
}

async fn reconcile_at(
    txn: &DatabaseTransaction,
    tenant: Uuid,
    author: Uuid,
    before: &[ReportingLine],
    after: &[ReportingLine],
    observed: &ReportingTimes,
) -> anyhow::Result<u64> {
    reject_cycles(after)?;
    let times = timing::effective_times(txn, tenant, before, after, observed).await?;
    let by_key: HashMap<_, _> = after
        .iter()
        .map(|line| ((line.child, &line.source_type, line.source_id), line))
        .collect();
    let mut unchanged = HashSet::new();
    let mut count = 0;
    for line in before {
        let key = (line.child, &line.source_type, line.source_id);
        if by_key.get(&key).is_some_and(|desired| **desired == *line) {
            unchanged.insert(key);
            continue;
        }
        let at = times[&timing::key(line)];
        txn.execute_raw(Statement::from_sql_and_values(DbBackend::MySql,
            "UPDATE org_chart SET valid_to = GREATEST(valid_from, ?) WHERE insight_tenant_id = ? AND insight_source_type = ? AND insight_source_id = ? AND child_person_id = ? AND valid_to IS NULL",
            [at.into(), tenant.as_bytes().to_vec().into(), line.source_type.clone().into(), line.source_id.as_bytes().to_vec().into(), line.child.as_bytes().to_vec().into()])).await?;
    }
    for line in after {
        if unchanged.contains(&(line.child, &line.source_type, line.source_id)) {
            continue;
        }
        let at = times[&key(line)];
        txn.execute_raw(Statement::from_sql_and_values(DbBackend::MySql,
            "INSERT INTO org_chart (insight_tenant_id, insight_source_type, insight_source_id, child_person_id, parent_person_id, author_person_id, reason, valid_from, valid_to, parent_reference) VALUES (?, ?, ?, ?, ?, ?, 'roster-profile', ?, NULL, ?)",
            [tenant.as_bytes().to_vec().into(), line.source_type.clone().into(), line.source_id.as_bytes().to_vec().into(), line.child.as_bytes().to_vec().into(), line.parent.map(|id| id.as_bytes().to_vec()).into(), author.as_bytes().to_vec().into(), at.into(), line.reference.as_ref().map(serde_json::to_string).transpose()?.into()])).await?;
        count += 1;
    }
    Ok(count)
}

pub(crate) async fn reconcile_seed(
    txn: &DatabaseTransaction,
    tenant: Uuid,
    author: Uuid,
    assignments: &[PersonAssignment],
) -> anyhow::Result<()> {
    let before = current(txn, tenant).await?;
    let people = super::people_repo::current_projections(txn, tenant).await?;
    let bindings = super::resolution_repo::current_bindings_in_tenant(
        txn,
        tenant,
        super::resolution_repo::Ceiling::Bounded(super::resolution_repo::MAX_TENANT_BINDINGS),
    )
    .await?;
    anyhow::ensure!(!bindings.truncated, "reporting bindings exceed the limit");
    let profiles: HashMap<_, _> = assignments
        .iter()
        .flat_map(|assignment| &assignment.profiles)
        .map(|profile| (profile.account.clone(), profile.clone()))
        .collect();
    let governed = txn.query_all_raw(Statement::from_sql_and_values(DbBackend::MySql,
        "SELECT DISTINCT person_id, profile_source_type FROM people WHERE insight_tenant_id = ? AND profile_source_type IS NOT NULL",
        [tenant.as_bytes().to_vec().into()])).await?;
    let governed: HashSet<_> = governed
        .iter()
        .map(|row| {
            Ok::<_, anyhow::Error>((
                Uuid::from_slice(&row.try_get::<Vec<u8>>("", "person_id")?)?,
                row.try_get::<String>("", "profile_source_type")?,
            ))
        })
        .collect::<Result<_, _>>()?;
    let mut after: Vec<_> = before
        .iter()
        .filter(|line| !governed.contains(&(line.child, line.source_type.clone())))
        .cloned()
        .collect();
    let mut observed = HashMap::new();
    for person in people.values() {
        let Some(account) = person.profile_account.as_deref() else {
            continue;
        };
        let old: Vec<_> = before
            .iter()
            .filter(|line| {
                line.child == person.person_id && line.source_type == account.source_type
            })
            .collect();
        match project_profile(
            person.person_id,
            account,
            profiles.get(account),
            old.first().copied(),
            &bindings.by_account,
            &profiles,
        ) {
            Ok(line) => {
                if let Some(at) = profiles.get(account).and_then(timing::observed_at) {
                    observed.insert(key(&line), at);
                }
                after.push(line);
            }
            Err(ReportingError::UnresolvedReference) => after.extend(old.into_iter().cloned()),
            Err(error) => return Err(error.into()),
        }
    }
    reconcile_at(txn, tenant, author, &before, &after, &observed).await?;
    Ok(())
}
