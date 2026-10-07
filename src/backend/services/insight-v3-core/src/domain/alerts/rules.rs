//! The operations an administrator has on rules, the same through API and
//! MCP: every write lands in the store first and on the schedule second. The
//! store is the answer; a schedule write that fails is logged and repaired by
//! reconciling.

use uuid::Uuid;

use super::rule::{
    AlertPage, AlertRule, AlertStore, AlertStoreError, Destinations, Limits, Notification,
    RuleDraft, RuleError, RuleSpec, Write,
};
use super::schedule::{AlertSchedule, Scheduled};
use crate::domain::definition::{DefinitionKind, Lookup, Page};

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

    /// Creates a rule, enabled unless the draft says otherwise, and
    /// schedules its checks. The store holds the rule bound.
    pub(crate) async fn create(
        &self,
        draft: &RuleDraft,
        actor: Option<Uuid>,
    ) -> Result<AlertRule, AlertsError> {
        if draft.expected_revision.is_some() {
            return Err(AlertsError::RevisionOnCreate);
        }
        let spec = self.checked(draft).await?;

        let write = Write {
            spec,
            enabled: draft.enabled.unwrap_or(true),
            actor,
        };
        let rule = self.store.create(write, self.limits.max_rules).await?;

        self.reschedule(&rule).await;

        Ok(rule)
    }

    /// Replaces a rule at the revision the draft expects, and reschedules
    /// its checks. A draft silent on `enabled` leaves it as it is: the read
    /// here may be stale, but the revision the store checks is not.
    pub(crate) async fn replace(
        &self,
        id: Uuid,
        draft: &RuleDraft,
        actor: Option<Uuid>,
    ) -> Result<AlertRule, AlertsError> {
        let expected = draft
            .expected_revision
            .ok_or(AlertsError::RevisionRequired)?;
        let spec = self.checked(draft).await?;
        let current = self.get(id).await?;

        let write = Write {
            spec,
            enabled: draft.enabled.unwrap_or(current.enabled),
            actor,
        };
        let rule = self.store.replace(id, expected, write).await?;

        self.reschedule(&rule).await;

        Ok(rule)
    }

    /// The draft as a rule, once its metric is known to exist.
    async fn checked(&self, draft: &RuleDraft) -> Result<RuleSpec, AlertsError> {
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

        Ok(spec)
    }

    pub(crate) async fn get(&self, id: Uuid) -> Result<AlertRule, AlertsError> {
        self.store.get(id).await?.ok_or(AlertsError::NotFound(id))
    }

    pub(crate) async fn page(&self, needle: &str, page: Page) -> Result<AlertPage, AlertsError> {
        Ok(self.store.page(needle.trim(), page).await?)
    }

    /// Turns checks on or off. Turning them on is refused while the rule's
    /// destination is no longer configured: every breach would owe a
    /// notification nothing can carry.
    pub(crate) async fn set_enabled(
        &self,
        id: Uuid,
        expected_revision: u32,
        enabled: bool,
    ) -> Result<AlertRule, AlertsError> {
        if enabled {
            let current = self.get(id).await?;
            self.configured(&current.spec.destination)?;
        }

        let rule = self
            .store
            .set_enabled(id, expected_revision, enabled)
            .await?;

        self.reschedule(&rule).await;

        Ok(rule)
    }

    fn configured(&self, destination: &str) -> Result<(), AlertsError> {
        if self.destinations.provider_of(destination).is_none() {
            return Err(RuleError::Destination(destination.to_owned()).into());
        }

        Ok(())
    }

    /// Removes the rule, its checks and everything recorded for it.
    pub(crate) async fn delete(&self, id: Uuid) -> Result<(), AlertsError> {
        let Some(rule) = self.store.delete(id).await? else {
            return Err(AlertsError::NotFound(id));
        };

        if let Err(error) = self.schedule.remove(rule.id).await {
            tracing::error!(rule_id = %rule.id, error = ?error, "a deleted alert's checks could not be unscheduled");
        }

        Ok(())
    }

    pub(crate) async fn notifications(
        &self,
        id: Uuid,
        page: Page,
    ) -> Result<Vec<Notification>, AlertsError> {
        let rule = self.get(id).await?;

        Ok(self.store.notifications(rule.id, page).await?)
    }

    async fn reschedule(&self, rule: &AlertRule) {
        let written = if rule.enabled {
            self.schedule.upsert(Scheduled::of(rule)).await
        } else {
            self.schedule.remove(rule.id).await
        };

        if let Err(error) = written {
            tracing::error!(rule_id = %rule.id, revision = rule.revision, error = ?error, "an alert's checks could not be rescheduled");
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AlertsError {
    #[error(transparent)]
    Invalid(#[from] RuleError),
    #[error("metric `{0}` was not found")]
    MetricMissing(String),
    #[error("alert {0} was not found")]
    NotFound(Uuid),
    #[error("expected_revision is required to replace an alert")]
    RevisionRequired,
    #[error("expected_revision is not accepted when creating an alert")]
    RevisionOnCreate,
    #[error("alert {id} is at revision {current}, not {expected}")]
    Conflict {
        id: Uuid,
        current: u32,
        expected: u32,
    },
    #[error("this installation allows at most {0} alerts")]
    TooMany(u64),
    #[error(transparent)]
    Store(AlertStoreError),
    #[error(transparent)]
    Definitions(crate::domain::definition::DefinitionStoreError),
}

impl From<AlertStoreError> for AlertsError {
    fn from(error: AlertStoreError) -> Self {
        match error {
            AlertStoreError::NotFound(id) => Self::NotFound(id),
            AlertStoreError::Conflict {
                id,
                current,
                expected,
            } => Self::Conflict {
                id,
                current,
                expected,
            },
            AlertStoreError::TooMany(limit) => Self::TooMany(limit),
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
            | Self::RevisionRequired
            | Self::RevisionOnCreate
            | Self::Conflict { .. }
            | Self::TooMany(_) => true,
            Self::Store(_) | Self::Definitions(_) => false,
        }
    }
}
