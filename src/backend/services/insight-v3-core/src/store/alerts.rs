//! Alert rules and notifications as `MariaDB` holds them.

#[cfg(test)]
pub(crate) mod memory;

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sea_orm::{
    ConnectionTrait as _, DatabaseConnection, DbBackend, FromQueryResult, Statement,
    TransactionTrait as _,
};
use uuid::Uuid;

use crate::domain::alerts::rule::{
    Accepted, AlertRule, AlertStore, AlertStoreError, Condition, EvaluationState, Notification,
    NotificationStatus, Operator, Recorded, Recording, RuleSpec, Write,
};
use crate::domain::alerts::{Number, Outcome, UnknownReason, last_valid_breached, transition};
use crate::domain::definition::{DefinitionName, NamePage, Page};
use crate::store::like_escaped;

const RULE_COLUMNS: &str = "id, name, metric, column_name, operator, threshold, range_code, interval_secs, destination, enabled, revision, last_evaluated_at, last_outcome, last_reason, last_value, last_valid_breached, breached_since, created_by, created_at, updated_at";

const SELECT_BY_NAME: &str = "SELECT {columns} FROM alert_rules WHERE name = ?";
/// The same read, holding the row, so two writers of one rule take turns.
const SELECT_BY_NAME_HELD: &str = "SELECT {columns} FROM alert_rules WHERE name = ? FOR UPDATE";
const SELECT_BY_ID: &str = "SELECT {columns} FROM alert_rules WHERE id = ?";
const SELECT_BY_ID_HELD: &str = "SELECT {columns} FROM alert_rules WHERE id = ? FOR UPDATE";
const SELECT_ENABLED: &str = "SELECT {columns} FROM alert_rules WHERE enabled = 1 ORDER BY name";
const PAGE_NAMES: &str = "SELECT name FROM alert_rules WHERE name LIKE ? OR metric LIKE ? ORDER BY name LIMIT ? OFFSET ?";
const COUNT_MATCHES: &str =
    "SELECT COUNT(*) AS total FROM alert_rules WHERE name LIKE ? OR metric LIKE ?";
const COUNT_ALL: &str = "SELECT COUNT(*) AS total FROM alert_rules";
const NOW: &str = "SELECT UTC_TIMESTAMP(6) AS now";

const INSERT_RULE: &str = "INSERT INTO alert_rules (id, name, metric, column_name, operator, threshold, range_code, interval_secs, destination, enabled, revision, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)";
/// A configuration write replaces the spec, bumps the revision and forgets
/// every check made under the old one.
const REPLACE_RULE: &str = "UPDATE alert_rules SET metric = ?, column_name = ?, operator = ?, threshold = ?, range_code = ?, interval_secs = ?, destination = ?, enabled = ?, revision = revision + 1, last_evaluated_at = NULL, last_outcome = NULL, last_reason = NULL, last_value = NULL, last_valid_breached = NULL, breached_since = NULL, updated_at = ? WHERE id = ? AND revision = ?";
const SET_ENABLED: &str = "UPDATE alert_rules SET enabled = ?, revision = revision + 1, last_evaluated_at = NULL, last_outcome = NULL, last_reason = NULL, last_value = NULL, last_valid_breached = NULL, breached_since = NULL, updated_at = ? WHERE id = ? AND revision = ?";
const DELETE_RULE: &str = "DELETE FROM alert_rules WHERE id = ?";
const DELETE_NOTIFICATIONS: &str = "DELETE FROM alert_notifications WHERE rule_id = ?";
const CANCEL_PENDING: &str =
    "UPDATE alert_notifications SET status = ?, updated_at = ? WHERE rule_id = ? AND status = ?";

