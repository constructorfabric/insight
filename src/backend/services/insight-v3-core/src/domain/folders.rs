use std::fmt;

use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::definition::{
    DefinitionName, DefinitionStoreError, Definitions, NamePage, Page,
};

const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FolderId(Uuid);

impl FolderId {
    pub(crate) fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub(crate) fn parse(value: &str) -> Result<Self, FolderError> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| FolderError::Id)
    }
}

impl fmt::Display for FolderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FolderName(String);

impl FolderName {
    pub(crate) fn parse(value: &str) -> Result<Self, FolderError> {
        let trimmed = value.trim();
        let chars = trimmed.chars().count();
        if chars == 0 || chars > MAX_NAME_CHARS {
            return Err(FolderError::Name);
        }

        Ok(Self(trimmed.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Folder {
    pub(crate) id: FolderId,
    pub(crate) name: FolderName,
}

#[derive(Debug, Clone)]
pub(crate) struct FolderSummary {
    pub(crate) folder: Folder,
    pub(crate) dashboards: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct FolderList {
    pub(crate) folders: Vec<FolderSummary>,
    pub(crate) unfiled: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FolderFilter {
    Unfiled,
    In(FolderId),
}

impl FolderFilter {
    pub(crate) fn parse(value: &str) -> Result<Self, FolderError> {
        if value == "unfiled" {
            return Ok(Self::Unfiled);
        }

        FolderId::parse(value).map(Self::In)
    }
}

#[derive(Debug, Error)]
pub(crate) enum FolderError {
    #[error("folder names are 1 to 64 characters, not counting the spaces around them")]
    Name,
    #[error("folder ids are UUIDs")]
    Id,
    #[error("no folder has the id `{0}`")]
    FolderNotFound(FolderId),
    #[error("no dashboard is named `{0}`")]
    DashboardNotFound(String),
    #[error("a folder named `{0}` already exists")]
    NameTaken(String),
    #[error(transparent)]
    Store(#[from] DefinitionStoreError),
}

impl From<sea_orm::DbErr> for FolderError {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::Store(DefinitionStoreError::Database(error))
    }
}

#[async_trait]
pub(crate) trait Folders: Send + Sync + fmt::Debug {
    async fn list_folders(&self) -> Result<FolderList, FolderError>;

    async fn create_folder(&self, name: FolderName) -> Result<Folder, FolderError>;

    async fn rename_folder(&self, id: FolderId, name: FolderName) -> Result<Folder, FolderError>;

    async fn delete_folder(&self, id: FolderId) -> Result<bool, FolderError>;

    async fn folder_of(&self, dashboard: &DefinitionName) -> Result<Option<Folder>, FolderError>;

    async fn file(
        &self,
        dashboard: &DefinitionName,
        folder: Option<FolderId>,
    ) -> Result<(), FolderError>;

    async fn page_filed(
        &self,
        needle: &str,
        page: Page,
        filter: FolderFilter,
    ) -> Result<NamePage, FolderError>;
}

/// INVARIANT: one store holds both, because a dashboard rename carries its
/// folder inside the definitions' own transaction.
pub(crate) trait DefinitionStore: Definitions + Folders {}

impl<T: Definitions + Folders> DefinitionStore for T {}

#[cfg(test)]
mod tests;
