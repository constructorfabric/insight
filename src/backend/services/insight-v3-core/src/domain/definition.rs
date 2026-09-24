//! Metric, widget and dashboard definitions.
//!
//! These live in `MariaDB`, not in `ClickHouse` with the data they describe.
//! They are a few hundred rows read by name and edited in place by whoever is
//! asking the assistant for a change — which is what a row store is for, and
//! what a column store is not. On `ClickHouse` a change meant inserting a new
//! version and reading with `FINAL`, nothing stopped two rows claiming one
//! name, and a create that failed halfway left what it had already written
//! behind with nothing to roll it back with.

pub(crate) mod arriving;

use std::fmt;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use thiserror::Error;

const MAX_NAME_CHARS: usize = 128;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionName(String);

impl DefinitionName {
    /// The same rule as [`DefinitionName::parse`], for a caller that can only
    /// be given a pattern - the chat tool schema the model answers in.
    pub(crate) const PATTERN: &'static str = "^[A-Za-z0-9_-]{1,128}$";

    pub(crate) fn parse(value: &str) -> Result<Self, DefinitionError> {
        if value.is_empty() || value.chars().count() > MAX_NAME_CHARS {
            return Err(DefinitionError::Name);
        }

        if !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(DefinitionError::Name);
        }

        Ok(Self(value.to_owned()))
    }

    pub(crate) fn into_string(self) -> String {
        self.0
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

/// One page of a catalogue, and how many definitions it is a page of.
#[derive(Debug)]
pub(crate) struct NamePage {
    pub(crate) names: Vec<String>,
    /// Every match, not the page — so a reader can say what is behind it.
    pub(crate) total: u64,
}

/// One write in a batch.
///
/// A rename is a put under the new name, a delete of the old one, and a put
/// per dependent that pointed at it - which is only a rename if all of them
/// land together.
#[derive(Debug)]
pub(crate) enum Change {
    Put(DefinitionKind, DefinitionName, serde_json::Value),
    /// A write that refuses to replace anything.
    ///
    /// A rename gives a definition a name nobody else holds. Asking first and
    /// upserting after leaves a window where another writer takes that name in
    /// between and the rename overwrites them; the store refuses instead.
    Create(DefinitionKind, DefinitionName, serde_json::Value),
    Delete(DefinitionKind, DefinitionName),
    CarryFolder {
        from: DefinitionName,
        to: DefinitionName,
    },
}

/// A definition as it stands: what kind it is, what it is called, and what it
/// holds.
#[derive(Debug, Clone)]
pub(crate) struct Definition {
    pub(crate) kind: DefinitionKind,
    pub(crate) name: DefinitionName,
    pub(crate) body: serde_json::Value,
}

impl Definition {
    pub(crate) fn new(kind: DefinitionKind, name: DefinitionName, body: serde_json::Value) -> Self {
        Self { kind, name, body }
    }
}

/// Reading one definition by name, which is all a kind's own rules may do.
///
/// A rule that could write would write mid-check, before the batch it belongs
/// to has been accepted.
#[async_trait]
pub(crate) trait Lookup: Send + Sync + fmt::Debug {
    async fn get(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError>;
}

/// What the API and the chat need of the store, so neither has to know where
/// definitions live — and so their tests can hold them in a map rather than
/// answer a database's wire protocol.
#[async_trait]
pub(crate) trait Definitions: Lookup {
    /// Stores `body` under `name`, replacing whatever that name held.
    async fn put(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError>;

    /// Every name of this kind.
    ///
    /// For the dependency scans, which have to see the definitions a page
    /// would leave out.
    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError>;

    /// One page of the names of this kind whose name or body holds `needle`.
    ///
    /// The body too, because "which metrics read `class_git_commits`" is the
    /// question a catalogue of a few hundred definitions is actually asked,
    /// and a name cannot answer it. An empty needle is every definition of
    /// that kind.
    async fn page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<NamePage, DefinitionStoreError>;

    /// Removes the definition, reporting whether there was one.
    async fn delete(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
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
    #[error("definition names use letters, digits, underscore and dash, up to 128 characters")]
    Name,
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
    NameTaken(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests;