/// A check lands only on the revision it was scheduled for.
const RECORD_CHECK: &str = "UPDATE alert_rules SET last_evaluated_at = ?, last_outcome = ?, last_reason = ?, last_value = ?, last_valid_breached = ?, breached_since = ?, updated_at = ? WHERE id = ? AND revision = ? AND enabled = 1";
const INSERT_NOTIFICATION: &str = "INSERT INTO alert_notifications (id, rule_id, rule_revision, rule_name, metric, column_name, operator, threshold, value, evaluated_at, destination, status, attempts, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)";
/// Notifications past the kept count go, oldest first.
const TRIM_NOTIFICATIONS: &str = "DELETE FROM alert_notifications WHERE rule_id = ? AND id NOT IN (SELECT id FROM (SELECT id FROM alert_notifications WHERE rule_id = ? ORDER BY created_at DESC, id DESC LIMIT ?) AS kept)";

const NOTIFICATION_COLUMNS: &str = "id, rule_id, rule_revision, rule_name, metric, column_name, operator, threshold, value, evaluated_at, destination, status, created_at";
const SELECT_NOTIFICATIONS: &str = "SELECT {columns} FROM alert_notifications WHERE rule_id = ? ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?";

fn rules_sql(template: &str) -> String {
    template.replace("{columns}", RULE_COLUMNS)
}

fn notifications_sql(template: &str) -> String {
    template.replace("{columns}", NOTIFICATION_COLUMNS)
}

fn statement(sql: String, values: impl IntoIterator<Item = sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::MySql, sql, values)
}

fn id_column(id: Uuid) -> sea_orm::Value {
    id.simple().to_string().into()
}

fn stamp(at: DateTime<Utc>) -> sea_orm::Value {
    at.naive_utc().into()
}

pub(crate) struct MariaAlerts {
    db: DatabaseConnection,
    notifications_kept_per_rule: u64,
}

impl MariaAlerts {
    pub(crate) fn new(db: DatabaseConnection, notifications_kept_per_rule: u64) -> Self {
        Self {
            db,
            notifications_kept_per_rule,
        }
    }

    async fn now<C: sea_orm::ConnectionTrait>(
        connection: &C,
    ) -> Result<DateTime<Utc>, AlertStoreError> {
        let row = NowRow::find_by_statement(Statement::from_string(DbBackend::MySql, NOW))
            .one(connection)
            .await?
            .ok_or_else(|| sea_orm::DbErr::Custom("the store has no clock".to_owned()))?;

        Ok(row.now.and_utc())
    }

    async fn one<C: sea_orm::ConnectionTrait>(
        connection: &C,
        template: &str,
        value: sea_orm::Value,
    ) -> Result<Option<AlertRule>, AlertStoreError> {
        RuleRow::find_by_statement(statement(rules_sql(template), [value]))
            .one(connection)
            .await?
            .map(RuleRow::into_rule)
            .transpose()
    }

    async fn create(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        write: &Write,
        now: DateTime<Utc>,
    ) -> Result<AlertRule, AlertStoreError> {
        let id = Uuid::now_v7();
        let spec = &write.spec;
        let inserted = transaction
            .execute_raw(statement(
                INSERT_RULE.to_owned(),
                [
                    id_column(id),
                    write.name.as_str().into(),
                    spec.metric.as_str().into(),
                    spec.column.as_str().into(),
                    spec.condition.operator.as_str().into(),
                    spec.condition.threshold.to_stored().into(),
                    spec.range.clone().into(),
                    spec.interval_secs.into(),
                    spec.destination.as_str().into(),
                    write.enabled.into(),
                    write.actor.map(|actor| actor.simple().to_string()).into(),
                    stamp(now),
                    stamp(now),
                ],
            ))
            .await;
        if let Err(error) = inserted {
            return Err(match error.sql_err() {
                Some(sea_orm::SqlErr::UniqueConstraintViolation(_)) => {
                    AlertStoreError::NameTaken(write.name.as_str().to_owned())
                }
                _ => AlertStoreError::Database(error),
            });
        }

        Self::one(transaction, SELECT_BY_ID, id_column(id))
            .await?
            .ok_or_else(|| AlertStoreError::NotFound(write.name.as_str().to_owned()))
    }

