//! A rule as it is written, checked and stored.

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::scalar::{Number, Outcome, UnknownReason};
use crate::domain::definition::{DefinitionName, NamePage, Page};
use crate::domain::query::time_window::{WindowError, WindowRequest};

/// The bounds every installation gets unless it sets its own.
pub(crate) const DEFAULT_MIN_INTERVAL_SECS: u32 = 60;
pub(crate) const DEFAULT_MAX_INTERVAL_SECS: u32 = 7 * 24 * 3600;
pub(crate) const DEFAULT_MAX_RULES: u64 = 200;
pub(crate) const DEFAULT_EVALUATION_CONCURRENCY: usize = 4;
pub(crate) const DEFAULT_EVALUATION_LOCK_SECS: u64 = 60;
pub(crate) const DEFAULT_NOTIFICATIONS_KEPT_PER_RULE: u64 = 200;

const MAX_COLUMN_CHARS: usize = 128;

/// How the observed value is compared with the threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub(crate) enum Operator {
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
}

impl Operator {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        [Self::Gt, Self::Ge, Self::Lt, Self::Le]
            .into_iter()
            .find(|operator| operator.as_str() == value)
    }
}

/// The comparison a rule makes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Condition {
    pub(crate) operator: Operator,
    pub(crate) threshold: Number,
}

impl Condition {
    /// Whether `value` meets the condition, or nothing where the two cannot
    /// be compared exactly.
    pub(crate) fn holds(&self, value: Number) -> Option<bool> {
        use std::cmp::Ordering::{Greater, Less};

        let ordering = value.compare(self.threshold)?;

        Some(match self.operator {
            Operator::Gt => ordering == Greater,
            Operator::Ge => ordering != Less,
            Operator::Lt => ordering == Less,
            Operator::Le => ordering != Greater,
        })
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.operator.as_str(), self.threshold)
    }
}

/// The bounds an installation places on rules.
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

/// The destinations an installation lets rules send to, by name, with the
/// provider each one reaches. Credentials are not here.
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

    /// Every destination as a reader may list it: name and provider.
    pub(crate) fn listed(&self) -> Vec<(&str, &str)> {
        self.providers
            .iter()
            .map(|(name, provider)| (name.as_str(), provider.as_str()))
            .collect()
    }
}

/// A rule as an administrator writes it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleDraft {
    /// The stored metric to check.
    pub(crate) metric: String,
    /// The result column holding the number, by the `as_name` the metric
    /// gives it.
    pub(crate) column: String,
    /// One of `>`, `>=`, `<`, `<=`.
    pub(crate) operator: Operator,
    /// The number the value is compared with.
    pub(crate) threshold: serde_json::Value,
    /// The window the metric is run over, as `run_metric` takes it. Omit it
    /// to run the metric unbounded.
    #[serde(default)]
    pub(crate) range: Option<String>,
    /// How often to check, in seconds.
    pub(crate) interval_secs: u32,
    /// The configured destination the notification goes to.
    pub(crate) destination: String,
    /// Whether checks run. `true` by default.
    #[serde(default = "enabled_by_default")]
    pub(crate) enabled: bool,
    /// The revision this write expects to replace. Required to change a rule
    /// that exists; refused when creating one.
    #[serde(default)]
    pub(crate) expected_revision: Option<u32>,
}

fn enabled_by_default() -> bool {
    true
}

/// What a rule watches, checked against the installation's bounds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuleSpec {
    pub(crate) metric: DefinitionName,
    pub(crate) column: String,
    pub(crate) condition: Condition,
    pub(crate) range: Option<String>,
    pub(crate) interval_secs: u32,
    pub(crate) destination: String,
}

impl RuleSpec {
    /// A draft as a rule, or the first thing wrong with it.
    pub(crate) fn parse(
        draft: &RuleDraft,
        limits: Limits,
        destinations: &Destinations,
    ) -> Result<Self, RuleError> {
        let metric = DefinitionName::parse(&draft.metric).map_err(|_| RuleError::Metric)?;

        let column = draft.column.trim();
        if column.is_empty()
            || column.chars().count() > MAX_COLUMN_CHARS
            || !column
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(RuleError::Column);
        }

        let threshold = Number::parse(&draft.threshold).ok_or(RuleError::Threshold)?;

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
            metric,
            column: column.to_owned(),
            condition: Condition {
                operator: draft.operator,
                threshold,
            },
            range: draft.range.clone(),
            interval_secs: draft.interval_secs,
            destination: draft.destination.clone(),
        })
    }

    /// The window a check runs the metric over: the rule's range, never
    /// bucketed, because one row is what a check reads.
    pub(crate) fn window(&self) -> Result<WindowRequest, WindowError> {
        WindowRequest::parse(self.range.as_deref(), Some(false))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum RuleError {
    #[error("metric must be a definition name")]
    Metric,
    #[error("column must be a result column name: letters, digits or underscore")]
    Column,
    #[error("threshold must be a finite JSON number")]
    Threshold,
    #[error("range: {0}")]
    Range(WindowError),
    #[error("interval_secs must be {min} to {max}")]
    Interval { min: u32, max: u32 },
    #[error("no destination named `{0}` is configured")]
    Destination(String),
}

/// What the latest checks left on a rule.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct EvaluationState {
    pub(crate) last_evaluated_at: Option<DateTime<Utc>>,
    pub(crate) last_outcome: Option<Outcome>,
    /// What the last valid check found; unknown checks leave it as it was.
    pub(crate) last_valid_breached: Option<bool>,
    /// When the current breach began, while one is on.
    pub(crate) breached_since: Option<DateTime<Utc>>,
}

