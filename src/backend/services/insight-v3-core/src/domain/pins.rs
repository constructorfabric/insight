use std::fmt;

use async_trait::async_trait;
use thiserror::Error;

use crate::domain::access::Caller;
use crate::domain::dashboards::sight;
use crate::domain::definition::{DefinitionId, DefinitionStoreError, Listed};
use crate::domain::folders::DefinitionStore;

pub(crate) const MAX_PINS: usize = 20;

#[derive(Debug, Error)]
pub(crate) enum PinError {
    #[error("no dashboard has the id `{0}`")]
    DashboardNotFound(String),
    #[error("at most {MAX_PINS} dashboards can be pinned; unpin one first")]
    TooMany,
    #[error(transparent)]
    Store(#[from] DefinitionStoreError),
}

impl From<sea_orm::DbErr> for PinError {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::Store(DefinitionStoreError::Database(error))
    }
}

#[async_trait]
pub(crate) trait Pins: Send + Sync + fmt::Debug {
    async fn pins_of(&self, caller: Caller) -> Result<Vec<Listed>, PinError>;

    async fn pin(&self, caller: Caller, dashboard: &DefinitionId) -> Result<(), PinError>;

    async fn unpin(&self, caller: Caller, dashboard: &DefinitionId) -> Result<bool, PinError>;
}

pub(crate) async fn pin(
    store: &dyn DefinitionStore,
    caller: Caller,
    dashboard: &DefinitionId,
) -> Result<(), PinError> {
    if sight(store, dashboard, caller).await?.is_none() {
        return Err(PinError::DashboardNotFound(dashboard.as_str().to_owned()));
    }

    store.pin(caller, dashboard).await
}

pub(crate) async fn unpin(
    store: &dyn DefinitionStore,
    caller: Caller,
    dashboard: &DefinitionId,
) -> Result<(), PinError> {
    if store.unpin(caller, dashboard).await? {
        return Ok(());
    }
    if sight(store, dashboard, caller).await?.is_none() {
        return Err(PinError::DashboardNotFound(dashboard.as_str().to_owned()));
    }

    Ok(())
}
