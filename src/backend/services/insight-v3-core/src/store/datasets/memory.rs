//! The dataset store the tests use.
//!
//! It holds rows in a map under one lock, which is the serialization the real
//! store gets from holding the row, and it decides with the same rule the real
//! store does.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::domain::datasets::{
    Attempt, Dataset, DatasetStoreError, Datasets, Held, OperationToken, Refused, Taking,
    lease_until, taking,
};
use crate::domain::definition::DefinitionName;
use crate::domain::kinds::dataset::lifecycle::{DatasetState, Operation};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(crate) struct MemoryDatasets {
    stored: Mutex<BTreeMap<String, Dataset>>,
    /// The clock the leases are read against, so a test can let one lapse
    /// without waiting for it.
    now: Mutex<DateTime<Utc>>,
}

impl MemoryDatasets {
    pub(crate) fn at(now: DateTime<Utc>) -> Self {
        Self {
            stored: Mutex::new(BTreeMap::new()),
            now: Mutex::new(now),
        }
    }

    /// Moves the clock the leases are read against.
    pub(crate) fn set_now(&self, now: DateTime<Utc>) {
        *self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = now;
    }

    fn now(&self) -> DateTime<Utc> {
        *self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Dataset>> {
        self.stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl Datasets for MemoryDatasets {
    async fn get(&self, name: &DefinitionName) -> Result<Option<Dataset>, DatasetStoreError> {
        Ok(self.lock().get(name.as_str()).cloned())
    }

    async fn list(&self) -> Result<Vec<String>, DatasetStoreError> {
        Ok(self.lock().keys().cloned().collect())
    }

    async fn take_operation(
        &self,
        name: &DefinitionName,
        operation: Operation,
        declaration: &Value,
    ) -> Result<Attempt, DatasetStoreError> {
        let now = self.now();
        let mut stored = self.lock();

        let token = OperationToken::mint();
        let held = Held {
            operation,
            token: token.clone(),
            until: lease_until(now),
        };

        let state = match taking(stored.get(name.as_str()), operation, now) {
            Taking::Refuse(refusal) => return Err(refusal.into()),
            Taking::Gone => return Err(Refused::Gone.into()),
            Taking::Claim => {
                stored.insert(
                    name.as_str().to_owned(),
                    Dataset {
                        name: name.clone(),
                        declaration: declaration.clone(),
                        state: DatasetState::Claimed,
                        physical_table: None,
                        held: Some(held),
                    },
                );

                DatasetState::Claimed
            }
            Taking::Take(state) => {
                let Some(dataset) = stored.get_mut(name.as_str()) else {
                    return Err(Refused::Gone.into());
                };
                dataset.state = state;
                dataset.held = Some(held);

                state
            }
        };

        Ok(Attempt { token, state })
    }
}