    async fn replace(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        current: &AlertRule,
        write: &Write,
        expected: u32,
        now: DateTime<Utc>,
    ) -> Result<AlertRule, AlertStoreError> {
        if current.revision != expected {
            return Err(AlertStoreError::Conflict {
                name: write.name.as_str().to_owned(),
                current: current.revision,
                expected,
            });
        }

        let spec = &write.spec;
        transaction
            .execute_raw(statement(
                REPLACE_RULE.to_owned(),
                [
                    spec.metric.as_str().into(),
                    spec.column.as_str().into(),
                    spec.condition.operator.as_str().into(),
                    spec.condition.threshold.to_stored().into(),
                    spec.range.clone().into(),
                    spec.interval_secs.into(),
                    spec.destination.as_str().into(),
                    write.enabled.into(),
                    stamp(now),
                    id_column(current.id),
                    expected.into(),
                ],
            ))
            .await?;
        if !write.enabled {
            Self::cancel_pending(transaction, current.id, now).await?;
        }

        Self::one(transaction, SELECT_BY_ID, id_column(current.id))
            .await?
            .ok_or_else(|| AlertStoreError::NotFound(write.name.as_str().to_owned()))
    }

    /// Writes the notification a check owes, and drops the oldest past the
    /// kept count.
    async fn owe_notification(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        current: &AlertRule,
        value: Number,
        evaluated_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Notification, AlertStoreError> {
        let notification = Notification {
            id: Uuid::now_v7(),
            rule_id: current.id,
            rule_revision: current.revision,
            rule_name: current.name.clone(),
            metric: current.spec.metric.clone(),
            column: current.spec.column.clone(),
            condition: current.spec.condition,
            value,
            evaluated_at,
            destination: current.spec.destination.clone(),
            status: NotificationStatus::Pending,
            created_at: now,
        };
        transaction
            .execute_raw(statement(
                INSERT_NOTIFICATION.to_owned(),
                [
                    id_column(notification.id),
                    id_column(notification.rule_id),
                    notification.rule_revision.into(),
                    notification.rule_name.as_str().into(),
                    notification.metric.as_str().into(),
                    notification.column.as_str().into(),
                    notification.condition.operator.as_str().into(),
                    notification.condition.threshold.to_stored().into(),
                    notification.value.to_stored().into(),
                    stamp(notification.evaluated_at),
                    notification.destination.as_str().into(),
                    notification.status.as_str().into(),
                    stamp(now),
                    stamp(now),
                ],
            ))
            .await?;
        transaction
            .execute_raw(statement(
                TRIM_NOTIFICATIONS.to_owned(),
                [
                    id_column(current.id),
                    id_column(current.id),
                    self.notifications_kept_per_rule.into(),
                ],
            ))
            .await?;

        Ok(notification)
    }

    async fn cancel_pending(
        transaction: &sea_orm::DatabaseTransaction,
        rule_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), AlertStoreError> {
        transaction
            .execute_raw(statement(
                CANCEL_PENDING.to_owned(),
                [
                    NotificationStatus::Cancelled.as_str().into(),
                    stamp(now),
                    id_column(rule_id),
                    NotificationStatus::Pending.as_str().into(),
                ],
            ))
            .await?;

        Ok(())
    }
}

#[async_trait]
impl AlertStore for MariaAlerts {
    async fn put(&self, write: Write) -> Result<AlertRule, AlertStoreError> {
        let transaction = self.db.begin().await?;
        let now = Self::now(&transaction).await?;

        // SAFETY: a create inserts without a held read: locking an absent name
        // takes a gap lock, and two creates then deadlock on each other's insert.
        let rule = match write.expected_revision {
            None => self.create(&transaction, &write, now).await?,
            Some(expected) => {
                let current = Self::one(
                    &transaction,
                    SELECT_BY_NAME_HELD,
                    write.name.as_str().into(),
                )
                .await?
                .ok_or_else(|| AlertStoreError::NotFound(write.name.as_str().to_owned()))?;

                self.replace(&transaction, &current, &write, expected, now)
                    .await?
            }
        };

        transaction.commit().await?;

        Ok(rule)
    }

