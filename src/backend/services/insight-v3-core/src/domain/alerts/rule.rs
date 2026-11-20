use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::condition::{Condition, ConditionError, ConditionShape, Witness};
use super::scalar::{Outcome, UnknownReason};
use crate::domain::definition::{DefinitionId, Page};
use crate::domain::metric_run::Columns;
use crate::domain::query::time_window::{WindowError, WindowRequest};

pub(crate) const DEFAULT_MIN_INTERVAL_SECS: u32 = 60;
pub(crate) const DEFAULT_MAX_INTERVAL_SECS: u32 = 7 * 24 * 3600;
pub(crate) const DEFAULT_MAX_RULES: u64 = 200;
pub(crate) const DEFAULT_EVALUATION_CONCURRENCY: usize = 4;
pub(crate) const DEFAULT_EVALUATION_LOCK_SECS: u64 = 60;
pub(crate) const DEFAULT_NOTIFICATIONS_KEPT_PER_RULE: u64 = 200;

const MAX_NAME_CHARS: usize = 200;
pub(crate) const MAX_CONTEXT_COLUMNS: usize = 5;
const MAX_COLUMN_CHARS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlertName(String);

impl AlertName {
    pub(crate) fn parse(value: &str) -> Result<Self, RuleError> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_NAME_CHARS {
            return Err(RuleError::Name);
        }

        Ok(Self(trimmed.to_owned()))
    }

    pub(crate) fn from_row(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) min_interval_secs: u32,
    pub(crate) max_interval_secs: u32,
    pub(crate) max_rules: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            min_interval_secs: DEFAULT_MIN_INTERVAL_SECS,
            max_interval_secs: DEFAULT_MAX_INTERVAL_SECS,
            max_rules: DEFAULT_MAX_RULES,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Destinations {
    providers: BTreeMap<String, String>,
}

impl Destinations {
    pub(crate) fn new(providers: BTreeMap<String, String>) -> Self {
        Self { providers }
    }

    pub(crate) fn provider_of(&self, name: &str) -> Option<&str> {
        self.providers.get(name).map(String::as_str)
    }

    pub(crate) fn listed(&self) -> Vec<(&str, &str)> {
        self.providers
            .iter()
            .map(|(name, provider)| (name.as_str(), provider.as_str()))
            .collect()
    }
}

/// A rule as an administrator writes it.
#[derive(Debug, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleDraft {
    /// What the alert is called, as people read it: up to 200 characters.
    pub(crate) name: String,
    /// The id of the stored metric to check.
    pub(crate) metric: String,
    /// Which rows must pass, and the test each row takes.
    #[schemars(with = "ConditionShape")]
    #[schema(value_type = ConditionShape)]
    pub(crate) condition: serde_json::Value,
    /// Result columns, by `as_name`, shown beside the condition's in every
    /// notification: up to 5.
    #[serde(default)]
    pub(crate) context_columns: Vec<String>,
    /// The window the metric is run over, as `run_metric` takes it. Omit it
    /// to run the metric unbounded.
    #[serde(default)]
    pub(crate) range: Option<String>,
    /// How often to check, in seconds.
    pub(crate) interval_secs: u32,
    /// The configured destination the notification goes to.
    pub(crate) destination: String,
    /// Whether checks run. Left out, a new alert is enabled and a replaced
    /// one keeps what it had.
    #[serde(default)]
    pub(crate) enabled: Option<bool>,
    /// The revision this write expects to replace. Required to change an
    /// alert that exists; refused when creating one.
    #[serde(default)]
    pub(crate) expected_revision: Option<u32>,
}

impl toolkit::api::api_dto::RequestApiDto for RuleDraft {}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuleSpec {
    pub(crate) name: AlertName,
    pub(crate) metric: DefinitionId,
    pub(crate) condition: Condition,
    pub(crate) context_columns: Vec<String>,
    pub(crate) range: Option<String>,
    pub(crate) interval_secs: u32,
    pub(crate) destination: String,
}

