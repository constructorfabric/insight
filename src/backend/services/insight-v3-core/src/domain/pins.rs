use std::fmt;

use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::definition::{DefinitionName, DefinitionStoreError};

pub(crate) const MAX_PINS: usize = 20;

#[derive(Debug, Error)]
pub(crate) enum PinError {
    #[error("no dashboard is named `{0}`")]
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
    async fn pins_of(&self, person: Uuid) -> Result<Vec<String>, PinError>;

    async fn pin(&self, person: Uuid, dashboard: &DefinitionName) -> Result<(), PinError>;

    async fn unpin(&self, person: Uuid, dashboard: &DefinitionName) -> Result<(), PinError>;
}