    async fn get(&self, name: &DefinitionName) -> Result<Option<AlertRule>, AlertStoreError> {
        Self::one(&self.db, SELECT_BY_NAME, name.as_str().into()).await
    }

    async fn get_by_id(&self, id: Uuid) -> Result<Option<AlertRule>, AlertStoreError> {
        Self::one(&self.db, SELECT_BY_ID, id_column(id)).await
    }

    async fn page(&self, needle: &str, page: Page) -> Result<NamePage, AlertStoreError> {
        let pattern = format!("%{}%", like_escaped(needle));
        let rows = NameRow::find_by_statement(statement(
            PAGE_NAMES.to_owned(),
            [
                pattern.clone().into(),
                pattern.clone().into(),
                page.limit().into(),
                page.offset().into(),
            ],
        ))
        .all(&self.db)
        .await?;

        let counted = TotalRow::find_by_statement(statement(
            COUNT_MATCHES.to_owned(),
            [pattern.clone().into(), pattern.into()],
        ))
        .one(&self.db)
        .await?;

        Ok(NamePage {
            names: rows.into_iter().map(|row| row.name).collect(),
            total: counted.map_or(0, |row| u64::try_from(row.total).unwrap_or(0)),
        })
    }

    async fn count(&self) -> Result<u64, AlertStoreError> {
        let counted =
            TotalRow::find_by_statement(Statement::from_string(DbBackend::MySql, COUNT_ALL))
                .one(&self.db)
                .await?;

        Ok(counted.map_or(0, |row| u64::try_from(row.total).unwrap_or(0)))
    }

    async fn enabled(&self) -> Result<Vec<AlertRule>, AlertStoreError> {
        RuleRow::find_by_statement(Statement::from_string(
            DbBackend::MySql,
            rules_sql(SELECT_ENABLED),
        ))
        .all(&self.db)
        .await?
        .into_iter()
        .map(RuleRow::into_rule)
        .collect()
    }

    async fn set_enabled(
        &self,
        name: &DefinitionName,
        expected_revision: u32,
        enabled: bool,
    ) -> Result<AlertRule, AlertStoreError> {
        let transaction = self.db.begin().await?;
        let now = Self::now(&transaction).await?;

        let current = Self::one(&transaction, SELECT_BY_NAME_HELD, name.as_str().into())
            .await?
            .ok_or_else(|| AlertStoreError::NotFound(name.as_str().to_owned()))?;
        if current.revision != expected_revision {
            return Err(AlertStoreError::Conflict {
                name: name.as_str().to_owned(),
                current: current.revision,
                expected: expected_revision,
            });
        }

        transaction
            .execute_raw(statement(
                SET_ENABLED.to_owned(),
                [
                    enabled.into(),
                    stamp(now),
                    id_column(current.id),
                    expected_revision.into(),
                ],
            ))
            .await?;
        if !enabled {
            Self::cancel_pending(&transaction, current.id, now).await?;
        }

        let rule = Self::one(&transaction, SELECT_BY_ID, id_column(current.id))
            .await?
            .ok_or_else(|| AlertStoreError::NotFound(name.as_str().to_owned()))?;
        transaction.commit().await?;

        Ok(rule)
    }

