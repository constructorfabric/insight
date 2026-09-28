//! The operations an administrator has on rules, the same through API and
//! MCP: every write lands in the store first and on the schedule second, so
//! a schedule that misses the second half is repaired by reconciling.

use uuid::Uuid;

use super::rule::{
    AlertRule, AlertStore, AlertStoreError, Destinations, Limits, Notification, RuleDraft,
    RuleError, RuleSpec, Write,
};
use super::schedule::{AlertSchedule, ScheduleError, Scheduled};
use crate::domain::definition::{DefinitionKind, DefinitionName, Lookup, NamePage, Page};

#[derive(Debug)]
pub(crate) struct AlertRules<'a> {
    store: &'a dyn AlertStore,
    schedule: &'a dyn AlertSchedule,
    definitions: &'a dyn Lookup,
    limits: Limits,
    destinations: &'a Destinations,
}

impl<'a> AlertRules<'a> {
    pub(crate) fn new(
        store: &'a dyn AlertStore,
        schedule: &'a dyn AlertSchedule,
        definitions: &'a dyn Lookup,
        limits: Limits,
        destinations: &'a Destinations,
    ) -> Self {
        Self {
            store,
            schedule,
            definitions,
            limits,
            destinations,
        }
    }

    pub(crate) fn destinations(&self) -> &Destinations {
        self.destinations
    }

    /// Creates or replaces a rule, and schedules its checks.
    pub(crate) async fn put(
        &self,
        name: &DefinitionName,
        draft: &RuleDraft,
        actor: Option<Uuid>,
    ) -> Result<AlertRule, AlertsError> {
        let spec = RuleSpec::parse(draft, self.limits, self.destinations)?;
        if self
            .definitions
            .get(DefinitionKind::Metric, &spec.metric)
            .await
            .map_err(AlertsError::Definitions)?
            .is_none()
        {
            return Err(AlertsError::MetricMissing(spec.metric.as_str().to_owned()));
        }
        if draft.expected_revision.is_none() && self.store.count().await? >= self.limits.max_rules {
            return Err(AlertsError::TooMany(self.limits.max_rules));
        }

        let rule = self
            .store
            .put(Write {
                name: name.clone(),
                spec,
                enabled: draft.enabled,
                expected_revision: draft.expected_revision,
                actor,
            })
            .await?;

        self.reschedule(&rule).await?;

        Ok(rule)
    }

    pub(crate) async fn get(&self, name: &DefinitionName) -> Result<AlertRule, AlertsError> {
        self.store
            .get(name)
            .await?
            .ok_or_else(|| AlertsError::NotFound(name.as_str().to_owned()))
    }

    pub(crate) async fn page(&self, needle: &str, page: Page) -> Result<NamePage, AlertsError> {
        Ok(self.store.page(needle.trim(), page).await?)
    }

    pub(crate) async fn set_enabled(
        &self,
        name: &DefinitionName,
        expected_revision: u32,
        enabled: bool,
    ) -> Result<AlertRule, AlertsError> {
        let rule = self
            .store
            .set_enabled(name, expected_revision, enabled)
            .await?;

        self.reschedule(&rule).await?;

        Ok(rule)
    }

    /// Removes the rule, its checks and everything recorded for it.
    pub(crate) async fn delete(&self, name: &DefinitionName) -> Result<(), AlertsError> {
        let Some(rule) = self.store.delete(name).await? else {
            return Err(AlertsError::NotFound(name.as_str().to_owned()));
        };

        self.schedule.remove(rule.id).await?;

        Ok(())
    }

    pub(crate) async fn notifications(
        &self,
        name: &DefinitionName,
        page: Page,
    ) -> Result<Vec<Notification>, AlertsError> {
        let rule = self.get(name).await?;

        Ok(self.store.notifications(rule.id, page).await?)
    }

    async fn reschedule(&self, rule: &AlertRule) -> Result<(), ScheduleError> {
        if rule.enabled {
            return self.schedule.upsert(Scheduled::of(rule)).await;
        }

        self.schedule.remove(rule.id).await
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AlertsError {
    #[error(transparent)]
    Invalid(#[from] RuleError),
    #[error("metric `{0}` was not found")]
    MetricMissing(String),
    #[error("alert `{0}` was not found")]
    NotFound(String),
    #[error("alert `{0}` already exists; send expected_revision to replace it")]
    NameTaken(String),
    #[error("alert `{name}` is at revision {current}, not {expected}")]
    Conflict {
        name: String,
        current: u32,
        expected: u32,
    },
    #[error("this installation allows at most {0} alerts")]
    TooMany(u64),
    #[error(transparent)]
    Store(AlertStoreError),
    #[error(transparent)]
    Definitions(crate::domain::definition::DefinitionStoreError),
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
}

impl From<AlertStoreError> for AlertsError {
    fn from(error: AlertStoreError) -> Self {
        match error {
            AlertStoreError::NotFound(name) => Self::NotFound(name),
            AlertStoreError::NameTaken(name) => Self::NameTaken(name),
            AlertStoreError::Conflict {
                name,
                current,
                expected,
            } => Self::Conflict {
                name,
                current,
                expected,
            },
            AlertStoreError::Database(_) | AlertStoreError::UnreadableRow(_) => Self::Store(error),
        }
    }
}

impl AlertsError {
    /// Whether the caller can act on this, which decides whether it is
    /// logged: a refusal the caller caused is answered, not recorded.
    pub(crate) fn is_about_the_caller(&self) -> bool {
        match self {
            Self::Invalid(_)
            | Self::MetricMissing(_)
            | Self::NotFound(_)
            | Self::NameTaken(_)
            | Self::Conflict { .. }
            | Self::TooMany(_) => true,
            Self::Store(_) | Self::Definitions(_) | Self::Schedule(_) => false,
        }
    }
}
