//! What the assistant is told before it is asked anything.

use super::datasets::{self, Datasets};
use super::definition::{DefinitionKind, Definitions};
use super::kinds::dataset::declaration::Declaration;
use super::kinds::dataset::describe::{describe, describe_all};
use crate::chat::{Catalogue, Schemas};

/// Everything the model is given about this stand, gathered once per ask.
#[derive(Debug, Default)]
pub(crate) struct Briefing {
    /// The declared datasets, as a reader is told them.
    pub(crate) datasets: String,
    /// What is already built, so the model can name and replace it.
    pub(crate) catalogue: Catalogue,
    /// Every dataset a metric may read.
    pub(crate) allowed: Vec<String>,
}

/// Reads the stand for what the assistant needs to know about it.
#[derive(Debug)]
pub(crate) struct Assistant<'a> {
    definitions: &'a dyn Definitions,
    datasets: &'a dyn Datasets,
}

impl<'a> Assistant<'a> {
    pub(crate) fn new(definitions: &'a dyn Definitions, datasets: &'a dyn Datasets) -> Self {
        Self {
            definitions,
            datasets,
        }
    }

    /// Everything the model is told, read fresh: a stand gains datasets and
    /// definitions between one question and the next.
    pub(crate) async fn briefing(&self) -> Briefing {
        let declared = self.declared().await;

        Briefing {
            allowed: declared.iter().map(|(name, _)| name.clone()).collect(),
            datasets: describe_all(&declared),
            catalogue: self.catalogue().await,
        }
    }

    /// Every dataset that is ready to be read, with its declaration.
    ///
    /// Only the ready ones: an unfinished create is never described, so
    /// nothing is offered that cannot be queried. A listing failure leaves the
    /// model with no datasets rather than failing the ask.
    async fn declared(&self) -> Vec<(String, Declaration)> {
        let names = match self.datasets.list().await {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(error = ?error, "could not list the datasets for the chat");
                return Vec::new();
            }
        };

        let mut declared = Vec::with_capacity(names.len());
        for name in names {
            if let Some(held) = self.ready(&name).await {
                declared.push((name, held));
            }
        }

        declared
    }

    async fn ready(&self, name: &str) -> Option<Declaration> {
        Some(datasets::ready(self.datasets, name).await?.declaration)
    }

    /// What is already stored, so the model can name it, reuse it, and replace it
    /// when the reader asks for a change. A listing failure degrades the hint; it
    /// does not fail the chat.
    async fn catalogue(&self) -> Catalogue {
        let mut built = Vec::with_capacity(DefinitionKind::ALL.len());

        for kind in DefinitionKind::ALL {
            built.push((kind, self.names(kind).await));
        }

        Catalogue::new(built)
    }

    async fn names(&self, kind: DefinitionKind) -> Vec<String> {
        match self.definitions.list(kind).await {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(error = ?error, ?kind, "could not list definitions for the chat");
                Vec::new()
            }
        }
    }
}

/// The declarations of the datasets the model asked about.
#[derive(Debug)]
pub(crate) struct DatasetSchemas<'a> {
    datasets: &'a dyn Datasets,
    definitions: &'a dyn Definitions,
}

impl<'a> DatasetSchemas<'a> {
    pub(crate) fn new(datasets: &'a dyn Datasets, definitions: &'a dyn Definitions) -> Self {
        Self {
            datasets,
            definitions,
        }
    }
}

#[async_trait::async_trait]
impl Schemas for DatasetSchemas<'_> {
    async fn describe(&self, datasets: &[String]) -> String {
        let assistant = Assistant::new(self.definitions, self.datasets);
        let mut written = String::new();

        for name in datasets {
            // A name that resolved to nothing is said so rather than left
            // out: silence reads as "no fields" and the model invents them.
            if let Some(declaration) = assistant.ready(name).await {
                written.push_str(&describe(name, &declaration));
            } else {
                written.push_str(name);
                written.push_str(": no dataset of that name is ready\n");
            }
        }

        written
    }
}
