//! The dashboards one caller sees, one at a time and a page at a time.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::domain::access::{
    Access, Caller, DashboardStanding, Entry, Level, Owner, Refused, access,
};
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionStoreError, Page};
use crate::domain::folders::{FolderError, FolderFilter};
use crate::domain::tags::{TagError, TagFilter};

/// A dashboard the caller sees, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) owner: Owner,
    pub(crate) access: Access,
    /// What an entry the caller adds now carries.
    pub(crate) entry: Entry,
    /// The strongest of its grants to the caller or to everyone.
    granted: Option<Level>,
}

impl Seen {
    pub(crate) fn of(caller: Caller, standing: &DashboardStanding) -> Option<Self> {
        let access = access(caller, standing)?;

        Some(Self {
            owner: standing.owner,
            access,
            entry: access.entry(standing.granted),
            granted: standing.granted,
        })
    }

    /// Whether the caller may list, add, change and withdraw its grants.
    pub(crate) fn share(&self) -> Result<(), Refused> {
        self.access.share(self.granted)
    }
}

/// Which of the visible dashboards a listing keeps by who owns them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owned {
    /// The caller's own.
    Me,
    /// Not the caller's own, reaching them through a grant or a folder.
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Order {
    #[default]
    Name,
    Recent,
}

impl Order {
    pub(crate) fn parse(kind: DefinitionKind, raw: &str) -> Result<Self, OrderError> {
        if kind != DefinitionKind::Dashboard {
            return Err(OrderError::Unsorted(kind.plural()));
        }

        match raw {
            "name" => Ok(Self::Name),
            "recent" => Ok(Self::Recent),
            _ => Err(OrderError::Word),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum OrderError {
    #[error("sort is `name` or `recent`")]
    Word,
    #[error("{0} list only by name")]
    Unsorted(&'static str),
}

/// What a listing of dashboards keeps, beyond being visible to the caller.
#[derive(Debug, Clone, Default)]
pub(crate) struct DashboardQuery {
    /// Text to find in a name or a body; blank keeps every one.
    pub(crate) needle: String,
    pub(crate) folder: Option<FolderFilter>,
    pub(crate) tags: Option<TagFilter>,
    pub(crate) owned: Option<Owned>,
    pub(crate) order: Order,
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
        order: Option<&str>,
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
        let order = order
            .map(|raw| Order::parse(kind, raw))
            .transpose()
            .map_err(QueryError::Order)?
            .unwrap_or_default();

        Ok(Self {
            needle: needle.trim().to_owned(),
            folder,
            tags,
            owned,
            order,
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
    #[error(transparent)]
    Order(OrderError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Visibility {
    Private,
    People,
    Everyone,
}

impl Visibility {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "private" => Some(Self::Private),
            "people" => Some(Self::People),
            "everyone" => Some(Self::Everyone),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::People => "people",
            Self::Everyone => "everyone",
        }
    }
}

/// One dashboard in a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedDashboard {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) owner: Owner,
    pub(crate) access: Access,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) visibility: Visibility,
    pub(crate) title: Option<String>,
    pub(crate) tags: Vec<String>,
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
    /// What decides `caller`'s access to the dashboard, or none when no
    /// dashboard has the id.
    async fn standing(
        &self,
        dashboard: &DefinitionId,
        caller: Caller,
    ) -> Result<Option<DashboardStanding>, DefinitionStoreError>;

    /// One page of the dashboards `caller` sees that match `query`, ordered
    /// by name, then id.
    async fn page_dashboards(
        &self,
        caller: Caller,
        query: &DashboardQuery,
        page: Page,
    ) -> Result<DashboardPage, DefinitionStoreError>;
}

/// How `caller` sees the dashboard, or none when they cannot or it is not
/// there - which the caller is never told apart.
pub(crate) async fn sight(
    catalogue: &dyn DashboardCatalogue,
    dashboard: &DefinitionId,
    caller: Caller,
) -> Result<Option<Seen>, DefinitionStoreError> {
    let standing = catalogue.standing(dashboard, caller).await?;

    Ok(standing.and_then(|standing| Seen::of(caller, &standing)))
}

#[cfg(test)]
mod tests;
