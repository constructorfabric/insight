//! The alert store the handler tests use: the same rules as `MariaDB`
//! enforces, kept in a map.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use super::attempt_columns;
use crate::domain::alerts::delivery::Attempted;
use crate::domain::alerts::rule::{
    Accepted, AlertPage, AlertRule, AlertStore, AlertStoreError, AlertSummary,
    DEFAULT_NOTIFICATIONS_KEPT_PER_RULE, EvaluationState, Notification, NotificationStatus,
    Recorded, Recording, Write,
};
use crate::domain::alerts::{Transition, last_valid_breached, transition};
use crate::domain::definition::Page;

#[derive(Debug)]
pub(crate) struct MemoryAlerts {
    rules: Mutex<BTreeMap<Uuid, AlertRule>>,
    notifications: Mutex<Vec<Notification>>,
    notifications_kept_per_rule: u64,
    failing: bool,
}

impl Default for MemoryAlerts {
    fn default() -> Self {
        Self::keeping(DEFAULT_NOTIFICATIONS_KEPT_PER_RULE)
    }
}

impl MemoryAlerts {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A store that keeps this many settled notifications per rule.
    pub(crate) fn keeping(notifications_kept_per_rule: u64) -> Self {
        Self {
            rules: Mutex::default(),
            notifications: Mutex::default(),
            notifications_kept_per_rule,
            failing: false,
        }
    }

    /// A store that refuses everything, for the cases about one that is down.
    pub(crate) fn refusing() -> Self {
        Self {
            failing: true,
            ..Self::default()
        }
    }

    fn rules(&self) -> std::sync::MutexGuard<'_, BTreeMap<Uuid, AlertRule>> {
        self.rules
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Vec<Notification>> {
        self.notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn check(&self) -> Result<(), AlertStoreError> {
        if self.failing {
            return Err(AlertStoreError::Database(sea_orm::DbErr::Custom(
                "store is down".to_owned(),
            )));
        }

        Ok(())
    }

    fn cancel_pending(&self, rule_id: Uuid) {
        for notification in self.held().iter_mut() {
            if notification.rule_id == rule_id && notification.status == NotificationStatus::Pending
            {
                notification.status = NotificationStatus::Cancelled;
            }
        }
    }

    /// Drops the rule's settled notifications past the newest kept count, as
    /// the SQL store's trim does; one still owed stays.
    fn trim(&self, rule_id: Uuid) {
        let mut held = self.held();
        let mut newest_first: Vec<(chrono::DateTime<Utc>, Uuid)> = held
            .iter()
            .filter(|notification| notification.rule_id == rule_id)
            .map(|notification| (notification.created_at, notification.id))
            .collect();
        newest_first.sort_by(|left, right| right.cmp(left));

        let kept: Vec<Uuid> = newest_first
            .into_iter()
            .take(usize::try_from(self.notifications_kept_per_rule).unwrap_or(usize::MAX))
            .map(|(_, id)| id)
            .collect();
        held.retain(|notification| {
            notification.rule_id != rule_id
                || notification.status == NotificationStatus::Pending
                || kept.contains(&notification.id)
        });
    }
}

#[async_trait]
impl AlertStore for MemoryAlerts {
    async fn create(&self, write: Write, max_rules: u64) -> Result<AlertRule, AlertStoreError> {
        self.check()?;
        let now = Utc::now();
        let rule = AlertRule {
            id: Uuid::now_v7(),
            spec: write.spec,
            enabled: write.enabled,
            revision: 1,
            state: EvaluationState::default(),
            created_by: write.actor,
            created_at: now,
            updated_at: now,
        };

        let mut rules = self.rules();
        if rules.len() as u64 >= max_rules {
            return Err(AlertStoreError::TooMany(max_rules));
        }
        rules.insert(rule.id, rule.clone());

        Ok(rule)
    }

    async fn replace(
        &self,
        id: Uuid,
        expected_revision: u32,
        write: Write,
    ) -> Result<AlertRule, AlertStoreError> {
        self.check()?;
        let mut rules = self.rules();
        let current = rules.get_mut(&id).ok_or(AlertStoreError::NotFound(id))?;
        if current.revision != expected_revision {
            return Err(AlertStoreError::Conflict {
                id,
                current: current.revision,
                expected: expected_revision,
            });
        }

        current.spec = write.spec;
        current.enabled = write.enabled;
        current.revision += 1;
        current.state = EvaluationState::default();
        current.updated_at = Utc::now();
        let rule = current.clone();
        drop(rules);
        if !rule.enabled {
            self.cancel_pending(rule.id);
        }

        Ok(rule)
    }

    async fn get(&self, id: Uuid) -> Result<Option<AlertRule>, AlertStoreError> {
        self.check()?;

        Ok(self.rules().get(&id).cloned())
    }

