//! Where the checks are scheduled: one repeating job per enabled rule, keyed
//! by the rule, carrying the revision the checks are for.

use std::fmt;

use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use super::evaluation::EvaluationJob;
use super::rule::AlertRule;

/// One rule's place in the schedule.
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
    /// Puts the rule's checks on the schedule, replacing whatever was there
    /// for it. The first check runs at once.
    async fn upsert(&self, scheduled: Scheduled) -> Result<(), ScheduleError>;

    /// Takes the rule's checks off the schedule. Absent is fine.
    async fn remove(&self, rule_id: Uuid) -> Result<(), ScheduleError>;

    /// Every rule the schedule currently holds checks for.
    async fn scheduled(&self) -> Result<Vec<Uuid>, ScheduleError>;
}

#[derive(Debug, Error)]
#[error("the alert schedule could not be reached")]
pub(crate) struct ScheduleError(#[source] pub(crate) Box<dyn std::error::Error + Send + Sync>);

/// What bringing the schedule in line with the rules changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Reconciled {
    pub(crate) scheduled: usize,
    pub(crate) removed: usize,
}

/// Makes the schedule say what the store says: every enabled rule on it at
/// its revision, nothing else.
///
/// Run at startup, so a schedule that lost entries or kept ones for rules
/// since removed is put right before the first check, and again whenever
/// the two might have drifted.
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
