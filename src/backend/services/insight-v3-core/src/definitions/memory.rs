//! The definition store the tests use.
//!
//! It keeps definitions in a map, so a test that stores one and reads it back
//! asserts on behaviour rather than on a database's wire protocol. The handler
//! tests used to prime a queue of `ClickHouse` responses in call order, which
//! meant every change to what a request reads rewrote them.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::{
    Change, DefinitionKind, DefinitionName, DefinitionStoreError, Definitions, NamePage, Page,
};

#[derive(Debug, Default)]
pub(crate) struct MemoryDefinitions {
    stored: Mutex<BTreeMap<(&'static str, String), serde_json::Value>>,
    /// Set to fail every write, for the cases about a store that is down.
    failing: bool,
}

impl MemoryDefinitions {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn key(kind: DefinitionKind, name: &DefinitionName) -> (&'static str, String) {
        (kind.table(), name.as_str().to_owned())
    }

    fn write(&self, kind: DefinitionKind, name: &DefinitionName, body: &serde_json::Value) {
        let mut stored = self.lock();
        stored.insert(Self::key(kind, name), body.clone());
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<(&'static str, String), serde_json::Value>> {
        self.stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn refuse() -> DefinitionStoreError {
        DefinitionStoreError::Database(sea_orm::DbErr::Custom("store is down".to_owned()))
    }
}

#[async_trait]
impl Definitions for MemoryDefinitions {
    async fn put(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError> {
        if self.failing {
            return Err(Self::refuse());
        }
        self.write(kind, name, body);

        Ok(())
    }

    async fn get(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError> {
        Ok(self.lock().get(&Self::key(kind, name)).cloned())
    }

    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError> {
        Ok(self
            .lock()
            .keys()
            .filter(|(table, _)| *table == kind.table())
            .map(|(_, name)| name.clone())
            .collect())
    }

    async fn page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<NamePage, DefinitionStoreError> {
        let needle = needle.to_lowercase();
        let matched: Vec<String> = self
            .lock()
            .iter()
            .filter(|((table, _), _)| *table == kind.table())
            .filter(|((_, name), body)| {
                name.to_lowercase().contains(&needle)
                    || body.to_string().to_lowercase().contains(&needle)
            })
            .map(|((_, name), _)| name.clone())
            .collect();

        let total = matched.len() as u64;
        let names = matched
            .into_iter()
            .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
            .collect();

        Ok(NamePage { names, total })
    }

    async fn delete(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<bool, DefinitionStoreError> {
        Ok(self.lock().remove(&Self::key(kind, name)).is_some())
    }

    async fn apply(&self, changes: &[Change]) -> Result<(), DefinitionStoreError> {
        if self.failing {
            return Err(Self::refuse());
        }
        // All or nothing, as the transaction is: the map is held for the whole
        // batch, so a reader never sees half of it.
        let mut stored = self.lock();
        for change in changes {
            match change {
                Change::Put(kind, name, body) => {
                    stored.insert(Self::key(*kind, name), body.clone());
                }
                Change::Delete(kind, name) => {
                    stored.remove(&Self::key(*kind, name));
                }
            }
        }

        Ok(())
    }
}
