use async_trait::async_trait;
use serde_json::Value;

use super::{Definition, DefinitionId, DefinitionKind, DefinitionStoreError, Lookup};

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
        id: &DefinitionId,
    ) -> Result<Option<Value>, DefinitionStoreError> {
        let arriving = self
            .batch
            .iter()
            .rev()
            .find(|arriving| arriving.kind == kind && arriving.id == *id);

        match arriving {
            Some(arriving) => Ok(Some(arriving.body.clone())),
            None => self.stored.get(kind, id).await,
        }
    }
}
