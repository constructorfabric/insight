use std::fmt;

use async_trait::async_trait;
use thiserror::Error;

use crate::domain::access::{Owner, PersonId, Refused};
use crate::domain::dashboards::sight;
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionStoreError};
use crate::domain::folders::DefinitionStore;

const MAX_NAME_CHARS: usize = 32;

pub(crate) const MAX_TAGS_PER_DASHBOARD: usize = 10;

/// Per owner, the ownerless scope included.
pub(crate) const MAX_TAGS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TagName(String);

impl TagName {
    pub(crate) fn parse(value: &str) -> Result<Self, TagError> {
        let trimmed = value.trim();
        let chars = trimmed.chars().count();
        if chars == 0 || chars > MAX_NAME_CHARS {
            return Err(TagError::Name);
        }

        Ok(Self(trimmed.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn folded(&self) -> String {
        self.0.to_lowercase()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TagSet(Vec<TagName>);

impl TagSet {
    pub(crate) fn parse(values: &[String]) -> Result<Self, TagError> {
        distinct(values, MAX_TAGS_PER_DASHBOARD, TagError::TooManyOnDashboard).map(Self)
    }

    pub(crate) fn names(&self) -> &[TagName] {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TagFilter(Vec<TagName>);

impl TagFilter {
    pub(crate) fn parse(kind: DefinitionKind, values: &[String]) -> Result<Option<Self>, TagError> {
        if values.is_empty() {
            return Ok(None);
        }
        if kind != DefinitionKind::Dashboard {
            return Err(TagError::NotTagged(kind.plural()));
        }

        distinct(values, MAX_TAGS, TagError::FilterTooWide).map(|names| Some(Self(names)))
    }

    pub(crate) fn names(&self) -> &[TagName] {
        &self.0
    }
}

fn distinct(values: &[String], cap: usize, overflow: TagError) -> Result<Vec<TagName>, TagError> {
    let mut names: Vec<TagName> = Vec::new();

    for value in values {
        let name = TagName::parse(value)?;
        if names.iter().any(|held| held.folded() == name.folded()) {
            continue;
        }
        if names.len() == cap {
            return Err(overflow);
        }
        names.push(name);
    }

    Ok(names)
}

#[derive(Debug, Clone)]
pub(crate) struct TagSummary {
    pub(crate) name: TagName,
    pub(crate) dashboards: u64,
}

#[derive(Debug, Error)]
pub(crate) enum TagError {
    #[error("tag names are 1 to 32 characters, not counting the spaces around them")]
    Name,
    #[error("a dashboard carries at most {MAX_TAGS_PER_DASHBOARD} tags")]
    TooManyOnDashboard,
    #[error("filter by at most {MAX_TAGS} tags")]
    FilterTooWide,
    #[error("{0} are not tagged")]
    NotTagged(&'static str),
    #[error("no dashboard has the id `{0}`")]
    DashboardNotFound(String),
    #[error("only the owner can change this dashboard")]
    NotOwner,
    #[error("at most {MAX_TAGS} tags can exist per owner; reuse one that does")]
    TooMany,
    #[error(transparent)]
    Store(#[from] DefinitionStoreError),
}

impl TagError {
    pub(crate) fn is_about_the_caller(&self) -> bool {
        !matches!(self, Self::Store(_))
    }
}

impl From<sea_orm::DbErr> for TagError {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::Store(DefinitionStoreError::Database(error))
    }
}

#[async_trait]
pub(crate) trait Tags: Send + Sync + fmt::Debug {
    /// The names of the tags on dashboards the caller sees, each counting
    /// those dashboards; one name held in several scopes is one entry.
    async fn list_tags(&self, caller: PersonId) -> Result<Vec<TagSummary>, TagError>;

    async fn tags_of(&self, dashboard: &DefinitionId) -> Result<Vec<TagName>, TagError>;

    /// Replaces the dashboard's tags with tags of `scope`, its owner's,
    /// making any that scope does not hold yet.
    async fn set_tags(
        &self,
        dashboard: &DefinitionId,
        scope: Owner,
        tags: &TagSet,
    ) -> Result<(), TagError>;
}

/// Sets the tags of a dashboard the caller may change, in its owner's scope.
pub(crate) async fn retag(
    store: &dyn DefinitionStore,
    caller: PersonId,
    dashboard: &DefinitionId,
    tags: &TagSet,
) -> Result<(), TagError> {
    let seen = sight(store, dashboard, caller)
        .await?
        .ok_or_else(|| TagError::DashboardNotFound(dashboard.as_str().to_owned()))?;
    seen.access.change().map_err(|refused| match refused {
        Refused::NotOwner | Refused::Ownerless => TagError::NotOwner,
    })?;

    store.set_tags(dashboard, seen.owner, tags).await
}

#[cfg(test)]
mod tests;
