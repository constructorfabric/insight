//! The alert store the handler tests use: the same rules as `MariaDB`
//! enforces, kept in a map.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::domain::alerts::rule::{
    Accepted, AlertRule, AlertStore, AlertStoreError, EvaluationState, Notification,
    NotificationStatus, Recorded, Recording, Write,
};
use crate::domain::alerts::{Transition, last_valid_breached, transition};
use crate::domain::definition::{DefinitionName, NamePage, Page};

#[derive(Debug, Default)]
pub(crate) struct MemoryAlerts {
    rules: Mutex<BTreeMap<String, AlertRule>>,
    notifications: Mutex<Vec<Notification>>,
    failing: bool,
}

impl MemoryAlerts {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A store that refuses everything, for the cases about one that is down.
    pub(crate) fn refusing() -> Self {
        Self {
            failing: true,
            ..Self::default()
        }
    }

    fn rules(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, AlertRule>> {
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
}

#[async_trait]
impl AlertStore for MemoryAlerts {
    async fn put(&self, write: Write) -> Result<AlertRule, AlertStoreError> {
        self.check()?;
        let now = Utc::now();
        let mut rules = self.rules();
        let name = write.name.as_str().to_owned();

        let rule = match (rules.get(&name), write.expected_revision) {
            (None, None) => AlertRule {
                id: Uuid::now_v7(),
                name: write.name.clone(),
                spec: write.spec,
                enabled: write.enabled,
                revision: 1,
                state: EvaluationState::default(),
                created_by: write.actor,
                created_at: now,
                updated_at: now,
            },
            (None, Some(_)) => return Err(AlertStoreError::NotFound(name)),
            (Some(_), None) => return Err(AlertStoreError::NameTaken(name)),
            (Some(current), Some(expected)) => {
                if current.revision != expected {
                    return Err(AlertStoreError::Conflict {
                        name,
                        current: current.revision,
                        expected,
                    });
                }
                if !write.enabled {
                    self.cancel_pending(current.id);
                }

                AlertRule {
                    spec: write.spec,
                    enabled: write.enabled,
                    revision: current.revision + 1,
                    state: EvaluationState::default(),
                    updated_at: now,
                    ..current.clone()
                }
            }
        };
        rules.insert(name, rule.clone());

        Ok(rule)
    }

    async fn get(&self, name: &DefinitionName) -> Result<Option<AlertRule>, AlertStoreError> {
        self.check()?;

        Ok(self.rules().get(name.as_str()).cloned())
    }

    async fn get_by_id(&self, id: Uuid) -> Result<Option<AlertRule>, AlertStoreError> {
        self.check()?;

        Ok(self.rules().values().find(|rule| rule.id == id).cloned())
    }

    async fn page(&self, needle: &str, page: Page) -> Result<NamePage, AlertStoreError> {
        self.check()?;
        let matching: Vec<String> = self
            .rules()
            .values()
            .filter(|rule| {
                rule.name.as_str().contains(needle) || rule.spec.metric.as_str().contains(needle)
            })
            .map(|rule| rule.name.as_str().to_owned())
            .collect();
        let total = matching.len() as u64;
        let names = matching
            .into_iter()
            .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
            .collect();

        Ok(NamePage { names, total })
    }

    async fn count(&self) -> Result<u64, AlertStoreError> {
        self.check()?;

        Ok(self.rules().len() as u64)
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
        name: &DefinitionName,
        expected_revision: u32,
        enabled: bool,
    ) -> Result<AlertRule, AlertStoreError> {
        self.check()?;
        let mut rules = self.rules();
        let current = rules
            .get_mut(name.as_str())
            .ok_or_else(|| AlertStoreError::NotFound(name.as_str().to_owned()))?;
        if current.revision != expected_revision {
            return Err(AlertStoreError::Conflict {
                name: name.as_str().to_owned(),
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

    async fn delete(&self, name: &DefinitionName) -> Result<Option<AlertRule>, AlertStoreError> {
        self.check()?;
        let removed = self.rules().remove(name.as_str());
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
        let Some(current) = rules.values_mut().find(|rule| rule.id == recording.rule_id) else {
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
                rule_name: current.name.clone(),
                metric: current.spec.metric.clone(),
                column: current.spec.column.clone(),
                condition: current.spec.condition,
                value,
                evaluated_at: recording.evaluated_at,
                destination: current.spec.destination.clone(),
                status: NotificationStatus::Pending,
                created_at: now,
            }),
            _ => None,
        };
        let rule = current.clone();
        drop(rules);
        if let Some(notification) = &notification {
            self.held().push(notification.clone());
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
}
