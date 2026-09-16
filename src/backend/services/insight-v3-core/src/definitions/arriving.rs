//! The definition store as it will be once a batch of writes lands.

use async_trait::async_trait;
use serde_json::Value;

use super::{
    Change, DefinitionKind, DefinitionName, DefinitionStoreError, Definitions, NamePage, Page,
};

/// What a batch is about to write, over what is already stored.
///
/// One batch can hold a widget and the metric it draws, so checking the widget
/// against the stored definitions alone would refuse it for naming a metric
/// that is not there yet. The set it is checked against is the one the write
/// will leave behind.
#[derive(Debug)]
pub(crate) struct Arriving<'a> {
    stored: &'a dyn Definitions,
    batch: &'a [(DefinitionKind, DefinitionName, Value)],
}

impl<'a> Arriving<'a> {
    pub(crate) fn new(
        stored: &'a dyn Definitions,
        batch: &'a [(DefinitionKind, DefinitionName, Value)],
    ) -> Self {
        Self { stored, batch }
    }
}

#[async_trait]
impl Definitions for Arriving<'_> {
    async fn get(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Option<Value>, DefinitionStoreError> {
        let arriving = self
            .batch
            .iter()
            .rev()
            .find(|(held, held_name, _)| *held == kind && held_name == name);

        match arriving {
            Some((_, _, body)) => Ok(Some(body.clone())),
            None => self.stored.get(kind, name).await,
        }
    }

    async fn put(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &Value,
    ) -> Result<(), DefinitionStoreError> {
        self.stored.put(kind, name, body).await
    }

    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError> {
        self.stored.list(kind).await
    }

    async fn page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<NamePage, DefinitionStoreError> {
        self.stored.page(kind, needle, page).await
    }

    async fn delete(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<bool, DefinitionStoreError> {
        self.stored.delete(kind, name).await
    }

    async fn apply(&self, changes: &[Change]) -> Result<(), DefinitionStoreError> {
        self.stored.apply(changes).await
    }
}
