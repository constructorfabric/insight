//! The schedule the handler tests use: which rules are on it, at which
//! revision, and nothing that runs.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::alerts::schedule::{AlertSchedule, ScheduleError, Scheduled};

#[derive(Debug, Default)]
pub(crate) struct MemorySchedule {
    held: Mutex<BTreeMap<Uuid, Scheduled>>,
    failing: bool,
}

impl MemorySchedule {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A schedule that refuses every write, for the cases about one that is
    /// down.
    pub(crate) fn refusing() -> Self {
        Self {
            failing: true,
            ..Self::default()
        }
    }

    fn check(&self) -> Result<(), ScheduleError> {
        if self.failing {
            return Err(ScheduleError(Box::new(std::io::Error::other(
                "schedule is down",
            ))));
        }

        Ok(())
    }

    pub(crate) fn entries(&self) -> Vec<Scheduled> {
        self.lock().values().copied().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Uuid, Scheduled>> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl AlertSchedule for MemorySchedule {
    async fn upsert(&self, scheduled: Scheduled) -> Result<(), ScheduleError> {
        self.check()?;
        self.lock().insert(scheduled.job.rule_id, scheduled);

        Ok(())
    }

    async fn remove(&self, rule_id: Uuid) -> Result<(), ScheduleError> {
        self.check()?;
        self.lock().remove(&rule_id);

        Ok(())
    }

    async fn scheduled(&self) -> Result<Vec<Uuid>, ScheduleError> {
        Ok(self.lock().keys().copied().collect())
    }
}