    async fn page(&self, needle: &str, page: Page) -> Result<AlertPage, AlertStoreError> {
        self.check()?;
        let needle = needle.to_lowercase();
        let mut matching: Vec<AlertSummary> = self
            .rules()
            .values()
            .filter(|rule| {
                rule.spec.name.as_str().to_lowercase().contains(&needle)
                    || rule.spec.metric.as_str().to_lowercase().contains(&needle)
            })
            .map(|rule| AlertSummary {
                id: rule.id,
                name: rule.spec.name.clone(),
                metric: rule.spec.metric.clone(),
                enabled: rule.enabled,
            })
            .collect();
        matching.sort_by(|left, right| {
            left.name
                .as_str()
                .cmp(right.name.as_str())
                .then(left.id.cmp(&right.id))
        });
        let total = matching.len() as u64;
        let alerts = matching
            .into_iter()
            .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
            .collect();

        Ok(AlertPage { alerts, total })
    }

    async fn enabled(&self) -> Result<Vec<AlertRule>, AlertStoreError> {
        self.check()?;

        Ok(self
            .rules()
            .values()
            .filter(|rule| rule.enabled)
            .cloned()
            .collect())
    }

    async fn set_enabled(
        &self,
        id: Uuid,
        expected_revision: u32,
        enabled: bool,
    ) -> Result<AlertRule, AlertStoreError> {
        self.check()?;
        let mut rules = self.rules();
        let current = rules.get_mut(&id).ok_or(AlertStoreError::NotFound(id))?;
        if current.revision != expected_revision {
            return Err(AlertStoreError::Conflict {
                id,
                current: current.revision,
                expected: expected_revision,
            });
        }

        current.enabled = enabled;
        current.revision += 1;
        current.state = EvaluationState::default();
        current.updated_at = Utc::now();
        let rule = current.clone();
        drop(rules);
        if !enabled {
            self.cancel_pending(rule.id);
        }

        Ok(rule)
    }

    async fn delete(&self, id: Uuid) -> Result<Option<AlertRule>, AlertStoreError> {
        self.check()?;
        let removed = self.rules().remove(&id);
        if let Some(rule) = &removed {
            self.held()
                .retain(|notification| notification.rule_id != rule.id);
        }

        Ok(removed)
    }

    async fn record(&self, recording: Recording) -> Result<Recorded, AlertStoreError> {
        self.check()?;
        let now = Utc::now();
        let mut rules = self.rules();
        let Some(current) = rules.get_mut(&recording.rule_id) else {
            return Ok(Recorded::Stale);
        };
        if current.revision != recording.revision || !current.enabled {
            return Ok(Recorded::Stale);
        }

        let previous = current.state.last_valid_breached;
        let next = last_valid_breached(previous, &recording.outcome);
        current.state.breached_since = match (next, current.state.breached_since) {
            (Some(true), Some(since)) if previous == Some(true) => Some(since),
            (Some(true), _) => Some(recording.evaluated_at),
            (Some(false), _) => None,
            (None, since) => since,
        };
        current.state.last_valid_breached = next;
        current.state.last_evaluated_at = Some(recording.evaluated_at);
        current.state.last_outcome = Some(recording.outcome);

        let owed = transition(previous, &recording.outcome) == Transition::Notify;
        let notification = match (owed, recording.outcome.value()) {
            (true, Some(value)) => Some(Notification {
                id: Uuid::now_v7(),
                rule_id: current.id,
                rule_revision: current.revision,
                rule_name: current.spec.name.clone(),
                metric: current.spec.metric.clone(),
                column: current.spec.column.clone(),
                condition: current.spec.condition,
                value,
                evaluated_at: recording.evaluated_at,
                destination: current.spec.destination.clone(),
                status: NotificationStatus::Pending,
                attempts: 0,
                last_error: None,
                provider_receipt: None,
                created_at: now,
            }),
            _ => None,
        };
        let rule = current.clone();
        drop(rules);
        if let Some(notification) = &notification {
            self.held().push(notification.clone());
            self.trim(rule.id);
        }

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
        self.check()?;

        Ok(self
            .held()
            .iter()
            .rev()
            .filter(|notification| notification.rule_id == rule_id)
            .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
            .cloned()
            .collect())
    }

    async fn notification(&self, id: Uuid) -> Result<Option<Notification>, AlertStoreError> {
        self.check()?;

        Ok(self
            .held()
            .iter()
            .find(|notification| notification.id == id)
            .cloned())
    }

    async fn pending_notifications(&self) -> Result<Vec<Notification>, AlertStoreError> {
        self.check()?;

        Ok(self
            .held()
            .iter()
            .filter(|notification| notification.status == NotificationStatus::Pending)
            .cloned()
            .collect())
    }

    async fn record_attempt(
        &self,
        id: Uuid,
        attempted: &Attempted,
    ) -> Result<Option<Notification>, AlertStoreError> {
        self.check()?;
        let mut held = self.held();
        let Some(current) = held.iter_mut().find(|notification| {
            notification.id == id && notification.status == NotificationStatus::Pending
        }) else {
            return Ok(None);
        };

        let columns = attempt_columns(attempted);
        current.attempts += 1;
        current.status = columns.status;
        current.last_error = columns.last_error;
        current.provider_receipt = columns.provider_receipt;

        Ok(Some(current.clone()))
    }
}
