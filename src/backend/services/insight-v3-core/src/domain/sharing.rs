//! Sharing a dashboard with other administrators, and handing one whose owner
//! is no longer an administrator to one who is.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::domain::access::{Owner, PersonId, Refused};
use crate::domain::dashboards::{Seen, Standing, sight};
use crate::domain::definition::{DefinitionId, DefinitionStoreError, Page};
use crate::domain::folders::DefinitionStore;
use crate::domain::tags::MAX_TAGS;

/// One person a dashboard is shared with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Share {
    pub(crate) person: PersonId,
    pub(crate) granted_by: PersonId,
    pub(crate) created_at: DateTime<Utc>,
}

/// A dashboard whose owner no longer holds the administrator role.
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
    /// Its tags would take the new owner's scope past [`MAX_TAGS`].
    RecipientTagsFull,
}

#[derive(Debug)]
pub(crate) struct OrphanPage {
    pub(crate) items: Vec<Orphan>,
    pub(crate) total: u64,
}

#[async_trait]
pub(crate) trait Sharing: Send + Sync + fmt::Debug {
    async fn shares_of(&self, dashboard: &DefinitionId)
    -> Result<Vec<Share>, DefinitionStoreError>;

    /// Shares the dashboard with `person`; a share already held is kept as
    /// it was.
    async fn share(
        &self,
        dashboard: &DefinitionId,
        person: PersonId,
        granted_by: PersonId,
    ) -> Result<(), DefinitionStoreError>;

    /// Removes `person`'s share, pin and filing of the dashboard together.
    async fn withdraw(
        &self,
        dashboard: &DefinitionId,
        person: PersonId,
    ) -> Result<(), DefinitionStoreError>;

    /// Makes `to` the owner if `from` still is, dropping any share `to`
    /// held and moving the dashboard's tags into `to`'s scope under the same
    /// names, all or nothing.
    async fn transfer(
        &self,
        dashboard: &DefinitionId,
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
}

/// Who holds the administrator role right now, as identity answers it.
#[async_trait]
pub(crate) trait Administrators: Send + Sync {
    async fn holds_admin(&self, person: PersonId) -> Result<bool, AdministratorsError>;

    /// Every current administrator.
    async fn current(&self) -> Result<Vec<PersonId>, AdministratorsError>;
}

#[derive(Debug, Error)]
pub(crate) enum AdministratorsError {
    #[error("the administrators could not be read: {0}")]
    Unavailable(String),
    #[error("the administrator listing came back full, so it may be cut short")]
    Truncated,
}

#[derive(Debug, Error)]
pub(crate) enum SharingError {
    #[error("dashboard `{0}` was not found")]
    NotFound(String),
    #[error("only the owner can change this dashboard")]
    NotOwner,
    #[error("an ownerless dashboard cannot be shared or transferred")]
    Ownerless,
    #[error("the owner already sees this dashboard")]
    SharedWithOwner,
    #[error("a dashboard is shared only with a current administrator")]
    TargetNotAdministrator,
    #[error("a dashboard is transferred only to a current administrator")]
    RecipientNotAdministrator,
    #[error("the dashboard's owner is still an administrator")]
    OwnerIsAdministrator,
    #[error("the dashboard changed owner meanwhile")]
    Moved,
    #[error(
        "the new owner has too many tags to take this dashboard's; at most {MAX_TAGS} tags can exist per owner"
    )]
    RecipientTagsFull,
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

impl From<Refused> for SharingError {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::NotOwner => Self::NotOwner,
            Refused::Ownerless => Self::Ownerless,
        }
    }
}

/// The sharing operations, as one administrator.
pub(crate) struct DashboardShares<'a> {
    store: &'a dyn DefinitionStore,
    administrators: &'a dyn Administrators,
    caller: PersonId,
}

impl fmt::Debug for DashboardShares<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DashboardShares")
            .field("caller", &self.caller)
            .finish_non_exhaustive()
    }
}

impl<'a> DashboardShares<'a> {
    pub(crate) fn new(
        store: &'a dyn DefinitionStore,
        administrators: &'a dyn Administrators,
        caller: PersonId,
    ) -> Self {
        Self {
            store,
            administrators,
            caller,
        }
    }