impl EvaluationState {
    /// The reason the last check was unknown, as a row stores it.
    pub(crate) fn last_reason(&self) -> Option<UnknownReason> {
        self.last_outcome.as_ref().and_then(Outcome::reason)
    }
}

/// A rule as it is stored.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AlertRule {
    pub(crate) id: Uuid,
    pub(crate) name: DefinitionName,
    pub(crate) spec: RuleSpec,
    pub(crate) enabled: bool,
    /// Bumped by every configuration write. A check carries the revision it
    /// was scheduled for, and a store refuses to record one for any other.
    pub(crate) revision: u32,
    pub(crate) state: EvaluationState,
    /// Who wrote the rule, where the surface knows: the API resolves a
    /// person, the MCP server does not.
    pub(crate) created_by: Option<Uuid>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

/// Whether a notification has been handed to a destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationStatus {
    /// Owed, and not yet sent.
    Pending,
    /// Withdrawn before it was sent: the rule was disabled.
    Cancelled,
}

impl NotificationStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        [Self::Pending, Self::Cancelled]
            .into_iter()
            .find(|status| status.as_str() == value)
    }
}

/// A notification a check decided is owed: the facts it will carry, kept
/// apart from the rule so a later edit does not rewrite what was seen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Notification {
    pub(crate) id: Uuid,
    pub(crate) rule_id: Uuid,
    pub(crate) rule_revision: u32,
    pub(crate) rule_name: DefinitionName,
    pub(crate) metric: DefinitionName,
    pub(crate) column: String,
    pub(crate) condition: Condition,
    pub(crate) value: Number,
    pub(crate) evaluated_at: DateTime<Utc>,
    pub(crate) destination: String,
    pub(crate) status: NotificationStatus,
    pub(crate) created_at: DateTime<Utc>,
}

/// One check as it is handed to a store to record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Recording {
    pub(crate) rule_id: Uuid,
    pub(crate) revision: u32,
    pub(crate) outcome: Outcome,
    pub(crate) evaluated_at: DateTime<Utc>,
}

/// What recording a check left behind.
#[derive(Debug, PartialEq)]
pub(crate) enum Recorded {
    /// The rule took the outcome; a notification is owed where one is given.
    Accepted(Box<Accepted>),
    /// The rule is gone, disabled, or at another revision: nothing written.
    Stale,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Accepted {
    pub(crate) rule: AlertRule,
    pub(crate) notification: Option<Notification>,
}

/// A write that names the revision it expects to replace.
#[derive(Debug, Clone)]
pub(crate) struct Write {
    pub(crate) name: DefinitionName,
    pub(crate) spec: RuleSpec,
    pub(crate) enabled: bool,
    pub(crate) expected_revision: Option<u32>,
    pub(crate) actor: Option<Uuid>,
}

#[async_trait]
pub(crate) trait AlertStore: Send + Sync + fmt::Debug {
    /// Creates the rule when `expected_revision` is absent, replaces it at
    /// that revision otherwise. Every success bumps the revision.
    async fn put(&self, write: Write) -> Result<AlertRule, AlertStoreError>;

    async fn get(&self, name: &DefinitionName) -> Result<Option<AlertRule>, AlertStoreError>;

    async fn get_by_id(&self, id: Uuid) -> Result<Option<AlertRule>, AlertStoreError>;

    async fn page(&self, needle: &str, page: Page) -> Result<NamePage, AlertStoreError>;

    async fn count(&self) -> Result<u64, AlertStoreError>;

    /// Every rule whose checks should be scheduled.
    async fn enabled(&self) -> Result<Vec<AlertRule>, AlertStoreError>;

    /// Turns checks on or off. Both reset the evaluation state and bump the
    /// revision; turning off withdraws every pending notification.
    async fn set_enabled(
        &self,
        name: &DefinitionName,
        expected_revision: u32,
        enabled: bool,
    ) -> Result<AlertRule, AlertStoreError>;

    /// Removes the rule and everything recorded for it.
    async fn delete(&self, name: &DefinitionName) -> Result<Option<AlertRule>, AlertStoreError>;

    /// Records one check on the rule it was scheduled for, and the
    /// notification it owes, together.
    async fn record(&self, recording: Recording) -> Result<Recorded, AlertStoreError>;

    async fn notifications(
        &self,
        rule_id: Uuid,
        page: Page,
    ) -> Result<Vec<Notification>, AlertStoreError>;
}

#[derive(Debug, Error)]
pub(crate) enum AlertStoreError {
    #[error("alert `{0}` was not found")]
    NotFound(String),
    #[error("alert `{0}` already exists")]
    NameTaken(String),
    #[error("alert `{name}` is at revision {current}, not {expected}")]
    Conflict {
        name: String,
        current: u32,
        expected: u32,
    },
    #[error(transparent)]
    Database(#[from] sea_orm::DbErr),
    #[error("an alert row holds `{0}`, which this service never wrote")]
    UnreadableRow(String),
}