    async fn delete(&self, name: &DefinitionName) -> Result<Option<AlertRule>, AlertStoreError> {
        let transaction = self.db.begin().await?;

        let Some(current) =
            Self::one(&transaction, SELECT_BY_NAME_HELD, name.as_str().into()).await?
        else {
            transaction.rollback().await?;
            return Ok(None);
        };

        transaction
            .execute_raw(statement(
                DELETE_NOTIFICATIONS.to_owned(),
                [id_column(current.id)],
            ))
            .await?;
        transaction
            .execute_raw(statement(DELETE_RULE.to_owned(), [id_column(current.id)]))
            .await?;
        transaction.commit().await?;

        Ok(Some(current))
    }

    async fn record(&self, recording: Recording) -> Result<Recorded, AlertStoreError> {
        let transaction = self.db.begin().await?;
        let now = Self::now(&transaction).await?;

        let Some(current) = Self::one(
            &transaction,
            SELECT_BY_ID_HELD,
            id_column(recording.rule_id),
        )
        .await?
        else {
            transaction.rollback().await?;
            return Ok(Recorded::Stale);
        };
        if current.revision != recording.revision || !current.enabled {
            transaction.rollback().await?;
            return Ok(Recorded::Stale);
        }

        let previous = current.state.last_valid_breached;
        let next = last_valid_breached(previous, &recording.outcome);
        let owed =
            transition(previous, &recording.outcome) == crate::domain::alerts::Transition::Notify;
        let breached_since = match (next, current.state.breached_since) {
            (Some(true), Some(since)) if previous == Some(true) => Some(since),
            (Some(true), _) => Some(recording.evaluated_at),
            (Some(false), _) => None,
            (None, since) => since,
        };

        let outcome = recording.outcome;
        transaction
            .execute_raw(statement(
                RECORD_CHECK.to_owned(),
                [
                    stamp(recording.evaluated_at),
                    outcome.as_str().into(),
                    outcome.reason().map(UnknownReason::as_str).into(),
                    outcome.value().map(Number::to_stored).into(),
                    next.into(),
                    breached_since.map(|since| since.naive_utc()).into(),
                    stamp(now),
                    id_column(current.id),
                    recording.revision.into(),
                ],
            ))
            .await?;

        let notification = match (owed, outcome.value()) {
            (true, Some(value)) => Some(
                self.owe_notification(&transaction, &current, value, recording.evaluated_at, now)
                    .await?,
            ),
            _ => None,
        };

        let rule = Self::one(&transaction, SELECT_BY_ID, id_column(current.id))
            .await?
            .ok_or_else(|| AlertStoreError::NotFound(current.name.as_str().to_owned()))?;
        transaction.commit().await?;

        Ok(Recorded::Accepted(Box::new(Accepted {
            rule,
            notification,
        })))
    }

