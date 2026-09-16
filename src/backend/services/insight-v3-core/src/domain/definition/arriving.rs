//! The definitions as they will read once a batch of writes lands.

use async_trait::async_trait;
use serde_json::Value;

use super::{Definition, DefinitionKind, DefinitionName, DefinitionStoreError, Lookup};

/// What a batch is about to write, over what is already stored.
///
/// One batch can hold a widget and the metric it draws, so checking the widget
/// against the stored definitions alone would refuse it for naming a metric
/// that is not there yet. The set it is checked against is the one the write
/// will leave behind.
#[derive(Debug)]
pub(crate) struct Arriving<'a> {
    stored: &'a dyn Lookup,
    batch: &'a [Definition],
}

impl<'a> Arriving<'a> {
    pub(crate) fn new(stored: &'a dyn Lookup, batch: &'a [Definition]) -> Self {
        Self { stored, batch }
    }
}

#[async_trait]
impl Lookup for Arriving<'_> {
    async fn get(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Option<Value>, DefinitionStoreError> {
        let arriving = self
            .batch
            .iter()
            .rev()
            .find(|arriving| arriving.kind == kind && arriving.name == *name);

        match arriving {
            Some(arriving) => Ok(Some(arriving.body.clone())),
            None => self.stored.get(kind, name).await,
        }
    }
}
