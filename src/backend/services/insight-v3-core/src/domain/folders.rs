use std::fmt;

use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::access::{Owner, PersonId};
use crate::domain::dashboards::{DashboardCatalogue, sight};
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionStoreError, Definitions};
use crate::domain::pins::Pins;
use crate::domain::sharing::Sharing;
use crate::domain::tags::Tags;

const MAX_NAME_CHARS: usize = 64;

/// Per person: nobody fills, or sees the count of, another's folders.
pub(crate) const MAX_FOLDERS: usize = 200;

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
    pub(crate) owner: Owner,
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
    pub(crate) fn parse(kind: DefinitionKind, value: &str) -> Result<Self, FolderError> {
        if kind != DefinitionKind::Dashboard {
            return Err(FolderError::NotFiled(kind.plural()));
        }
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
    #[error("no dashboard has the id `{0}`")]
    DashboardNotFound(String),
    #[error("{0}")]
    WrongShelf(&'static str),
    #[error("{0} are not filed in folders")]
    NotFiled(&'static str),
    #[error("at most {MAX_FOLDERS} folders can be yours; delete one first")]
    TooMany,
    #[error(transparent)]
    Store(#[from] DefinitionStoreError),
}

impl FolderError {
    pub(crate) fn is_about_the_caller(&self) -> bool {
        !matches!(self, Self::Store(_))
    }
}

impl From<sea_orm::DbErr> for FolderError {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::Store(DefinitionStoreError::Database(error))
    }
}

#[async_trait]
pub(crate) trait Folders: Send + Sync + fmt::Debug {
    /// The caller's folders and the ownerless ones, each counting the
    /// dashboards in it the caller sees, and how many they see in none.
    async fn list_folders(&self, caller: PersonId) -> Result<FolderList, FolderError>;

    async fn create_folder(&self, owner: PersonId, name: FolderName)
    -> Result<Folder, FolderError>;

    /// Renames the caller's folder or an ownerless one; anyone else's is not
    /// found.
    async fn rename_folder(
        &self,
        caller: PersonId,
        id: FolderId,
        name: FolderName,
    ) -> Result<Folder, FolderError>;

    /// Removes the caller's folder or an ownerless one, reporting whether
    /// there was one.
    async fn delete_folder(&self, caller: PersonId, id: FolderId) -> Result<bool, FolderError>;

    /// Whose folder this is, or none when there is no such folder.
    async fn folder_owner(&self, id: FolderId) -> Result<Option<Owner>, FolderError>;

    /// The folder the caller files the dashboard in: their own placement of
    /// an owned one, the shared folder of an ownerless one.
    async fn folder_of(
        &self,
        dashboard: &DefinitionId,
        caller: PersonId,
    ) -> Result<Option<Folder>, FolderError>;

    /// Files the dashboard among `folders_of`: a person's placement, or an ownerless
    /// dashboard's own folder.
    async fn file(
        &self,
        dashboard: &DefinitionId,
        folders_of: Owner,
        folder: Option<FolderId>,
    ) -> Result<(), FolderError>;
}

/// Files a dashboard the caller sees in one of the folders its ownership
/// allows: their own for one they own or were shared, an ownerless one for
/// an ownerless dashboard.
pub(crate) async fn file(
    store: &dyn DefinitionStore,
    caller: PersonId,
    dashboard: &DefinitionId,
    folder: Option<FolderId>,
) -> Result<(), FolderError> {
    let seen = sight(store, dashboard, caller)
        .await?
        .ok_or_else(|| FolderError::DashboardNotFound(dashboard.as_str().to_owned()))?;
    let folders_of = seen.access.shelf(caller);

    if let Some(id) = folder {
        let owner = store.folder_owner(id).await?;
        fits(folders_of, caller, id, owner)?;
    }

    store.file(dashboard, folders_of, folder).await
}

/// Whether a folder may hold a dashboard filed among `folders_of`. Someone else's
/// folder is not found, as it is everywhere else.
fn fits(
    folders_of: Owner,
    caller: PersonId,
    id: FolderId,
    owner: Option<Owner>,
) -> Result<(), FolderError> {
    let Some(owner) = owner else {
        return Err(FolderError::FolderNotFound(id));
    };

    match (folders_of, owner) {
        (_, Owner::Person(person)) if person != caller => Err(FolderError::FolderNotFound(id)),
        (Owner::Person(_), Owner::Person(_)) | (Owner::Unowned, Owner::Unowned) => Ok(()),
        (Owner::Person(_), Owner::Unowned) => Err(FolderError::WrongShelf(
            "a dashboard with an owner is filed in your own folders",
        )),
        (Owner::Unowned, Owner::Person(_)) => Err(FolderError::WrongShelf(
            "an ownerless dashboard is filed in an ownerless folder",
        )),
    }
}

/// INVARIANT: one store holds them all, because a dashboard duplicate carries
/// its owner, placement and tags inside the definitions' own transaction.
pub(crate) trait DefinitionStore:
    Definitions + Folders + Tags + Pins + DashboardCatalogue + Sharing
{
}

impl<T: Definitions + Folders + Tags + Pins + DashboardCatalogue + Sharing> DefinitionStore for T {}

#[cfg(test)]
mod tests;
