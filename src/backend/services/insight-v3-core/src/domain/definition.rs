pub(crate) mod arriving;

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::access::{Caller, Owner, PersonId};

const MAX_ID_CHARS: usize = 128;
const MAX_LABEL_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DefinitionKind {
    Metric,
    Widget,
    Dashboard,
}

impl DefinitionKind {
    pub(crate) const ALL: [Self; 3] = [Self::Metric, Self::Widget, Self::Dashboard];

    pub(crate) fn plural(self) -> &'static str {
        match self {
            Self::Metric => "metrics",
            Self::Widget => "widgets",
            Self::Dashboard => "dashboards",
        }
    }

    pub(crate) fn table(self) -> &'static str {
        match self {
            Self::Metric => "metrics",
            Self::Widget => "widgets",
            Self::Dashboard => "dashboards",
        }
    }

    pub(crate) fn singular(self) -> &'static str {
        match self {
            Self::Metric => "metric",
            Self::Widget => "widget",
            Self::Dashboard => "dashboard",
        }
    }

    pub(crate) fn library(self) -> Option<LibraryKind> {
        match self {
            Self::Metric => Some(LibraryKind::Metric),
            Self::Widget => Some(LibraryKind::Widget),
            Self::Dashboard => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryKind {
    Metric,
    Widget,
}

impl LibraryKind {
    pub(crate) fn kind(self) -> DefinitionKind {
        match self {
            Self::Metric => DefinitionKind::Metric,
            Self::Widget => DefinitionKind::Widget,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionId(String);

impl DefinitionId {
    pub(crate) const PATTERN: &'static str = "^[A-Za-z0-9_-]{1,128}$";

    pub(crate) fn parse(value: &str) -> Result<Self, DefinitionError> {
        if value.is_empty() || value.chars().count() > MAX_ID_CHARS {
            return Err(DefinitionError::Id);
        }

        if !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(DefinitionError::Id);
        }

        Ok(Self(value.to_owned()))
    }

    pub(crate) fn generate() -> Self {
        Self(Uuid::now_v7().hyphenated().to_string())
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionLabel(String);

impl DefinitionLabel {
    pub(crate) fn parse(value: &str) -> Result<Self, DefinitionError> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_LABEL_CHARS {
            return Err(DefinitionError::Label);
        }
        if trimmed.chars().any(char::is_control) {
            return Err(DefinitionError::Label);
        }

        Ok(Self(trimmed.to_owned()))
    }

    pub(crate) fn of_id(id: &DefinitionId) -> Self {
        Self(id.as_str().to_owned())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

const DEFAULT_PAGE_LIMIT: u64 = 50;
pub(crate) const MAX_PAGE_LIMIT: u64 = 200;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Page {
    limit: u64,
    offset: u64,
}

impl Page {
    pub(crate) fn parse(limit: Option<u64>, offset: Option<u64>) -> Result<Self, PageError> {
        let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
        if limit == 0 || limit > MAX_PAGE_LIMIT {
            return Err(PageError::Limit(MAX_PAGE_LIMIT));
        }

        Ok(Self {
            limit,
            offset: offset.unwrap_or(0),
        })
    }

    pub(crate) fn limit(self) -> u64 {
        self.limit
    }

    pub(crate) fn offset(self) -> u64 {
        self.offset
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listed {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[derive(Debug)]
pub(crate) struct ListedPage {
    pub(crate) items: Vec<Listed>,
    pub(crate) total: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct LibraryEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) body: serde_json::Value,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug)]
pub(crate) struct LibraryPage {
    pub(crate) items: Vec<LibraryEntry>,
    pub(crate) total: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredDefinition {
    pub(crate) name: String,
    pub(crate) body: serde_json::Value,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug)]
pub(crate) enum Change {
    Create(
        DefinitionKind,
        DefinitionId,
        DefinitionLabel,
        serde_json::Value,
    ),
    Replace(
        DefinitionKind,
        DefinitionId,
        DefinitionLabel,
        serde_json::Value,
    ),
    Own {
        dashboard: DefinitionId,
        owner: PersonId,
    },
    CarryEntries {
        from: DefinitionId,
        to: DefinitionId,
        placer: Caller,
    },
    CarryTags {
        from: DefinitionId,
        to: DefinitionId,
        scope: Owner,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct Definition {
    pub(crate) kind: DefinitionKind,
    pub(crate) id: DefinitionId,
    pub(crate) body: serde_json::Value,
}

impl Definition {
    pub(crate) fn new(kind: DefinitionKind, id: DefinitionId, body: serde_json::Value) -> Self {
        Self { kind, id, body }
    }
}

#[async_trait]
pub(crate) trait Lookup: Send + Sync + fmt::Debug {
    async fn get(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError>;
}

#[async_trait]
pub(crate) trait Definitions: Lookup {
    async fn read(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Option<StoredDefinition>, DefinitionStoreError>;

    #[cfg(test)]
    async fn create(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError>;

    async fn replace(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<bool, DefinitionStoreError>;

    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError>;

    async fn page(
        &self,
        kind: LibraryKind,
        needle: &str,
        page: Page,
    ) -> Result<ListedPage, DefinitionStoreError>;

    async fn page_entries(
        &self,
        kind: LibraryKind,
        needle: &str,
        page: Page,
    ) -> Result<LibraryPage, DefinitionStoreError>;

    async fn names_of(
        &self,
        kind: LibraryKind,
        ids: &[&str],
    ) -> Result<HashMap<String, String>, DefinitionStoreError>;

    async fn delete(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<bool, DefinitionStoreError>;

    async fn apply(&self, changes: &[Change]) -> Result<(), DefinitionStoreError>;
}

#[derive(Clone, Copy, Debug, Error)]
pub(crate) enum DefinitionError {
    #[error("identifiers use letters, digits, underscore and dash, up to 128 characters")]
    Id,
    #[error(
        "a name is 1 to 128 characters with no control characters, not counting the spaces around it"
    )]
    Label,
}

#[derive(Clone, Copy, Debug, Error)]
pub(crate) enum PageError {
    #[error("limit must be between 1 and {0}")]
    Limit(u64),
}

#[derive(Debug, Error)]
pub(crate) enum DefinitionStoreError {
    #[error("definition store operation failed")]
    Database(#[from] sea_orm::DbErr),
    #[error("`{0}` is already taken")]
    IdTaken(String),
    #[error("`{0}` does not exist")]
    Missing(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(
        "at most {} tags can exist per owner; reuse one that does",
        crate::domain::tags::MAX_TAGS
    )]
    TagsFull,
    #[error("a stored {0} is not one this service wrote")]
    UnreadableRow(&'static str),
}

#[cfg(test)]
mod tests;