    pub(crate) async fn list(&self, dashboard: &DefinitionId) -> Result<Vec<Share>, SharingError> {
        self.owned(dashboard).await?;

        Ok(self.store.shares_of(dashboard).await?)
    }

    pub(crate) async fn grant(
        &self,
        dashboard: &DefinitionId,
        person: PersonId,
    ) -> Result<(), SharingError> {
        self.owned(dashboard).await?;
        if person == self.caller {
            return Err(SharingError::SharedWithOwner);
        }
        if !self.administrators.holds_admin(person).await? {
            return Err(SharingError::TargetNotAdministrator);
        }

        self.store.share(dashboard, person, self.caller).await?;
        tracing::info!(
            actor_person_id = %self.caller,
            dashboard_id = dashboard.as_str(),
            person_id = %person,
            "dashboard shared"
        );

        Ok(())
    }

    pub(crate) async fn withdraw(
        &self,
        dashboard: &DefinitionId,
        person: PersonId,
    ) -> Result<(), SharingError> {
        self.owned(dashboard).await?;
        if person == self.caller {
            return Err(SharingError::SharedWithOwner);
        }

        self.store.withdraw(dashboard, person).await?;
        tracing::info!(
            actor_person_id = %self.caller,
            dashboard_id = dashboard.as_str(),
            person_id = %person,
            "dashboard share withdrawn"
        );

        Ok(())
    }

    /// The dashboards whose owner is not a current administrator; an
    /// administrator listing that may be cut short fails the whole answer.
    pub(crate) async fn orphaned(&self, page: Page) -> Result<OrphanPage, SharingError> {
        let administrators = self.administrators.current().await?;
        if !administrators.contains(&self.caller) {
            return Err(SharingError::Administrators(
                AdministratorsError::Unavailable(
                    "the administrator listing leaves out the administrator asking".to_owned(),
                ),
            ));
        }

        Ok(self.store.page_orphaned(&administrators, page).await?)
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
        let standing = self
            .store
            .standing(dashboard, self.caller)
            .await?
            .ok_or_else(|| SharingError::NotFound(dashboard.as_str().to_owned()))?;
        let Owner::Person(owner) = standing.owner else {
            return Err(SharingError::Ownerless);
        };

        self.owner_left(dashboard, standing, owner).await?;
        if !self.administrators.holds_admin(to).await? {
            return Err(SharingError::RecipientNotAdministrator);
        }

        match self.store.transfer(dashboard, owner, to).await? {
            Handover::Handed => {}
            Handover::OwnerChanged => return Err(SharingError::Moved),
            Handover::RecipientTagsFull => return Err(SharingError::RecipientTagsFull),
        }
        tracing::info!(
            actor_person_id = %self.caller,
            dashboard_id = dashboard.as_str(),
            from_person_id = %owner,
            to_person_id = %to,
            "dashboard transferred"
        );

        Ok(())
    }

    /// Whether `owner` no longer holds the administrator role. Where the
    /// caller cannot see the dashboard, any other answer is that it is
    /// missing, so neither the role nor an identity failure tells a private
    /// dashboard from an absent one.
    async fn owner_left(
        &self,
        dashboard: &DefinitionId,
        standing: Standing,
        owner: PersonId,
    ) -> Result<(), SharingError> {
        let missing = || SharingError::NotFound(dashboard.as_str().to_owned());
        let seen = standing.seen_by(self.caller).is_some();

        match (seen, self.administrators.holds_admin(owner).await) {
            (_, Ok(false)) => Ok(()),
            (true, Ok(true)) => Err(SharingError::OwnerIsAdministrator),
            (true, Err(error)) => Err(error.into()),
            (false, Ok(true)) => Err(missing()),
            (false, Err(error)) => {
                tracing::warn!(
                    error = %error,
                    dashboard_id = dashboard.as_str(),
                    "a transfer of a dashboard the caller cannot see met an identity failure"
                );
                Err(missing())
            }
        }
    }

    async fn owned(&self, dashboard: &DefinitionId) -> Result<Seen, SharingError> {
        let seen = sight(self.store, dashboard, self.caller)
            .await?
            .ok_or_else(|| SharingError::NotFound(dashboard.as_str().to_owned()))?;
        seen.access.share()?;

        Ok(seen)
    }
}

#[cfg(test)]
mod tests;
