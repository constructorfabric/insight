use std::fmt;

use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::access::{
    Caller, Entry, FOLDER_NOT_EDITOR, FOLDER_NOT_OWNER, FolderAccess, FolderStanding, Owner,
    PersonId, Refused, Shelf, folder_access, shelved,
};
use crate::domain::dashboards::{DashboardCatalogue, sight};
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionStoreError, Definitions};
use crate::domain::pins::Pins;
use crate::domain::sharing::Grants;
use crate::domain::tags::Tags;

const MAX_NAME_CHARS: usize = 64;

/// Per owner: a folder counts against whoever owns it, whoever it is
/// shared with.
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

/// A folder as the store holds it, with what decides one caller's access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeldFolder {
    pub(crate) folder: Folder,
    pub(crate) standing: FolderStanding,
}

/// A folder the caller sees, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeenFolder {
    pub(crate) folder: Folder,
    pub(crate) access: FolderAccess,
}

impl SeenFolder {
    pub(crate) fn of(caller: Caller, held: HeldFolder) -> Option<Self> {
        let access = folder_access(caller, held.standing)?;

        Some(Self {
            folder: held.folder,
            access,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FolderSummary {
    pub(crate) seen: SeenFolder,
    /// The dashboards in it the caller sees.
    pub(crate) dashboards: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct FolderList {
    pub(crate) folders: Vec<FolderSummary>,
    /// The dashboards the caller sees in none of the folders they see.
    pub(crate) unfiled: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FolderFilter {
    /// In none of the folders the caller sees.
    Unfiled,
    /// In this folder, which the caller sees.
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
    #[error("{}", .0.said())]
    WrongShelf(Shelf),
    #[error("{0} are not filed in folders")]
    NotFiled(&'static str),
    #[error("at most {MAX_FOLDERS} folders can be yours; delete one first")]
    TooMany,
    #[error("{FOLDER_NOT_EDITOR}")]
    NotEditor,
    #[error("{FOLDER_NOT_OWNER}")]
    NotOwner,
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

impl From<Refused> for FolderError {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::NotEditor | Refused::Ownerless => Self::NotEditor,
            Refused::NotOwner => Self::NotOwner,
        }
    }
}

#[async_trait]
pub(crate) trait Folders: Send + Sync + fmt::Debug {
    /// The folders `caller` sees, each counting the dashboards in it they
    /// see, and how many they see in none of them.
    async fn list_folders(&self, caller: Caller) -> Result<FolderList, FolderError>;

    /// Makes a folder of `owner`'s, refused once they own [`MAX_FOLDERS`].
    async fn create_folder(&self, owner: PersonId, name: FolderName)
    -> Result<Folder, FolderError>;

    /// The folder and what decides `caller`'s access to it, or none when no
    /// folder has the id.
    async fn held_folder(
        &self,
        id: FolderId,
        caller: Caller,
    ) -> Result<Option<HeldFolder>, DefinitionStoreError>;

    /// Renames the folder, reporting whether there was one.
    async fn rename_folder(&self, id: FolderId, name: &FolderName) -> Result<bool, FolderError>;

    /// Removes the folder with its entries and grants, never a dashboard,
    /// reporting whether there was one.
    async fn delete_folder(&self, id: FolderId) -> Result<bool, FolderError>;

    /// The folders `caller` sees that hold the dashboard, by name.
    async fn folders_holding(
        &self,
        dashboard: &DefinitionId,
        caller: Caller,
    ) -> Result<Vec<Folder>, FolderError>;

    /// Puts the dashboard in the folder as `caller` places it.
    ///
    /// INVARIANT: lands only on a dashboard `caller` sees as the row is
    /// written, so an entry cannot rely on a grant withdrawn meanwhile. An
    /// entry already there keeps carrying once it does.
    async fn add_entry(
        &self,
        folder: FolderId,
        dashboard: &DefinitionId,
        caller: Caller,
        entry: Entry,
    ) -> Result<(), FolderError>;

    /// Takes the dashboard out of the folder, reporting whether it was in.
    async fn remove_entry(
        &self,
        folder: FolderId,
        dashboard: &DefinitionId,
    ) -> Result<bool, FolderError>;
}

/// A folder the caller sees, or not found - which they are never told
/// apart.
pub(crate) async fn seen_folder(
    store: &dyn DefinitionStore,
    caller: Caller,
    id: FolderId,
) -> Result<SeenFolder, FolderError> {
    store
        .held_folder(id, caller)
        .await?
        .and_then(|held| SeenFolder::of(caller, held))
        .ok_or(FolderError::FolderNotFound(id))
}

pub(crate) async fn rename(
    store: &dyn DefinitionStore,
    caller: Caller,
    id: FolderId,
    name: FolderName,
) -> Result<SeenFolder, FolderError> {
    let seen = seen_folder(store, caller, id).await?;
    seen.access.arrange()?;

    if !store.rename_folder(id, &name).await? {
        return Err(FolderError::FolderNotFound(id));
    }

    Ok(SeenFolder {
        folder: Folder {
            name,
            ..seen.folder
        },
        access: seen.access,
    })
}

pub(crate) async fn delete(
    store: &dyn DefinitionStore,
    caller: Caller,
    id: FolderId,
) -> Result<(), FolderError> {
    let seen = seen_folder(store, caller, id).await?;
    seen.access.delete()?;

    if !store.delete_folder(id).await? {
        return Err(FolderError::FolderNotFound(id));
    }

    Ok(())
}

/// Puts a dashboard the caller sees in a folder they may arrange: carrying
/// where they own it or hold an editor grant on it, filing otherwise.
pub(crate) async fn add(
    store: &dyn DefinitionStore,
    caller: Caller,
    folder: FolderId,
    dashboard: &DefinitionId,
) -> Result<Entry, FolderError> {
    let seen_folder = seen_folder(store, caller, folder).await?;
    seen_folder.access.arrange()?;
    let seen = sight(store, dashboard, caller)
        .await?
        .ok_or_else(|| FolderError::DashboardNotFound(dashboard.as_str().to_owned()))?;
    shelved(seen.access, seen_folder.access).map_err(FolderError::WrongShelf)?;

    let entry = seen.entry;
    store.add_entry(folder, dashboard, caller, entry).await?;
    tracing::info!(
        actor_person_id = %caller.person,
        folder_id = %folder,
        dashboard_id = dashboard.as_str(),
        carries = entry.carries(),
        "dashboard added to a folder"
    );

    Ok(entry)
}

/// Takes a dashboard out of a folder the caller may arrange. An entry goes
/// whether or not they still see its dashboard; without one, a dashboard
/// they cannot see is not found.
pub(crate) async fn remove(
    store: &dyn DefinitionStore,
    caller: Caller,
    folder: FolderId,
    dashboard: &DefinitionId,
) -> Result<(), FolderError> {
    seen_folder(store, caller, folder).await?.access.arrange()?;

    if store.remove_entry(folder, dashboard).await? {
        tracing::info!(
            actor_person_id = %caller.person,
            folder_id = %folder,
            dashboard_id = dashboard.as_str(),
            "dashboard removed from a folder"
        );
        return Ok(());
    }
    if sight(store, dashboard, caller).await?.is_none() {
        return Err(FolderError::DashboardNotFound(
            dashboard.as_str().to_owned(),
        ));
    }

    Ok(())
}

/// INVARIANT: one store holds them all, because a dashboard duplicate carries
/// its owner, entries and tags inside the definitions' own transaction.
pub(crate) trait DefinitionStore:
    Definitions + Folders + Tags + Pins + DashboardCatalogue + Grants
{
}

impl<T: Definitions + Folders + Tags + Pins + DashboardCatalogue + Grants> DefinitionStore for T {}

#[cfg(test)]
mod tests;
