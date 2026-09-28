use std::fmt;

use async_trait::async_trait;
use thiserror::Error;

use crate::domain::definition::{
    DefinitionKind, DefinitionName, DefinitionStoreError, NamePage, Page,
};
use crate::domain::folders::FolderFilter;

const MAX_NAME_CHARS: usize = 32;

pub(crate) const MAX_TAGS_PER_DASHBOARD: usize = 10;

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
    #[error("no dashboard is named `{0}`")]
    DashboardNotFound(String),
    #[error("at most {MAX_TAGS} tags can exist; reuse one that does")]
    TooMany,
    #[error(transparent)]
    Store(#[from] DefinitionStoreError),
}

#[async_trait]
pub(crate) trait Tags: Send + Sync + fmt::Debug {
    async fn list_tags(&self) -> Result<Vec<TagSummary>, TagError>;

    async fn tags_of(&self, dashboard: &DefinitionName) -> Result<Vec<TagName>, TagError>;

    async fn set_tags(&self, dashboard: &DefinitionName, tags: &TagSet) -> Result<(), TagError>;

    async fn page_tagged(
        &self,
        needle: &str,
        page: Page,
        folder: Option<FolderFilter>,
        tags: &TagFilter,
    ) -> Result<NamePage, TagError>;
}

#[cfg(test)]
mod tests;