    async fn notifications(
        &self,
        rule_id: Uuid,
        page: Page,
    ) -> Result<Vec<Notification>, AlertStoreError> {
        NotificationRow::find_by_statement(statement(
            notifications_sql(SELECT_NOTIFICATIONS),
            [
                id_column(rule_id),
                page.limit().into(),
                page.offset().into(),
            ],
        ))
        .all(&self.db)
        .await?
        .into_iter()
        .map(NotificationRow::into_notification)
        .collect()
    }
}

impl fmt::Debug for MariaAlerts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MariaAlerts")
            .field(
                "notifications_kept_per_rule",
                &self.notifications_kept_per_rule,
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, FromQueryResult)]
struct RuleRow {
    id: String,
    name: String,
    metric: String,
    column_name: String,
    operator: String,
    threshold: String,
    range_code: Option<String>,
    interval_secs: u32,
    destination: String,
    enabled: bool,
    revision: u32,
    last_evaluated_at: Option<NaiveDateTime>,
    last_outcome: Option<String>,
    last_reason: Option<String>,
    last_value: Option<String>,
    last_valid_breached: Option<bool>,
    breached_since: Option<NaiveDateTime>,
    created_by: Option<String>,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

fn unreadable(value: &str) -> AlertStoreError {
    AlertStoreError::UnreadableRow(value.to_owned())
}

fn parse_id(value: &str) -> Result<Uuid, AlertStoreError> {
    Uuid::parse_str(value).map_err(|_| unreadable(value))
}

fn parse_condition(operator: &str, threshold: &str) -> Result<Condition, AlertStoreError> {
    Ok(Condition {
        operator: Operator::parse(operator).ok_or_else(|| unreadable(operator))?,
        threshold: Number::from_stored(threshold).ok_or_else(|| unreadable(threshold))?,
    })
}

impl RuleRow {
    fn into_rule(self) -> Result<AlertRule, AlertStoreError> {
        let condition = parse_condition(&self.operator, &self.threshold)?;
        let last_outcome = match (self.last_outcome.as_deref(), self.last_reason.as_deref()) {
            (None, _) => None,
            (Some("unknown"), Some(reason)) => Some(Outcome::Unknown(
                UnknownReason::parse(reason).ok_or_else(|| unreadable(reason))?,
            )),
            (Some(word @ ("breach" | "no_breach")), _) => {
                let stored = self.last_value.as_deref().ok_or_else(|| unreadable(word))?;
                Some(Outcome::Valid {
                    value: Number::from_stored(stored).ok_or_else(|| unreadable(stored))?,
                    breached: word == "breach",
                })
            }
            (Some(word), _) => return Err(unreadable(word)),
        };

        Ok(AlertRule {
            id: parse_id(&self.id)?,
            name: DefinitionName::parse(&self.name).map_err(|_| unreadable(&self.name))?,
            spec: RuleSpec {
                metric: DefinitionName::parse(&self.metric)
                    .map_err(|_| unreadable(&self.metric))?,
                column: self.column_name,
                condition,
                range: self.range_code,
                interval_secs: self.interval_secs,
                destination: self.destination,
            },
            enabled: self.enabled,
            revision: self.revision,
            state: EvaluationState {
                last_evaluated_at: self.last_evaluated_at.map(|at| at.and_utc()),
                last_outcome,
                last_valid_breached: self.last_valid_breached,
                breached_since: self.breached_since.map(|at| at.and_utc()),
            },
            created_by: self.created_by.as_deref().map(parse_id).transpose()?,
            created_at: self.created_at.and_utc(),
            updated_at: self.updated_at.and_utc(),
        })
    }
}

#[derive(Debug, FromQueryResult)]
struct NotificationRow {
    id: String,
    rule_id: String,
    rule_revision: u32,
    rule_name: String,
    metric: String,
    column_name: String,
    operator: String,
    threshold: String,
    value: String,
    evaluated_at: NaiveDateTime,
    destination: String,
    status: String,
    created_at: NaiveDateTime,
}

impl NotificationRow {
    fn into_notification(self) -> Result<Notification, AlertStoreError> {
        Ok(Notification {
            id: parse_id(&self.id)?,
            rule_id: parse_id(&self.rule_id)?,
            rule_revision: self.rule_revision,
            rule_name: DefinitionName::parse(&self.rule_name)
                .map_err(|_| unreadable(&self.rule_name))?,
            metric: DefinitionName::parse(&self.metric).map_err(|_| unreadable(&self.metric))?,
            column: self.column_name,
            condition: parse_condition(&self.operator, &self.threshold)?,
            value: Number::from_stored(&self.value).ok_or_else(|| unreadable(&self.value))?,
            evaluated_at: self.evaluated_at.and_utc(),
            destination: self.destination,
            status: NotificationStatus::parse(&self.status)
                .ok_or_else(|| unreadable(&self.status))?,
            created_at: self.created_at.and_utc(),
        })
    }
}

#[derive(Debug, FromQueryResult)]
struct NameRow {
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct TotalRow {
    total: i64,
}

#[derive(Debug, FromQueryResult)]
struct NowRow {
    now: NaiveDateTime,
}

#[cfg(test)]
mod tests;
