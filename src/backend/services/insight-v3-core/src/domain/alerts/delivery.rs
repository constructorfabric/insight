//! Sending an owed notification: what the message says, what a provider
//! answers, and what that answer means for the row.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use super::Number;
use super::condition::{Quantifier, Witness};
use super::rule::Notification;

/// The most a provider is asked to carry. Every provider here allows more;
/// the bound is on what an alert has any business saying.
const MAX_MESSAGE_CHARS: usize = 1500;
/// The most one line of a message carries, so a long condition or wide rows
/// never push the check time out.
const MAX_LINE_CHARS: usize = 280;
/// The most of one cell a message shows.
const MAX_CELL_CHARS: usize = 40;

/// What a notification says, the same for every provider: as markdown for
/// Discord and Zulip, and as HTML for Telegram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) markdown: String,
    pub(crate) html: String,
}

impl Message {
    /// The approved summary: the rule, the condition on which metric, how
    /// many rows met it, the rows that did, and when. Nothing else the
    /// metric produced.
    ///
    /// INVARIANT: every part stays on its own line, because control
    /// characters in what the warehouse answered become spaces. The rows
    /// line is the only text the warehouse writes, and both rich forms carry
    /// it as code, where no provider reads a mention or a link.
    pub(crate) fn of(notification: &Notification) -> Self {
        let condition = &notification.condition;
        let witness = &notification.witness;

        let heading = line(&format!(
            "Insight alert: {}",
            notification.rule_name.as_str()
        ));
        let met = line(&format!("{}: {condition}", notification.metric.as_str()));
        let counted = match condition.rows {
            Quantifier::One => None,
            Quantifier::Any | Quantifier::All | Quantifier::None | Quantifier::Count { .. } => {
                Some(line(&matched(witness)))
            }
        };
        let rows =
            (!witness.rows.is_empty()).then(|| line(&rows_seen(witness, &condition.columns())));
        let checked = format!("Checked {}.", stamp(notification.evaluated_at));
        let parts = Parts {
            heading: &heading,
            met: &met,
            counted: counted.as_deref(),
            rows: rows.as_deref(),
            checked: &checked,
        };

        Self {
            markdown: capped(&parts.joined(str::to_owned, |rows| {
                format!("`{}`", rows.replace('`', "'"))
            })),
            // INVARIANT: never cut after escaping, which could split an
            // entity or a tag and have the whole message refused. Each line
            // is already bounded, and Telegram counts the text it shows.
            html: parts.joined(escaped, |rows| format!("<code>{}</code>", escaped(rows))),
        }
    }
}

/// The lines of one message, before any provider's form is applied.
struct Parts<'a> {
    heading: &'a str,
    met: &'a str,
    counted: Option<&'a str>,
    rows: Option<&'a str>,
    checked: &'a str,
}

impl Parts<'_> {
    /// The lines joined, each written by `plain` except the warehouse's rows
    /// line, written by `rows`.
    fn joined(&self, plain: impl Fn(&str) -> String, rows: impl Fn(&str) -> String) -> String {
        [
            Some(plain(self.heading)),
            Some(plain(self.met)),
            self.counted.map(&plain),
            self.rows.map(rows),
            Some(plain(self.checked)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
    }
}

fn capped(text: &str) -> String {
    text.chars().take(MAX_MESSAGE_CHARS).collect()
}

/// Text as Telegram's HTML reads it literally.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `3 of 12 rows match`, or `1 of 1 row matches`.
fn matched(witness: &Witness) -> String {
    let rows = if witness.answered == 1 {
        "row matches"
    } else {
        "rows match"
    };

    format!("{} of {} {rows}", witness.matched, witness.answered)
}

/// One line of a message: control characters as spaces, cut to the line
/// bound.
fn line(text: &str) -> String {
    cut(&single_line(text), MAX_LINE_CHARS)
}

fn single_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn cut(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }

    let mut kept: String = text.chars().take(chars.saturating_sub(1)).collect();
    kept.push('…');
    kept
}