impl RuleSpec {
    pub(crate) fn parse(
        draft: &RuleDraft,
        limits: Limits,
        destinations: &Destinations,
        columns: &Columns,
    ) -> Result<Self, RuleError> {
        let name = AlertName::parse(&draft.name)?;
        let metric = DefinitionId::parse(&draft.metric).map_err(|_| RuleError::Metric)?;
        let condition = Condition::parse(&draft.condition, &columns.kinds)?;
        let context_columns = parse_context_columns(&draft.context_columns, columns)?;

        if let Some(range) = draft.range.as_deref() {
            WindowRequest::parse(Some(range), Some(false)).map_err(RuleError::Range)?;
        }

        if draft.interval_secs < limits.min_interval_secs
            || draft.interval_secs > limits.max_interval_secs
        {
            return Err(RuleError::Interval {
                min: limits.min_interval_secs,
                max: limits.max_interval_secs,
            });
        }

        if destinations.provider_of(&draft.destination).is_none() {
            return Err(RuleError::Destination(draft.destination.clone()));
        }

        Ok(Self {
            name,
            metric,
            condition,
            context_columns,
            range: draft.range.clone(),
            interval_secs: draft.interval_secs,
            destination: draft.destination.clone(),
        })
    }

    pub(crate) fn window(&self) -> Result<WindowRequest, WindowError> {
        WindowRequest::parse(self.range.as_deref(), Some(false))
    }
}

pub(crate) fn parse_context_columns(
    named: &[String],
    columns: &Columns,
) -> Result<Vec<String>, RuleError> {
    if named.len() > MAX_CONTEXT_COLUMNS {
        return Err(RuleError::ContextColumns(ContextColumnsProblem::TooMany));
    }

    let mut kept: Vec<String> = Vec::with_capacity(named.len());
    for name in named {
        if !is_column_name(name) {
            return Err(RuleError::ContextColumns(ContextColumnsProblem::Name));
        }
        if kept.iter().any(|seen| seen == name) {
            return Err(RuleError::ContextColumns(ContextColumnsProblem::Duplicate(
                name.clone(),
            )));
        }
        if !columns.kinds.iter().any(|(column, _)| column == name) {
            return Err(RuleError::ContextColumns(
                ContextColumnsProblem::NoSuchColumn(name.clone()),
            ));
        }
        if columns.groups.contains(name) {
            return Err(RuleError::ContextColumns(ContextColumnsProblem::GroupBy(
                name.clone(),
            )));
        }
        kept.push(name.clone());
    }

    Ok(kept)
}

