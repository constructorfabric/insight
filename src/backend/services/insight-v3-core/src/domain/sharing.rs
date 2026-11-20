use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::domain::access::{
    Caller, DASHBOARD_NOT_EDITOR, DASHBOARD_NOT_SHARER, FOLDER_NOT_EDITOR, Grantee, Level, Owner,
    PersonId, Refused,
};
use crate::domain::dashboards::{Seen, sight};
use crate::domain::definition::{DefinitionId, DefinitionStoreError, Page};
use crate::domain::folders::{DefinitionStore, FolderId, MAX_FOLDERS, SeenFolder};
use crate::domain::tags::MAX_TAGS;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Shared<'a> {
    Dashboard(&'a DefinitionId),
    Folder(FolderId),
}

impl Shared<'_> {
    fn id(&self) -> String {
        match self {
            Self::Dashboard(id) => id.as_str().to_owned(),
            Self::Folder(id) => id.to_string(),
        }
    }

    fn noun(&self) -> &'static str {
        match self {
            Self::Dashboard(_) => "dashboard",
            Self::Folder(_) => "folder",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grant {
    pub(crate) grantee: Grantee,
    pub(crate) level: Level,
    pub(crate) granted_by: PersonId,
    pub(crate) created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Orphan {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) owner: PersonId,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Handover {
    Handed,
    OwnerChanged,
    RecipientFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Granting {
    Done,
    NotSharer,
    ToOwner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Withdrawal {
    Done,
    NotSharer,
}

#[derive(Debug)]
pub(crate) struct OrphanPage {
    pub(crate) items: Vec<Orphan>,
    pub(crate) total: u64,
}

#[async_trait]
pub(crate) trait Grants: Send + Sync + fmt::Debug {
    async fn grants_of(&self, shared: Shared<'_>) -> Result<Vec<Grant>, DefinitionStoreError>;

    async fn grant(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
        level: Level,
        by: PersonId,
    ) -> Result<Granting, DefinitionStoreError>;

    async fn withdraw(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
        by: PersonId,
    ) -> Result<Withdrawal, DefinitionStoreError>;

    async fn transfer(
        &self,
        dashboard: &DefinitionId,
        from: PersonId,
        to: PersonId,
    ) -> Result<Handover, DefinitionStoreError>;

    async fn transfer_folder(
        &self,
        folder: FolderId,
        from: PersonId,
        to: PersonId,
    ) -> Result<Handover, DefinitionStoreError>;

    async fn page_orphaned(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError>;

    async fn page_orphaned_folders(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError>;
}

#[async_trait]
pub(crate) trait Directory: Send + Sync {
    async fn holds_admin(&self, person: PersonId) -> Result<bool, AdministratorsError>;

    async fn current(&self) -> Result<Vec<PersonId>, AdministratorsError>;

    async fn knows(&self, person: PersonId) -> Result<bool, AdministratorsError>;
}

#[derive(Debug, Error)]
pub(crate) enum AdministratorsError {
    #[error("identity could not be read: {0}")]
    Unavailable(String),
    #[error("the administrator listing came back full, so it may be cut short")]
    Truncated,
}

#[derive(Debug, Error)]
pub(crate) enum SharingError {
    #[error("dashboard `{0}` was not found")]
    NotFound(String),
    #[error("no folder has the id `{0}`")]
    FolderNotFound(FolderId),
    #[error("{DASHBOARD_NOT_EDITOR}")]
    NotEditor,
    #[error("{DASHBOARD_NOT_SHARER}")]
    NotSharer,
    #[error("{FOLDER_NOT_EDITOR}")]
    FolderNotEditor,
    #[error("an ownerless {0} cannot be shared or transferred")]
    Ownerless(&'static str),
    #[error("the owner's access is not a grant")]
    GrantToOwner,
    #[error("no person has the id `{0}`")]
    UnknownPerson(PersonId),
    #[error("a {0} is transferred only to a current administrator")]
    RecipientNotAdministrator(&'static str),
    #[error("the {0}'s owner is still an administrator")]
    OwnerIsAdministrator(&'static str),
    #[error("a folder shared with no one is not orphaned")]
    FolderUnshared,
    #[error("the {0} changed owner meanwhile")]
    Moved(&'static str),
    #[error(
        "the new owner has too many tags to take this dashboard's; at most {MAX_TAGS} tags can exist per owner"
    )]
    RecipientTagsFull,
    #[error(
        "the new owner has too many folders to take this one; at most {MAX_FOLDERS} folders can be theirs"
    )]
    RecipientFoldersFull,
    #[error(transparent)]
    Administrators(AdministratorsError),
    #[error(transparent)]
    Store(#[from] DefinitionStoreError),
}

impl From<AdministratorsError> for SharingError {
    fn from(error: AdministratorsError) -> Self {
        Self::Administrators(error)
    }
}

fn refused(shared: Shared<'_>, refused: Refused) -> SharingError {
    match (shared, refused) {
        (Shared::Dashboard(_), Refused::NotEditor | Refused::NotOwner) => SharingError::NotEditor,
        (Shared::Dashboard(_), Refused::EditorByFolder) => SharingError::NotSharer,
        (Shared::Folder(_), Refused::NotEditor | Refused::NotOwner | Refused::EditorByFolder) => {
            SharingError::FolderNotEditor
        }
        (shared, Refused::Ownerless) => SharingError::Ownerless(shared.noun()),
    }
}

fn no_longer_sharer(shared: Shared<'_>) -> SharingError {
    match shared {
        Shared::Dashboard(_) => SharingError::NotSharer,
        Shared::Folder(_) => SharingError::FolderNotEditor,
    }
}

pub(crate) struct Sharing<'a> {
    store: &'a dyn DefinitionStore,
    directory: &'a dyn Directory,
    caller: Caller,
}

impl fmt::Debug for Sharing<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Sharing")
            .field("caller", &self.caller)
            .finish_non_exhaustive()
    }
}

impl<'a> Sharing<'a> {
    pub(crate) fn new(
        store: &'a dyn DefinitionStore,
        directory: &'a dyn Directory,
        caller: Caller,
    ) -> Self {
        Self {
            store,
            directory,
            caller,
        }
    }

    pub(crate) async fn list(&self, shared: Shared<'_>) -> Result<Vec<Grant>, SharingError> {
        self.shareable(shared).await?;

        Ok(self.store.grants_of(shared).await?)
    }

    pub(crate) async fn grant(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
        level: Level,
    ) -> Result<(), SharingError> {
        let owner = self.shareable(shared).await?;
        self.grantable(grantee, owner).await?;

        match self
            .store
            .grant(shared, grantee, level, self.caller.person)
            .await?
        {
            Granting::Done => {}
            Granting::NotSharer => return Err(no_longer_sharer(shared)),
            Granting::ToOwner => return Err(SharingError::GrantToOwner),
        }
        tracing::info!(
            actor_person_id = %self.caller.person,
            object = shared.noun(),
            object_id = %shared.id(),
            grantee = %grantee,
            level = level.as_str(),
            "grant set"
        );

        Ok(())
    }

    pub(crate) async fn withdraw(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
    ) -> Result<(), SharingError> {
        self.shareable(shared).await?;

        match self
            .store
            .withdraw(shared, grantee, self.caller.person)
            .await?
        {
            Withdrawal::Done => {}
            Withdrawal::NotSharer => return Err(no_longer_sharer(shared)),
        }
        tracing::info!(
            actor_person_id = %self.caller.person,
            object = shared.noun(),
            object_id = %shared.id(),
            grantee = %grantee,
            "grant withdrawn"
        );

        Ok(())
    }

    async fn shareable(&self, shared: Shared<'_>) -> Result<Owner, SharingError> {
        let (owner, access) = match shared {
            Shared::Dashboard(id) => {
                let seen = self.seen_dashboard(id).await?;
                (seen.owner, seen.share())
            }
            Shared::Folder(id) => {
                let seen = self.seen_folder(id).await?;
                (seen.folder.owner, seen.access.share())
            }
        };
        access.map_err(|refusal| refused(shared, refusal))?;

        Ok(owner)
    }

    async fn grantable(&self, grantee: Grantee, owner: Owner) -> Result<(), SharingError> {
        let Grantee::Person(person) = grantee else {
            return Ok(());
        };
        if owner == Owner::Person(person) {
            return Err(SharingError::GrantToOwner);
        }
        if !self.directory.knows(person).await? {
            return Err(SharingError::UnknownPerson(person));
        }

        Ok(())
    }

    async fn seen_dashboard(&self, id: &DefinitionId) -> Result<Seen, SharingError> {
        sight(self.store, id, self.caller)
            .await?
            .ok_or_else(|| SharingError::NotFound(id.as_str().to_owned()))
    }

    async fn seen_folder(&self, id: FolderId) -> Result<SeenFolder, SharingError> {
        self.store
            .held_folder(id, self.caller)
            .await?
            .and_then(|held| SeenFolder::of(self.caller, held))
            .ok_or(SharingError::FolderNotFound(id))
    }

    async fn administrators(&self) -> Result<Vec<PersonId>, SharingError> {
        let administrators = self.directory.current().await?;
        if !administrators.contains(&self.caller.person) {
            return Err(SharingError::Administrators(
                AdministratorsError::Unavailable(
                    "the administrator listing leaves out the administrator asking".to_owned(),
                ),
            ));
        }

        Ok(administrators)
    }

    pub(crate) async fn orphaned(&self, page: Page) -> Result<OrphanPage, SharingError> {
        let administrators = self.administrators().await?;

        Ok(self.store.page_orphaned(&administrators, page).await?)
    }

    pub(crate) async fn orphaned_folders(&self, page: Page) -> Result<OrphanPage, SharingError> {
        let administrators = self.administrators().await?;

        Ok(self
            .store
            .page_orphaned_folders(&administrators, page)
            .await?)
    }

    pub(crate) async fn transfer(
        &self,
        dashboard: &DefinitionId,
        to: PersonId,
    ) -> Result<(), SharingError> {
        let missing = || SharingError::NotFound(dashboard.as_str().to_owned());
        let standing = self
            .store
            .standing(dashboard, self.caller)
            .await?
            .ok_or_else(missing)?;
        let Owner::Person(owner) = standing.owner else {
            return Err(SharingError::Ownerless("dashboard"));
        };
        let seen = Seen::of(self.caller, &standing).is_some();

        self.owner_left(seen, owner, "dashboard", missing).await?;
        self.recipient(to, "dashboard").await?;

        match self.store.transfer(dashboard, owner, to).await? {
            Handover::Handed => {}
            Handover::OwnerChanged => return Err(SharingError::Moved("dashboard")),
            Handover::RecipientFull => return Err(SharingError::RecipientTagsFull),
        }
        tracing::info!(
            actor_person_id = %self.caller.person,
            object = "dashboard",
            object_id = dashboard.as_str(),
            from_person_id = %owner,
            to_person_id = %to,
            "ownership transferred"
        );

        Ok(())
    }

    pub(crate) async fn transfer_folder(
        &self,
        folder: FolderId,
        to: PersonId,
    ) -> Result<(), SharingError> {
        let missing = || SharingError::FolderNotFound(folder);
        let held = self
            .store
            .held_folder(folder, self.caller)
            .await?
            .ok_or_else(missing)?;
        let Owner::Person(owner) = held.folder.owner else {
            return Err(SharingError::Ownerless("folder"));
        };
        let seen = SeenFolder::of(self.caller, held).is_some();

        self.owner_left(seen, owner, "folder", missing).await?;
        if self
            .store
            .grants_of(Shared::Folder(folder))
            .await?
            .is_empty()
        {
            return Err(if seen {
                SharingError::FolderUnshared
            } else {
                missing()
            });
        }
        self.recipient(to, "folder").await?;

        match self.store.transfer_folder(folder, owner, to).await? {
            Handover::Handed => {}
            Handover::OwnerChanged => return Err(SharingError::Moved("folder")),
            Handover::RecipientFull => return Err(SharingError::RecipientFoldersFull),
        }
        tracing::info!(
            actor_person_id = %self.caller.person,
            object = "folder",
            object_id = %folder,
            from_person_id = %owner,
            to_person_id = %to,
            "ownership transferred"
        );

        Ok(())
    }

    async fn recipient(&self, to: PersonId, noun: &'static str) -> Result<(), SharingError> {
        if self.directory.holds_admin(to).await? {
            return Ok(());
        }

        Err(SharingError::RecipientNotAdministrator(noun))
    }

    async fn owner_left(
        &self,
        seen: bool,
        owner: PersonId,
        noun: &'static str,
        missing: impl Fn() -> SharingError,
    ) -> Result<(), SharingError> {
        match (seen, self.directory.holds_admin(owner).await) {
            (_, Ok(false)) => Ok(()),
            (true, Ok(true)) => Err(SharingError::OwnerIsAdministrator(noun)),
            (true, Err(error)) => Err(error.into()),
            (false, Ok(true)) => Err(missing()),
            (false, Err(error)) => {
                tracing::warn!(
                    error = %error,
                    object = noun,
                    "a transfer of something the caller cannot see met an identity failure"
                );
                Err(missing())
            }
        }
    }
}

#[cfg(test)]
mod tests;
