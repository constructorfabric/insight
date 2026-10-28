//! Granting dashboards and folders to people or to everyone, and handing one
//! whose owner is no longer an administrator to one who is.

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

/// What a grant is on.
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

/// One grant of a dashboard or a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grant {
    pub(crate) grantee: Grantee,
    pub(crate) level: Level,
    pub(crate) granted_by: PersonId,
    pub(crate) created_at: DateTime<Utc>,
}

/// A dashboard or a folder whose owner no longer holds the administrator
/// role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Orphan {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) owner: PersonId,
    pub(crate) updated_at: DateTime<Utc>,
}

/// What a hand-over came to; only [`Handover::Handed`] changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Handover {
    Handed,
    /// The owner read before the hand-over no longer owned it.
    OwnerChanged,
    /// It would take the new owner past their cap: [`MAX_TAGS`] for a
    /// dashboard's tags, [`MAX_FOLDERS`] for a folder.
    RecipientFull,
}

/// What setting a grant came to; only [`Granting::Done`] changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Granting {
    Done,
    /// The one setting it no longer owns it or holds an editor grant on it,
    /// to them or to everyone.
    NotSharer,
    /// The grantee owns it now.
    ToOwner,
}

/// What withdrawing a grant came to; only [`Withdrawal::Done`] changed
/// anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Withdrawal {
    Done,
    /// The one withdrawing it no longer owns it or holds an editor grant on
    /// it, to them or to everyone.
    NotSharer,
}

#[derive(Debug)]
pub(crate) struct OrphanPage {
    pub(crate) items: Vec<Orphan>,
    pub(crate) total: u64,
}

#[async_trait]
pub(crate) trait Grants: Send + Sync + fmt::Debug {
    /// Every grant of it, oldest first.
    async fn grants_of(&self, shared: Shared<'_>) -> Result<Vec<Grant>, DefinitionStoreError>;

    /// Grants it to `grantee` at `level` as `by`; granting again sets the
    /// level.
    ///
    /// INVARIANT: lands only while `by` owns it or holds an editor grant on
    /// it to them or to everyone, and `grantee` does not own it, as the
    /// grant is written.
    async fn grant(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
        level: Level,
        by: PersonId,
    ) -> Result<Granting, DefinitionStoreError>;

    /// Withdraws `grantee`'s grant as `by`, leaving every pin and entry in
    /// place.
    ///
    /// INVARIANT: lands only while `by` owns it or holds an editor grant on
    /// it to them or to everyone, as the grant is removed.
    async fn withdraw(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
        by: PersonId,
    ) -> Result<Withdrawal, DefinitionStoreError>;

    /// Makes `to` the dashboard's owner if `from` still is, dropping any
    /// grant `to` held and moving its tags into `to`'s scope under the same
    /// names, all or nothing.
    async fn transfer(
        &self,
        dashboard: &DefinitionId,
        from: PersonId,
        to: PersonId,
    ) -> Result<Handover, DefinitionStoreError>;

    /// Makes `to` the folder's owner if `from` still is and `to` owns fewer
    /// than [`MAX_FOLDERS`], dropping any grant `to` held; its other grants
    /// and its entries stay.
    async fn transfer_folder(
        &self,
        folder: FolderId,
        from: PersonId,
        to: PersonId,
    ) -> Result<Handover, DefinitionStoreError>;

    /// One page of the owned dashboards whose owner is none of
    /// `administrators`, ordered by name, then id.
    async fn page_orphaned(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError>;

    /// One page of the owned folders with at least one grant whose owner is
    /// none of `administrators`, ordered by name, then id.
    async fn page_orphaned_folders(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError>;
}

/// Who holds the administrator role right now, and who exists, as identity
/// answers it.
#[async_trait]
pub(crate) trait Directory: Send + Sync {
    async fn holds_admin(&self, person: PersonId) -> Result<bool, AdministratorsError>;

    /// Every current administrator.
    async fn current(&self) -> Result<Vec<PersonId>, AdministratorsError>;

    /// Whether identity knows a person by this id.
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

/// The sharing operations, as one administrator.
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

    /// The owner of something the caller may share, or why they may not.
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

    /// Refuses a grant to the owner, whose access is not one, and to a
    /// person identity does not know.
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

    /// The current administrators, refused when the listing leaves out the
    /// administrator asking.
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

    /// The dashboards whose owner is not a current administrator; an
    /// administrator listing that may be cut short fails the whole answer.
    pub(crate) async fn orphaned(&self, page: Page) -> Result<OrphanPage, SharingError> {
        let administrators = self.administrators().await?;

        Ok(self.store.page_orphaned(&administrators, page).await?)
    }

    /// The folders shared with someone whose owner is not a current
    /// administrator.
    pub(crate) async fn orphaned_folders(&self, page: Page) -> Result<OrphanPage, SharingError> {
        let administrators = self.administrators().await?;

        Ok(self
            .store
            .page_orphaned_folders(&administrators, page)
            .await?)
    }

    /// Hands an orphaned dashboard to a current administrator.
    ///
    /// A dashboard whose owner is still an administrator is answered as
    /// missing unless the caller sees it, so the route reveals no private
    /// dashboard.
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

    /// Hands an orphaned folder - one shared with someone, whose owner is no
    /// longer an administrator - to a current administrator.
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

    /// Whether `owner` no longer holds the administrator role. Where the
    /// caller cannot see the thing, any other answer is that it is missing,
    /// so neither the role nor an identity failure tells a private one from
    /// an absent one.
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
