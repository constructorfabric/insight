//! Sending an owed notification: what the message says, what a provider
//! answers, and what that answer means for the row.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use super::rule::Notification;

/// The most a provider is asked to carry. Every provider here allows more;
/// the bound is on what an alert has any business saying.
const MAX_MESSAGE_CHARS: usize = 1500;

/// What a notification says, the same for every provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) text: String,
}

impl Message {
    /// The approved summary: rule, metric and column, the value seen, the
    /// condition it met, and when. Nothing else the metric produced.
    pub(crate) fn of(notification: &Notification) -> Self {
        let text = format!(
            "Insight alert `{rule}`: {metric}.{column} = {value} ({condition}) at {at}",
            rule = notification.rule_name.as_str(),
            metric = notification.metric.as_str(),
            column = notification.column,
            value = notification.value,
            condition = notification.condition,
            at = stamp(notification.evaluated_at),
        );

        Self {
            text: text.chars().take(MAX_MESSAGE_CHARS).collect(),
        }
    }
}

fn stamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// What a provider said when it accepted the message: the identity it
/// gave it, kept so an operator can find the message where it landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Receipt(pub(crate) String);

/// Why a send did not confirm, and whether trying again can help.
#[derive(Debug, Error)]
pub(crate) enum SendError {
    /// The provider said no and will keep saying no: a bad token, a
    /// missing channel, a malformed request.
    #[error("the provider rejected the message: {0}")]
    Rejected(String),
    /// The provider asked for a pause, or did not answer, or answered in a
    /// way that leaves the outcome unknown. The message may have landed.
    #[error("the provider did not confirm the message: {0}")]
    Unconfirmed(String),
}

impl SendError {
    pub(crate) fn is_retryable(&self) -> bool {
        match self {
            Self::Rejected(_) => false,
            Self::Unconfirmed(_) => true,
        }
    }

    /// What a row and a reader see: the class of failure and the provider's
    /// own words, never a credential.
    pub(crate) fn summary(&self) -> String {
        self.to_string()
    }
}

#[async_trait]
pub(crate) trait Provider: Send + Sync + fmt::Debug {
    async fn send(&self, message: &Message) -> Result<Receipt, SendError>;
}

/// What one delivery attempt left on the notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Attempted {
    /// Accepted, with the provider's receipt.
    Sent(Receipt),
    /// Not confirmed; another attempt follows.
    Retry(String),
    /// Rejected, or the last attempt failed: no more attempts.
    Failed(String),
}

/// What a send outcome means on the attempt it happened on.
pub(crate) fn attempted(
    result: Result<Receipt, SendError>,
    attempt: u32,
    max_attempts: u32,
) -> Attempted {
    match result {
        Ok(receipt) => Attempted::Sent(receipt),
        Err(error) if error.is_retryable() && attempt < max_attempts => {
            Attempted::Retry(error.summary())
        }
        Err(error) => Attempted::Failed(error.summary()),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use uuid::Uuid;

    use super::*;
    use crate::domain::alerts::Number;
    use crate::domain::alerts::rule::{Condition, NotificationStatus, Operator};
    use crate::domain::definition::DefinitionName;

    fn notification() -> Notification {
        Notification {
            id: Uuid::nil(),
            rule_id: Uuid::nil(),
            rule_revision: 3,
            rule_name: DefinitionName::parse("too-many-prs")
                .unwrap_or_else(|error| panic!("{error}")),
            metric: DefinitionName::parse("prs-open").unwrap_or_else(|error| panic!("{error}")),
            column: "total".to_owned(),
            condition: Condition {
                operator: Operator::Gt,
                threshold: Number::Int(50),
            },
            value: Number::Int(63),
            evaluated_at: Utc
                .with_ymd_and_hms(2026, 9, 29, 8, 15, 0)
                .single()
                .unwrap_or_default(),
            destination: "ops".to_owned(),
            status: NotificationStatus::Pending,
            attempts: 0,
            last_error: None,
            provider_receipt: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn the_message_carries_the_rule_the_value_the_condition_and_the_time_and_nothing_else() {
        let message = Message::of(&notification());

        assert_eq!(
            message.text,
            "Insight alert `too-many-prs`: prs-open.total = 63 (> 50) at 2026-09-29T08:15:00Z"
        );
    }

    #[test]
    fn an_unconfirmed_send_is_retried_until_the_last_attempt_and_a_rejection_never() {
        let cases = [
            (
                "accepted",
                Ok(Receipt("m1".into())),
                1,
                Attempted::Sent(Receipt("m1".into())),
            ),
            (
                "unconfirmed early",
                Err(SendError::Unconfirmed("timed out".into())),
                1,
                Attempted::Retry("the provider did not confirm the message: timed out".into()),
            ),
            (
                "unconfirmed on the last attempt",
                Err(SendError::Unconfirmed("timed out".into())),
                5,
                Attempted::Failed("the provider did not confirm the message: timed out".into()),
            ),
            (
                "rejected early",
                Err(SendError::Rejected("401".into())),
                1,
                Attempted::Failed("the provider rejected the message: 401".into()),
            ),
        ];

        for (case, result, attempt, expected) in cases {
            assert_eq!(
                attempted(result, attempt, 5),
                expected,
                "should decide: {case}"
            );
        }
    }
}

/// Where owed notifications are queued to be sent, and how the queue is
/// told about one.
#[async_trait]
pub(crate) trait Deliveries: Send + Sync + fmt::Debug {
    /// Queues a send for this notification. Queuing the same one twice is
    /// one send.
    async fn enqueue(
        &self,
        notification_id: uuid::Uuid,
    ) -> Result<(), super::schedule::ScheduleError>;
}

/// The send a queue hands a worker: which notification, on which attempt
/// of how many.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DeliveryJob {
    pub(crate) notification_id: uuid::Uuid,
}

/// How one send ended.
#[derive(Debug)]
pub(crate) enum Delivered {
    /// Nothing to send: the notification is gone or no longer pending.
    Skipped,
    /// Recorded on the notification.
    Attempted(Attempted),
}

#[derive(Debug)]
pub(crate) struct Deliverer<'a> {
    store: &'a dyn super::rule::AlertStore,
    providers: &'a std::collections::BTreeMap<String, std::sync::Arc<dyn Provider>>,
}

impl<'a> Deliverer<'a> {
    pub(crate) fn new(
        store: &'a dyn super::rule::AlertStore,
        providers: &'a std::collections::BTreeMap<String, std::sync::Arc<dyn Provider>>,
    ) -> Self {
        Self { store, providers }
    }

    /// Sends the notification once and records what happened.
    ///
    /// A destination that is no longer configured is a rejection: the
    /// operator removed it, and no attempt can change that.
    pub(crate) async fn deliver(
        &self,
        job: DeliveryJob,
        attempt: u32,
        max_attempts: u32,
    ) -> Result<Delivered, super::rule::AlertStoreError> {
        let Some(notification) = self.store.notification(job.notification_id).await? else {
            return Ok(Delivered::Skipped);
        };
        if notification.status != super::rule::NotificationStatus::Pending {
            return Ok(Delivered::Skipped);
        }

        let sent = match self.providers.get(&notification.destination) {
            Some(provider) => provider.send(&Message::of(&notification)).await,
            None => Err(SendError::Rejected(format!(
                "destination `{}` is no longer configured",
                notification.destination
            ))),
        };
        let outcome = attempted(sent, attempt, max_attempts);

        match self.store.record_attempt(notification.id, &outcome).await? {
            Some(_) => Ok(Delivered::Attempted(outcome)),
            None => Ok(Delivered::Skipped),
        }
    }
}
