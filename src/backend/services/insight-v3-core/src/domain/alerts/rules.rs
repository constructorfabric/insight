//! The operations an administrator has on rules, the same through API and
//! MCP: every write lands in the store first and on the schedule second. The
//! store is the answer; a schedule write that fails is logged and repaired by
//! reconciling.

use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use super::condition::{Condition, ConditionShape};
use super::evaluation::observe;
use super::rule::{
    AlertPage, AlertRule, AlertStore, AlertStoreError, Destinations, Limits, Notification,
    RuleDraft, RuleError, RuleSpec, Write, parse_context_columns,
};
use super::scalar::Outcome;
use super::schedule::{AlertSchedule, Scheduled};
use crate::domain::definition::{DefinitionName, Page};
use crate::domain::metric_run::{Columns, MetricRuns};
use crate::domain::query::time_window::WindowRequest;
use crate::domain::surfaces::CustomError;

/// What the form asks before a rule is saved: what a check of this metric,
/// window and condition would find now.
#[derive(Debug, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewDraft {
    /// The stored metric to check.
    pub(crate) metric: String,
    /// The window the metric is run over, as a rule names it. Omit it to
    /// run the metric unbounded.
    #[serde(default)]
    pub(crate) range: Option<String>,
    /// Which rows must pass, and the test each row takes.
    #[schemars(with = "ConditionShape")]
    #[schema(value_type = ConditionShape)]
    pub(crate) condition: serde_json::Value,
    /// Result columns to show beside the condition's, as a rule names them.
    #[serde(default)]
    pub(crate) context_columns: Vec<String>,
}

impl toolkit::api::api_dto::RequestApiDto for PreviewDraft {}

#[derive(Debug)]
pub(crate) struct AlertRules<'a> {
    store: &'a dyn AlertStore,
    schedule: &'a dyn AlertSchedule,
    runs: MetricRuns<'a>,
    limits: Limits,
    destinations: &'a Destinations,
}

impl<'a> AlertRules<'a> {
    pub(crate) fn new(
        store: &'a dyn AlertStore,
        schedule: &'a dyn AlertSchedule,
        runs: MetricRuns<'a>,
        limits: Limits,
        destinations: &'a Destinations,
    ) -> Self {
        Self {
            store,
            schedule,
            runs,
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

    /// The draft as a rule, its condition typed against the columns its
    /// metric answers.
    async fn checked(&self, draft: &RuleDraft) -> Result<RuleSpec, AlertsError> {
        let metric = DefinitionName::parse(&draft.metric).map_err(|_| RuleError::Metric)?;
        let columns = self.columns_of(&metric).await?;

        Ok(RuleSpec::parse(
            draft,
            self.limits,
            self.destinations,
            &columns,
        )?)
    }

    /// What a check of the draft would find if it ran now: the metric run as
    /// a check runs it and the condition judged as a check judges it.
    pub(crate) async fn preview(&self, draft: &PreviewDraft) -> Result<Outcome, AlertsError> {
        let metric = DefinitionName::parse(&draft.metric).map_err(|_| RuleError::Metric)?;
        let columns = self.columns_of(&metric).await?;
        let condition =
            Condition::parse(&draft.condition, &columns.kinds).map_err(RuleError::from)?;
        let context = parse_context_columns(&draft.context_columns, &columns)?;
        let window =
            WindowRequest::parse(draft.range.as_deref(), Some(false)).map_err(RuleError::Range)?;

        Ok(observe(
            &self.runs,
            "a preview",
            &metric,
            &window,
            &condition,
            &context,
            Utc::now(),
        )
        .await)
    }

    async fn columns_of(&self, metric: &DefinitionName) -> Result<Columns, AlertsError> {
        self.runs
            .columns(metric)
            .await
            .map_err(|error| match error {
                CustomError::NotFound { .. } => {
                    AlertsError::MetricMissing(metric.as_str().to_owned())
                }
                CustomError::Body(_)
                | CustomError::Compile(_)
                | CustomError::DatasetNotReady(_) => AlertsError::MetricUnreadable {
                    metric: metric.as_str().to_owned(),
                    reason: error.to_string(),
                },
                CustomError::Run(_)
                | CustomError::Store(_)
                | CustomError::Datasets(_)
                | CustomError::InUse { .. }
                | CustomError::Widget(_)
                | CustomError::Range(_)
                | CustomError::Unanswerable(_) => AlertsError::Metric(error),
            })
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
    #[error("metric `{metric}` cannot be checked: {reason}")]
    MetricUnreadable { metric: String, reason: String },
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
    Metric(CustomError),
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
            | Self::MetricUnreadable { .. }
            | Self::NotFound(_)
            | Self::RevisionRequired
            | Self::RevisionOnCreate
            | Self::Conflict { .. }
            | Self::TooMany(_) => true,
            Self::Store(_) => false,
            Self::Metric(source) => source.is_about_the_caller(),
        }
    }
}
