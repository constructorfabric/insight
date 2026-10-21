//! The dashboards one caller sees, one at a time and a page at a time.

use std::fmt;

use async_trait::async_trait;
use thiserror::Error;

use crate::domain::access::{Access, Owner, PersonId, access};
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionStoreError, Page};
use crate::domain::folders::{FolderError, FolderFilter};
use crate::domain::tags::{TagError, TagFilter};

/// What decides a caller's access to one stored dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Standing {
    pub(crate) owner: Owner,
    pub(crate) shared_with_caller: bool,
}

/// A dashboard the caller sees, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) owner: Owner,
    pub(crate) access: Access,
}

impl Standing {
    pub(crate) fn seen_by(self, caller: PersonId) -> Option<Seen> {
        let access = access(caller, self.owner, self.shared_with_caller)?;

        Some(Seen {
            owner: self.owner,
            access,
        })
    }
}

/// Which of the visible dashboards a listing keeps by who owns them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owned {
    /// The caller's own.
    Me,
    /// Someone else's, shared with the caller.
    Shared,
}

impl Owned {
    pub(crate) fn parse(kind: DefinitionKind, raw: &str) -> Result<Self, OwnedError> {
        if kind != DefinitionKind::Dashboard {
            return Err(OwnedError::NotOwned(kind.plural()));
        }

        match raw {
            "me" => Ok(Self::Me),
            "shared" => Ok(Self::Shared),
            _ => Err(OwnedError::Word),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum OwnedError {
    #[error("owned is `me` or `shared`")]
    Word,
    #[error("{0} have no owner to filter by")]
    NotOwned(&'static str),
}

/// What a listing of dashboards keeps, beyond being visible to the caller.
#[derive(Debug, Clone, Default)]
pub(crate) struct DashboardQuery {
    /// Text to find in a name or a body; blank keeps every one.
    pub(crate) needle: String,
    pub(crate) folder: Option<FolderFilter>,
    pub(crate) tags: Option<TagFilter>,
    pub(crate) owned: Option<Owned>,
}

impl DashboardQuery {
    /// The filters a listing of `kind` was asked for; any of them on a kind
    /// other than dashboards is refused.
    pub(crate) fn parse(
        kind: DefinitionKind,
        needle: &str,
        folder: Option<&str>,
        tags: &[String],
        owned: Option<&str>,
    ) -> Result<Self, QueryError> {
        let folder = folder
            .map(|raw| FolderFilter::parse(kind, raw))
            .transpose()
            .map_err(QueryError::Folder)?;
        let tags = TagFilter::parse(kind, tags).map_err(QueryError::Tag)?;
        let owned = owned
            .map(|raw| Owned::parse(kind, raw))
            .transpose()
            .map_err(QueryError::Owned)?;

        Ok(Self {
            needle: needle.trim().to_owned(),
            folder,
            tags,
            owned,
        })
    }
}

#[derive(Debug, Error)]
pub(crate) enum QueryError {
    #[error(transparent)]
    Folder(FolderError),
    #[error(transparent)]
    Tag(TagError),
    #[error(transparent)]
    Owned(OwnedError),
}

/// One dashboard in a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedDashboard {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) owner: Owner,
    pub(crate) access: Access,
}

#[derive(Debug)]
pub(crate) struct DashboardPage {
    pub(crate) items: Vec<ListedDashboard>,
    /// Every visible match, not the page.
    pub(crate) total: u64,
}

/// What the store answers about dashboards for one caller.
///
/// INVARIANT: every page, count and lookup here keeps only the dashboards
/// [`access`] lets the caller see, so a dashboard they may not see is never
/// counted, listed or told apart from a missing one.
#[async_trait]
pub(crate) trait DashboardCatalogue: Send + Sync + fmt::Debug {
    /// The dashboard's owner and whether it is shared with `caller`, or none
    /// when no dashboard has the id.
    async fn standing(
        &self,
        dashboard: &DefinitionId,
        caller: PersonId,
    ) -> Result<Option<Standing>, DefinitionStoreError>;

    /// One page of the dashboards `caller` sees that match `query`, ordered
    /// by name, then id.
    async fn page_dashboards(
        &self,
        caller: PersonId,
        query: &DashboardQuery,
        page: Page,
    ) -> Result<DashboardPage, DefinitionStoreError>;
}

/// How `caller` sees the dashboard, or none when they cannot or it is not
/// there - which the caller is never told apart.
pub(crate) async fn sight(
    catalogue: &dyn DashboardCatalogue,
    dashboard: &DefinitionId,
    caller: PersonId,
) -> Result<Option<Seen>, DefinitionStoreError> {
    let standing = catalogue.standing(dashboard, caller).await?;

    Ok(standing.and_then(|standing| standing.seen_by(caller)))
}
