use std::collections::HashMap;

use chrono::{NaiveDateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement, Value};
use uuid::Uuid;

use crate::domain::reporting::{ReportingLine, manager_observation};
use crate::domain::seed::SeedProfile;

pub(super) type ReportingTimes = HashMap<(Uuid, String, Uuid), NaiveDateTime>;

const HISTORY_BATCH_SIZE: usize = 200;

pub(super) fn key(line: &ReportingLine) -> (Uuid, String, Uuid) {
    (line.child, line.source_type.clone(), line.source_id)
}

pub(super) fn observed_at(profile: &SeedProfile) -> Option<NaiveDateTime> {
    manager_observation(profile)
        .map(|observation| observation.synced_at)
        .or_else(|| {
            profile
                .roster_membership
                .map(|membership| membership.observed_at)
        })
}

pub(super) async fn effective_times(
    txn: &DatabaseTransaction,
    tenant: Uuid,
    before: &[ReportingLine],
    after: &[ReportingLine],
    observed: &ReportingTimes,
) -> anyhow::Result<ReportingTimes> {
    let now = Utc::now().naive_utc();
    let mut times = proposed_times(before, after, observed, now);
    constrain_to_history(txn, tenant, &mut times).await?;
    Ok(times)
}

fn proposed_times(
    before: &[ReportingLine],
    after: &[ReportingLine],
    observed: &ReportingTimes,
    now: NaiveDateTime,
) -> ReportingTimes {
    let previous: HashMap<_, _> = before.iter().map(|line| (key(line), line)).collect();
    let desired: HashMap<_, _> = after.iter().map(|line| (key(line), line)).collect();
    let mut times = HashMap::new();
    for line in before.iter().chain(after) {
        let key = key(line);
        if previous.get(&key) == desired.get(&key) {
            continue;
        }
        let binding_changed = previous
            .get(&key)
            .is_some_and(|old| old.reference == line.reference && old.parent != line.parent);
        let at = if binding_changed {
            now
        } else {
            observed.get(&key).copied().unwrap_or(now).min(now)
        };
        times.insert(key, at);
    }
    times
}

async fn constrain_to_history(
    txn: &DatabaseTransaction,
    tenant: Uuid,
    times: &mut ReportingTimes,
) -> anyhow::Result<()> {
    let keys: Vec<_> = times.keys().cloned().collect();
    for batch in keys.chunks(HISTORY_BATCH_SIZE) {
        let placeholders = vec!["(?, ?, ?)"; batch.len()].join(", ");
        let sql = format!(
            "SELECT child_person_id, insight_source_type, insight_source_id, MAX(GREATEST(valid_from + INTERVAL 1 MICROSECOND, COALESCE(valid_to, valid_from))) AS boundary FROM org_chart WHERE insight_tenant_id = ? AND (child_person_id, insight_source_type, insight_source_id) IN ({placeholders}) GROUP BY child_person_id, insight_source_type, insight_source_id"
        );
        let mut values: Vec<Value> = vec![tenant.as_bytes().to_vec().into()];
        for (child, source_type, source_id) in batch {
            values.extend([
                child.as_bytes().to_vec().into(),
                source_type.clone().into(),
                source_id.as_bytes().to_vec().into(),
            ]);
        }
        let rows = txn
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                sql,
                values,
            ))
            .await?;
        for row in rows {
            let key = (
                Uuid::from_slice(&row.try_get::<Vec<u8>>("", "child_person_id")?)?,
                row.try_get::<String>("", "insight_source_type")?,
                Uuid::from_slice(&row.try_get::<Vec<u8>>("", "insight_source_id")?)?,
            );
            let boundary = row.try_get::<NaiveDateTime>("", "boundary")?;
            if let Some(at) = times.get_mut(&key) {
                *at = (*at).max(boundary);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::reporting::ManagerReference;
    use crate::domain::seed::SourceAccountKey;

    #[test]
    fn only_source_reference_changes_use_observation_time() {
        let now = Utc::now().naive_utc();
        let observed_at = now - chrono::Duration::days(1);
        let first = ReportingLine {
            child: Uuid::from_u128(1),
            source_type: "directory".to_owned(),
            source_id: Uuid::from_u128(2),
            parent: Some(Uuid::from_u128(3)),
            reference: Some(ManagerReference::Account {
                account: SourceAccountKey {
                    source_type: "directory".to_owned(),
                    source_id: Uuid::from_u128(2),
                    account_id: "manager".to_owned(),
                },
            }),
        };
        let rebound = ReportingLine {
            parent: Some(Uuid::from_u128(4)),
            ..first.clone()
        };
        let changed = ReportingLine {
            reference: Some(ManagerReference::NoManager),
            parent: None,
            ..first.clone()
        };
        for (label, previous, next, timestamp, expected) in [
            (
                "initial seed",
                vec![],
                first.clone(),
                Some(observed_at),
                observed_at,
            ),
            (
                "source change",
                vec![first.clone()],
                changed.clone(),
                Some(observed_at),
                observed_at,
            ),
            (
                "binding change",
                vec![first.clone()],
                rebound,
                Some(observed_at),
                now,
            ),
            (
                "operator correction",
                vec![first.clone()],
                changed,
                None,
                now,
            ),
            (
                "future observation",
                vec![],
                first,
                Some(now + chrono::Duration::days(1)),
                now,
            ),
        ] {
            let observed = timestamp.map(|at| (key(&next), at)).into_iter().collect();
            let times = proposed_times(&previous, std::slice::from_ref(&next), &observed, now);
            assert_eq!(times.get(&key(&next)), Some(&expected), "{label}");
        }
    }
}
