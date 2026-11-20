use std::fmt;

use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use super::evaluation::EvaluationJob;
use super::rule::AlertRule;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scheduled {
    pub(crate) job: EvaluationJob,
    pub(crate) every_secs: u32,
}

impl Scheduled {
    pub(crate) fn of(rule: &AlertRule) -> Self {
        Self {
            job: EvaluationJob {
                rule_id: rule.id,
                revision: rule.revision,
            },
            every_secs: rule.spec.interval_secs,
        }
    }
}

#[async_trait]
pub(crate) trait AlertSchedule: Send + Sync + fmt::Debug {
    async fn upsert(&self, scheduled: Scheduled) -> Result<(), ScheduleError>;

    async fn remove(&self, rule_id: Uuid) -> Result<(), ScheduleError>;

    async fn scheduled(&self) -> Result<Vec<Uuid>, ScheduleError>;
}

#[derive(Debug, Error)]
#[error("the alert schedule could not be reached")]
pub(crate) struct ScheduleError(#[source] pub(crate) Box<dyn std::error::Error + Send + Sync>);

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Reconciled {
    pub(crate) scheduled: usize,
    pub(crate) removed: usize,
}

pub(crate) async fn reconcile(
    schedule: &dyn AlertSchedule,
    enabled: &[AlertRule],
) -> Result<Reconciled, ScheduleError> {
    let mut reconciled = Reconciled::default();

    for rule in enabled {
        schedule.upsert(Scheduled::of(rule)).await?;
        reconciled.scheduled += 1;
    }

    for held in schedule.scheduled().await? {
        if enabled.iter().any(|rule| rule.id == held) {
            continue;
        }
        schedule.remove(held).await?;
        reconciled.removed += 1;
    }

    Ok(reconciled)
}