/// The witness rows on one line: a row's group values, then each column
/// the condition reads with its value, as `core: open 12 · infra: open 15`.
fn rows_seen(witness: &Witness, read: &[&str]) -> String {
    let rows: Vec<String> = witness
        .rows
        .iter()
        .map(|row| {
            let mut groups = Vec::new();
            let mut values = Vec::new();
            for (column, cell) in witness.columns.iter().zip(row) {
                if read.contains(&column.as_str()) {
                    values.push(format!("{column} {}", cell_text(cell)));
                } else {
                    groups.push(cell_text(cell));
                }
            }

            if groups.is_empty() {
                values.join(", ")
            } else {
                format!("{}: {}", groups.join(" "), values.join(", "))
            }
        })
        .collect();

    rows.join(" · ")
}

fn cell_text(cell: &serde_json::Value) -> String {
    let text = match cell {
        serde_json::Value::Null => "empty".to_owned(),
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(_) => {
            Number::parse(cell).map_or_else(|| cell.to_string(), Number::readable)
        }
        serde_json::Value::Bool(_) | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            cell.to_string()
        }
    };

    cut(&single_line(&text), MAX_CELL_CHARS)
}

fn stamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M UTC").to_string()
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

    use serde_json::{Value, json};

    use super::*;
    use crate::domain::alerts::condition::Condition;
    use crate::domain::alerts::rule::{AlertName, NotificationStatus};
    use crate::domain::definition::DefinitionName;
    use crate::domain::query::metric_query::ColumnKind;

    fn notification(condition: &Value, witness: Witness) -> Notification {
        let columns = [
            ("total".to_owned(), ColumnKind::Number),
            ("open".to_owned(), ColumnKind::Number),
            ("team".to_owned(), ColumnKind::Text),
        ];

        Notification {
            id: Uuid::nil(),
            rule_id: Uuid::nil(),
            rule_revision: 3,
            rule_name: AlertName::parse("Too many open PRs")
                .unwrap_or_else(|error| panic!("{error}")),
            metric: DefinitionName::parse("prs-open").unwrap_or_else(|error| panic!("{error}")),
            condition: Condition::parse(condition, &columns)
                .unwrap_or_else(|error| panic!("{error}")),
            witness,
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

    fn witness(matched: u64, answered: u64, columns: &[&str], rows: Vec<Vec<Value>>) -> Witness {
        Witness {
            matched,
            answered,
            columns: columns.iter().map(|&column| column.to_owned()).collect(),
            rows,
        }
    }

    #[test]
    fn a_one_row_message_carries_the_rule_the_condition_the_value_and_the_time() {
        let message = Message::of(&notification(
            &json!({ "rows": "one", "where": { "column": "total", "op": ">", "value": 50 } }),
            witness(1, 1, &["total"], vec![vec![json!(63)]]),
        ));

        assert_eq!(
            message.markdown,
            "Insight alert: Too many open PRs\nprs-open: total above 50\n`total 63`\nChecked 2026-09-29 08:15 UTC."
        );
    }

    #[test]
    fn a_many_row_message_counts_the_matches_and_shows_each_by_its_group() {
        let message = Message::of(&notification(
            &json!({ "rows": "any", "where": { "all": [
                { "column": "open", "op": ">", "value": 10 },
                { "column": "team", "op": "!=", "value": "data" },
            ] } }),
            witness(
                2,
                12,
                &["repo", "open", "team"],
                vec![
                    vec![json!("api"), json!(12), json!("core")],
                    vec![json!("web"), json!(15.25), Value::Null],
                ],
            ),
        ));

        assert_eq!(
            message.markdown,
            "Insight alert: Too many open PRs\nprs-open: any row: open above 10 and team is not data\n2 of 12 rows match\n`api: open 12, team core · web: open 15.25, team empty`\nChecked 2026-09-29 08:15 UTC."
        );
    }

    #[test]
    fn a_breach_with_no_matching_rows_shows_the_count_and_no_row_line() {
        let message = Message::of(&notification(
            &json!({ "rows": "none", "where": { "column": "open", "op": ">", "value": 10 } }),
            witness(0, 4, &["open"], Vec::new()),
        ));

        assert_eq!(
            message.markdown,
            "Insight alert: Too many open PRs\nprs-open: no row: open above 10\n0 of 4 rows match\nChecked 2026-09-29 08:15 UTC."
        );
    }

    #[test]
    fn one_answered_row_is_counted_in_the_singular() {
        let message = Message::of(&notification(
            &json!({ "rows": "any", "where": { "column": "open", "op": ">", "value": 10 } }),
            witness(1, 1, &["open"], vec![vec![json!(12)]]),
        ));

        assert!(
            message.markdown.contains("\n1 of 1 row matches\n"),
            "{}",
            message.markdown
        );
    }

    #[test]
    fn a_long_message_keeps_its_check_time_and_cuts_each_line() {
        let long = "x".repeat(400);
        let rows = (0..5)
            .map(|_| vec![json!(long.clone()), json!(11)])
            .collect();
        let message = Message::of(&notification(
            &json!({ "rows": "any", "where": { "column": "open", "op": ">", "value": 10 } }),
            witness(5, 5, &["team", "open"], rows),
        ));

        assert!(
            message.markdown.ends_with("Checked 2026-09-29 08:15 UTC."),
            "{}",
            message.markdown
        );
        assert!(message.markdown.chars().count() <= MAX_MESSAGE_CHARS);
        assert!(
            message
                .markdown
                .lines()
                .all(|line| line.trim_matches('`').chars().count() <= MAX_LINE_CHARS),
            "{}",
            message.markdown
        );
    }

    #[test]
    fn warehouse_text_stays_on_its_line_and_reaches_markdown_inside_a_code_span() {
        let message = Message::of(&notification(
            &json!({ "rows": "any", "where": { "column": "open", "op": ">", "value": 10 } }),
            witness(
                1,
                1,
                &["team", "open"],
                vec![vec![
                    json!("@**all**\nInsight alert: [x](http://example.com) `y`"),
                    json!(12),
                ]],
            ),
        ));

        assert_eq!(message.markdown.lines().count(), 5, "{}", message.markdown);
        let rows_line = message.markdown.lines().nth(3).unwrap_or_default();
        assert!(
            rows_line.starts_with('`') && rows_line.ends_with('`'),
            "the rows line is one code span: {rows_line}"
        );
        assert_eq!(rows_line.matches('`').count(), 2, "{rows_line}");

        let html_rows = message.html.lines().nth(3).unwrap_or_default();
        assert!(
            html_rows.starts_with("<code>") && html_rows.ends_with("</code>"),
            "the rows line is one code element: {html_rows}"
        );
        assert_eq!(
            html_rows.matches('<').count(),
            2,
            "markup in the cell is escaped: {html_rows}"
        );
    }

    #[test]
    fn a_number_is_written_as_a_person_reads_it() {
        let cases = [
            (Number::Int(63), "63"),
            (Number::Int(-9_007_199_254_740_993), "-9007199254740993"),
            (Number::Float(0.1 + 0.2), "0.3"),
            (Number::Float(83.912_345_6), "83.9123"),
            (Number::Float(2.5), "2.5"),
            (Number::Float(1_234_567.8), "1234568"),
            (Number::Float(0.000_012_345_67), "0.0000123457"),
            (Number::Float(-0.0), "0"),
            (Number::Float(0.0), "0"),
        ];

        for (number, expected) in cases {
            assert_eq!(number.readable(), expected, "should write: {number:?}");
        }
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
    /// The attempt counted is the row's own, never below the job's: a job
    /// that repair created anew starts counting again, the notification does
    /// not, so the cap holds per notification. A destination that is no
    /// longer configured is a rejection: the operator removed it, and no
    /// attempt can change that.
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
        let attempt = attempt.max(notification.attempts.saturating_add(1));
        let outcome = attempted(sent, attempt, max_attempts);

        let recorded = self.store.record_attempt(notification.id, &outcome).await?;
        if recorded.is_none() {
            landed_after_withdrawal(&notification, &outcome);
            return Ok(Delivered::Skipped);
        }

        Ok(Delivered::Attempted(outcome))
    }
}

/// The one outcome a row no longer pending cannot show: the message was
/// confirmed while the notification was being withdrawn, so it exists at the
/// destination and the row says cancelled. The receipt goes to the log, the
/// only place left for it.
fn landed_after_withdrawal(notification: &Notification, outcome: &Attempted) {
    let Attempted::Sent(receipt) = outcome else {
        return;
    };

    tracing::warn!(
        notification_id = %notification.id,
        rule_id = %notification.rule_id,
        receipt = receipt.0,
        "a notification was confirmed after it was withdrawn; the row does not carry its receipt"
    );
}
