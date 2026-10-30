//! Metric, widget and dashboard definitions.
//!
//! These live in `MariaDB`, not in `ClickHouse` with the data they describe.
//! They are a few hundred rows read by id and edited in place by whoever is
//! asking the assistant for a change — which is what a row store is for, and
//! what a column store is not. On `ClickHouse` a change meant inserting a new
//! version and reading with `FINAL`, nothing stopped two rows claiming one
//! key, and a create that failed halfway left what it had already written
//! behind with nothing to roll it back with.

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
    /// Every kind there is, for the places that must answer for all of them:
    /// the routes, the tools, and what the assistant is told exists.
    ///
    /// INVARIANT: a kind missing here is a kind with no endpoint and no name
    /// the assistant can reuse.
    pub(crate) const ALL: [Self; 3] = [Self::Metric, Self::Widget, Self::Dashboard];

    /// What a group of them is called: the path segment, and the heading the
    /// assistant reads.
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

    /// The kind as the shared library holds it, or none for a dashboard,
    /// whose catalogue depends on who is asking.
    pub(crate) fn library(self) -> Option<LibraryKind> {
        match self {
            Self::Metric => Some(LibraryKind::Metric),
            Self::Widget => Some(LibraryKind::Widget),
            Self::Dashboard => None,
        }
    }
}

/// A kind every administrator sees all of.
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
    /// The same rule as [`DefinitionId::parse`], for a caller that can only
    /// be given a pattern - the chat tool schema the model answers in.
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

    /// A fresh id for a definition being created.
    ///
    /// INVARIANT: a hyphenated UUID satisfies [`DefinitionId::PATTERN`], so a
    /// generated id and an id carried over from an older name share one rule.
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

/// What a reader calls a definition. Two definitions may share one.
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

    /// An id read as a name, which is what a definition stored before names
    /// were labels is called.
    ///
    /// INVARIANT: every id satisfies the label rule - at most as long, never
    /// blank, no control characters - so this cannot make a label `parse`
    /// would refuse.
    pub(crate) fn of_id(id: &DefinitionId) -> Self {
        Self(id.as_str().to_owned())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The largest page a caller may ask for, and what it gets by default.
const DEFAULT_PAGE_LIMIT: u64 = 50;
pub(crate) const MAX_PAGE_LIMIT: u64 = 200;

/// How much of a catalogue to read.
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

/// One definition in a catalogue: what addresses it and what it is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listed {
    pub(crate) id: String,
    pub(crate) name: String,
}

/// One page of a catalogue, and how many definitions it is a page of.
#[derive(Debug)]
pub(crate) struct ListedPage {
    pub(crate) items: Vec<Listed>,
    /// Every match, not the page — so a reader can say what is behind it.
    pub(crate) total: u64,
}

/// One metric or widget as the portal's catalogue shows it: what addresses
/// it, what it is called, what it holds and when it last changed.
#[derive(Debug, Clone)]
pub(crate) struct LibraryEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) body: serde_json::Value,
    pub(crate) updated_at: DateTime<Utc>,
}

/// One page of library entries, and how many definitions it is a page of.
#[derive(Debug)]
pub(crate) struct LibraryPage {
    pub(crate) items: Vec<LibraryEntry>,
    /// Every match, not the page — so a reader can say what is behind it.
    pub(crate) total: u64,
}

/// A stored definition as a reader is given it.
#[derive(Debug, Clone)]
pub(crate) struct StoredDefinition {
    pub(crate) name: String,
    pub(crate) body: serde_json::Value,
    pub(crate) updated_at: DateTime<Utc>,
}

/// One write in a batch.
#[derive(Debug)]
pub(crate) enum Change {
    /// A write that refuses to replace anything.
    Create(
        DefinitionKind,
        DefinitionId,
        DefinitionLabel,
        serde_json::Value,
    ),
    /// A write that refuses to create anything.
    Replace(
        DefinitionKind,
        DefinitionId,
        DefinitionLabel,
        serde_json::Value,
    ),
    /// Makes a dashboard the person's.
    Own {
        dashboard: DefinitionId,
        owner: PersonId,
    },
    /// Puts `to`, carrying, in each owned folder holding `from` that
    /// `placer` may arrange.
    CarryEntries {
        from: DefinitionId,
        to: DefinitionId,
        placer: Caller,
    },
    /// Tags `to` with the names `from` carries, as tags of `scope`.
    CarryTags {
        from: DefinitionId,
        to: DefinitionId,
        scope: Owner,
    },
}

/// A definition as it stands: what kind it is, what addresses it, and what it
/// holds.
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

/// Reading one definition by id, which is all a kind's own rules may do.
///
/// A rule that could write would write mid-check, before the batch it belongs
/// to has been accepted.
#[async_trait]
pub(crate) trait Lookup: Send + Sync + fmt::Debug {
    async fn get(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError>;
}

/// What the API and the chat need of the store, so neither has to know where
/// definitions live — and so their tests can hold them in a map rather than
/// answer a database's wire protocol.
#[async_trait]
pub(crate) trait Definitions: Lookup {
    async fn read(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Option<StoredDefinition>, DefinitionStoreError>;

    /// Stores a new definition; an id already held is
    /// [`DefinitionStoreError::IdTaken`].
    ///
    /// For the tests alone: a dashboard stored this way has no owner, and
    /// the service creates every definition through [`Definitions::apply`].
    #[cfg(test)]
    async fn create(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError>;

    /// Replaces what an id holds, reporting whether there was one.
    async fn replace(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<bool, DefinitionStoreError>;

    /// Every id of this kind.
    ///
    /// For the dependency scans, which have to see the definitions a page
    /// would leave out.
    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError>;

    /// One page of the definitions of this kind whose name or body holds
    /// `needle`, ordered by name.
    ///
    /// The body too, because "which metrics read `class_git_commits`" is the
    /// question a catalogue of a few hundred definitions is actually asked,
    /// and a name cannot answer it. An empty needle is every definition of
    /// that kind.
    async fn page(
        &self,
        kind: LibraryKind,
        needle: &str,
        page: Page,
    ) -> Result<ListedPage, DefinitionStoreError>;

    /// The same page as [`Definitions::page`], each definition with its body
    /// and when it last changed, for a catalogue that draws a card of each.
    async fn page_entries(
        &self,
        kind: LibraryKind,
        needle: &str,
        page: Page,
    ) -> Result<LibraryPage, DefinitionStoreError>;

    /// The name of each of these ids that exists, by id, in one read.
    async fn names_of(
        &self,
        kind: LibraryKind,
        ids: &[&str],
    ) -> Result<HashMap<String, String>, DefinitionStoreError>;

    /// Removes the definition, reporting whether there was one.
    async fn delete(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<bool, DefinitionStoreError>;

    /// Applies every change or none of them.
    ///
    /// A chat request builds a metric, its widgets and the dashboard that
    /// holds them; written one at a time, a failure partway through left the
    /// reader a metric, no dashboard, and no way to tell.
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