fn is_column_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_COLUMN_CHARS
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ContextColumnsProblem {
    #[error("holds at most {MAX_CONTEXT_COLUMNS} columns")]
    TooMany,
    #[error("must each be the name of a result column: letters, digits or underscore")]
    Name,
    #[error("names `{0}` twice")]
    Duplicate(String),
    #[error("the metric answers no column `{0}`")]
    NoSuchColumn(String),
    #[error("`{0}` is a group-by column, which every notification shows already")]
    GroupBy(String),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum RuleError {
    #[error("name must be 1 to {MAX_NAME_CHARS} characters")]
    Name,
    #[error("metric must be a metric id")]
    Metric,
    #[error("{}: {}", .0.field(), .0.problem)]
    Condition(#[from] ConditionError),
    #[error("context_columns: {0}")]
    ContextColumns(ContextColumnsProblem),
    #[error("range: {0}")]
    Range(WindowError),
    #[error("interval_secs must be {min} to {max}")]
    Interval { min: u32, max: u32 },
    #[error("no destination named `{0}` is configured")]
    Destination(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct EvaluationState {
    pub(crate) last_evaluated_at: Option<DateTime<Utc>>,
    pub(crate) last_outcome: Option<Outcome>,
    pub(crate) last_valid_breached: Option<bool>,
    pub(crate) breached_since: Option<DateTime<Utc>>,
}

impl EvaluationState {
    pub(crate) fn last_reason(&self) -> Option<UnknownReason> {
        self.last_outcome.as_ref().and_then(Outcome::reason)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AlertRule {
    pub(crate) id: Uuid,
    pub(crate) spec: RuleSpec,
    pub(crate) enabled: bool,
    pub(crate) revision: u32,
    pub(crate) state: EvaluationState,
    pub(crate) created_by: Option<Uuid>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationStatus {
    Pending,
    Cancelled,
    Sent,
    Failed,
}

impl NotificationStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Cancelled => "cancelled",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        [Self::Pending, Self::Cancelled, Self::Sent, Self::Failed]
            .into_iter()
            .find(|status| status.as_str() == value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Notification {
    pub(crate) id: Uuid,
    pub(crate) rule_id: Uuid,
    pub(crate) rule_revision: u32,
    pub(crate) rule_name: AlertName,
    pub(crate) metric: DefinitionId,
    pub(crate) metric_name: Option<String>,
    pub(crate) condition: Condition,
    pub(crate) context_columns: Vec<String>,
    pub(crate) witness: Witness,
    pub(crate) evaluated_at: DateTime<Utc>,
    pub(crate) destination: String,
    pub(crate) status: NotificationStatus,
    pub(crate) attempts: u32,
    pub(crate) last_error: Option<String>,
    pub(crate) provider_receipt: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub(crate) struct Recording {
    pub(crate) rule_id: Uuid,
    pub(crate) revision: u32,
    pub(crate) outcome: Outcome,
    pub(crate) metric_name: Option<String>,
    pub(crate) evaluated_at: DateTime<Utc>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Recorded {
    Accepted(Box<Accepted>),
    Stale,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Accepted {
    pub(crate) rule: AlertRule,
    pub(crate) notification: Option<Notification>,
}

#[derive(Debug, Clone)]
pub(crate) struct Write {
    pub(crate) spec: RuleSpec,
    pub(crate) enabled: bool,
    pub(crate) actor: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlertSummary {
    pub(crate) id: Uuid,
    pub(crate) name: AlertName,
    pub(crate) metric: DefinitionId,
    pub(crate) enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlertPage {
    pub(crate) alerts: Vec<AlertSummary>,
    pub(crate) total: u64,
}

#[async_trait]
pub(crate) trait AlertStore: Send + Sync + fmt::Debug {
    async fn create(&self, write: Write, max_rules: u64) -> Result<AlertRule, AlertStoreError>;

    async fn replace(
        &self,
        id: Uuid,
        expected_revision: u32,
        write: Write,
    ) -> Result<AlertRule, AlertStoreError>;

    async fn get(&self, id: Uuid) -> Result<Option<AlertRule>, AlertStoreError>;

    async fn page(&self, needle: &str, page: Page) -> Result<AlertPage, AlertStoreError>;

    async fn enabled(&self) -> Result<Vec<AlertRule>, AlertStoreError>;

    async fn set_enabled(
        &self,
        id: Uuid,
        expected_revision: u32,
        enabled: bool,
    ) -> Result<AlertRule, AlertStoreError>;

    async fn delete(&self, id: Uuid) -> Result<Option<AlertRule>, AlertStoreError>;

    async fn record(&self, recording: Recording) -> Result<Recorded, AlertStoreError>;

    async fn notifications(
        &self,
        rule_id: Uuid,
        page: Page,
    ) -> Result<Vec<Notification>, AlertStoreError>;

    async fn notification(&self, id: Uuid) -> Result<Option<Notification>, AlertStoreError>;

    async fn pending_notifications(&self) -> Result<Vec<Notification>, AlertStoreError>;

    async fn record_attempt(
        &self,
        id: Uuid,
        attempted: &super::delivery::Attempted,
    ) -> Result<Option<Notification>, AlertStoreError>;
}

#[derive(Debug, Error)]
pub(crate) enum AlertStoreError {
    #[error("alert {0} was not found")]
    NotFound(Uuid),
    #[error("alert {id} is at revision {current}, not {expected}")]
    Conflict {
        id: Uuid,
        current: u32,
        expected: u32,
    },
    #[error("this installation allows at most {0} alerts")]
    TooMany(u64),
    #[error(transparent)]
    Database(#[from] sea_orm::DbErr),
    #[error("an alert row holds `{0}`, which this service never wrote")]
    UnreadableRow(String),
}
