use std::fmt;

use async_trait::async_trait;
use thiserror::Error;

use crate::domain::access::PersonId;
use crate::domain::dashboards::sight;
use crate::domain::definition::{DefinitionId, DefinitionStoreError, Listed};
use crate::domain::folders::DefinitionStore;

/// Counting only the pins the person still sees.
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
    /// The person's pins of dashboards they still see, oldest first.
    async fn pins_of(&self, person: PersonId) -> Result<Vec<Listed>, PinError>;

    async fn pin(&self, person: PersonId, dashboard: &DefinitionId) -> Result<(), PinError>;

    /// Removes the pin, reporting whether there was one.
    async fn unpin(&self, person: PersonId, dashboard: &DefinitionId) -> Result<bool, PinError>;
}

/// Pins a dashboard the caller sees.
pub(crate) async fn pin(
    store: &dyn DefinitionStore,
    caller: PersonId,
    dashboard: &DefinitionId,
) -> Result<(), PinError> {
    let seen = sight(store, dashboard, caller).await?;
    if !seen.is_some_and(|seen| seen.access.may_open()) {
        return Err(PinError::DashboardNotFound(dashboard.as_str().to_owned()));
    }

    store.pin(caller, dashboard).await
}

/// Unpins a dashboard. A pin the caller holds goes whether or not they still
/// see the dashboard; without one, a dashboard they cannot see is not found.
pub(crate) async fn unpin(
    store: &dyn DefinitionStore,
    caller: PersonId,
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
