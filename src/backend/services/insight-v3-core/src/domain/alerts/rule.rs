//! A rule as it is written, checked and stored.

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
use crate::domain::definition::{DefinitionName, Page};
use crate::domain::metric_run::Columns;
use crate::domain::query::time_window::{WindowError, WindowRequest};

/// The bounds every installation gets unless it sets its own.
pub(crate) const DEFAULT_MIN_INTERVAL_SECS: u32 = 60;
pub(crate) const DEFAULT_MAX_INTERVAL_SECS: u32 = 7 * 24 * 3600;
pub(crate) const DEFAULT_MAX_RULES: u64 = 200;
pub(crate) const DEFAULT_EVALUATION_CONCURRENCY: usize = 4;
pub(crate) const DEFAULT_EVALUATION_LOCK_SECS: u64 = 60;
pub(crate) const DEFAULT_NOTIFICATIONS_KEPT_PER_RULE: u64 = 200;

/// The longest an alert's name may be. It is shown, never used as a key.
const MAX_NAME_CHARS: usize = 200;
/// The most columns a rule may carry beside the ones its condition reads.
pub(crate) const MAX_CONTEXT_COLUMNS: usize = 5;
const MAX_COLUMN_CHARS: usize = 128;

/// What an administrator calls an alert: free text, shown in every message
/// and listing. The id is the handle, so two alerts may share a name.
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

    /// A name as a row recorded it.
    pub(crate) fn from_row(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
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
#[derive(Debug, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleDraft {
    /// What the alert is called, as people read it: up to 200 characters.
    pub(crate) name: String,
    /// The stored metric to check.
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

/// What a rule watches, checked against the installation's bounds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuleSpec {
    pub(crate) name: AlertName,
    pub(crate) metric: DefinitionName,
    pub(crate) condition: Condition,
    /// Columns the witness keeps beside the condition's, in this order.
    pub(crate) context_columns: Vec<String>,
    pub(crate) range: Option<String>,
    pub(crate) interval_secs: u32,
    pub(crate) destination: String,
}

impl RuleSpec {
    /// A draft as a rule, or the first thing wrong with it. `columns` are
    /// the metric's result columns and what each holds, which the condition
    /// and the context columns are checked against.
    pub(crate) fn parse(
        draft: &RuleDraft,
        limits: Limits,
        destinations: &Destinations,
        columns: &Columns,
    ) -> Result<Self, RuleError> {
        let name = AlertName::parse(&draft.name)?;
        let metric = DefinitionName::parse(&draft.metric).map_err(|_| RuleError::Metric)?;
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

    /// The window a check runs the metric over: the rule's range, never
    /// bucketed, because one row is what a check reads.
    pub(crate) fn window(&self) -> Result<WindowRequest, WindowError> {
        WindowRequest::parse(self.range.as_deref(), Some(false))
    }
}

/// The columns a rule asks to carry, each one the metric answers and none
/// twice. A column the condition reads too is fine: the witness keeps it
/// once. A group-by column is refused: the witness always shows it, and the
/// message writes it as the row's group.
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
    #[error("metric must be a definition name")]
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
    /// The provider confirmed it.
    Sent,
    /// The provider rejected it, or every attempt went unconfirmed.
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

/// A notification a check decided is owed: the facts it will carry, kept
/// apart from the rule so a later edit does not rewrite what was seen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Notification {
    pub(crate) id: Uuid,
    pub(crate) rule_id: Uuid,
    pub(crate) rule_revision: u32,
    pub(crate) rule_name: AlertName,
    pub(crate) metric: DefinitionName,
    pub(crate) condition: Condition,
    /// The columns the rule asked to carry when this was owed, so the message
    /// can tell them from the ones the condition read.
    pub(crate) context_columns: Vec<String>,
    /// The rows the check saw that owed this.
    pub(crate) witness: Witness,
    pub(crate) evaluated_at: DateTime<Utc>,
    pub(crate) destination: String,
    pub(crate) status: NotificationStatus,
    /// How many sends have been tried.
    pub(crate) attempts: u32,
    /// What the last unconfirmed or rejected send said, in the provider's
    /// words, never a credential.
    pub(crate) last_error: Option<String>,
    /// The identity the provider gave the message, once it confirmed it.
    pub(crate) provider_receipt: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
}

/// One check as it is handed to a store to record.
#[derive(Debug, Clone)]
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

/// What a write puts on a rule.
#[derive(Debug, Clone)]
pub(crate) struct Write {
    pub(crate) spec: RuleSpec,
    pub(crate) enabled: bool,
    pub(crate) actor: Option<Uuid>,
}

/// One rule as a listing shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlertSummary {
    pub(crate) id: Uuid,
    pub(crate) name: AlertName,
    pub(crate) metric: DefinitionName,
    pub(crate) enabled: bool,
}

/// One page of a listing, and how many there are in all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlertPage {
    pub(crate) alerts: Vec<AlertSummary>,
    pub(crate) total: u64,
}

#[async_trait]
pub(crate) trait AlertStore: Send + Sync + fmt::Debug {
    /// Creates a rule at revision 1 under a fresh id, unless the installation
    /// already holds `max_rules`. The count and the insert are one serialised
    /// step, so two creates at the bound cannot both land.
    async fn create(&self, write: Write, max_rules: u64) -> Result<AlertRule, AlertStoreError>;

    /// Replaces the rule at the revision it is expected to be at, bumping it
    /// and forgetting what its checks found.
    async fn replace(
        &self,
        id: Uuid,
        expected_revision: u32,
        write: Write,
    ) -> Result<AlertRule, AlertStoreError>;

    async fn get(&self, id: Uuid) -> Result<Option<AlertRule>, AlertStoreError>;

    /// Rules whose name or metric holds `needle`, by name.
    async fn page(&self, needle: &str, page: Page) -> Result<AlertPage, AlertStoreError>;

    /// Every rule whose checks should be scheduled.
    async fn enabled(&self) -> Result<Vec<AlertRule>, AlertStoreError>;

    /// Turns checks on or off. Both reset the evaluation state and bump the
    /// revision; turning off withdraws every pending notification.
    async fn set_enabled(
        &self,
        id: Uuid,
        expected_revision: u32,
        enabled: bool,
    ) -> Result<AlertRule, AlertStoreError>;

    /// Removes the rule and everything recorded for it.
    async fn delete(&self, id: Uuid) -> Result<Option<AlertRule>, AlertStoreError>;

    /// Records one check on the rule it was scheduled for, and the
    /// notification it owes, together.
    async fn record(&self, recording: Recording) -> Result<Recorded, AlertStoreError>;

    async fn notifications(
        &self,
        rule_id: Uuid,
        page: Page,
    ) -> Result<Vec<Notification>, AlertStoreError>;

    async fn notification(&self, id: Uuid) -> Result<Option<Notification>, AlertStoreError>;

    /// Every notification still owed, oldest first, for the schedule to
    /// pick up again after a restart.
    async fn pending_notifications(&self) -> Result<Vec<Notification>, AlertStoreError>;

    /// Records one send on a pending notification. Anything not pending is
    /// left as it is and answered as none.
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
