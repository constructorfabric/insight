//! The chat client: turns a message into either a one-time answer or a set
//! of metric/widget/dashboard definitions to store.

mod anthropic;
mod conversation;
mod prompt;
mod proposal;
mod tools;

use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;
use utoipa::ToSchema;

use anthropic::Anthropic;
use conversation::{converse, thread};
use prompt::system_prompt;
pub(crate) use proposal::Proposal;

use crate::domain::definition::DefinitionKind;
use crate::domain::query::metric_query::{MetricQueryError, People};

const CHAT_TIMEOUT_SECS: u64 = 30;

/// One turn of the conversation so far. The reader's panel keeps the thread
/// and sends it back, because the service stores no session.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(crate) struct Turn {
    /// `user` or `assistant`; anything else is dropped before the call.
    pub(crate) role: String,
    pub(crate) content: String,
}

/// What is already stored, so the model can name it, reuse it and replace it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Catalogue {
    built: Vec<(DefinitionKind, Vec<String>)>,
}

impl Catalogue {
    pub(crate) fn new(built: Vec<(DefinitionKind, Vec<String>)>) -> Self {
        Self { built }
    }

    fn is_empty(&self) -> bool {
        self.built.iter().all(|(_, names)| names.is_empty())
    }

    fn built(&self) -> &[(DefinitionKind, Vec<String>)] {
        &self.built
    }
}

/// One question, and everything the model needs to answer it.
#[derive(Debug)]
pub(crate) struct Ask<'a> {
    pub(crate) message: &'a str,
    /// The turns before this one; the service keeps no session.
    pub(crate) turns: &'a [Turn],
    /// The declared datasets, as a reader is told them.
    pub(crate) datasets: &'a str,
    pub(crate) catalogue: &'a Catalogue,
    /// Every dataset a metric may read.
    pub(crate) allowed: &'a [String],
    pub(crate) schemas: &'a dyn Schemas,
    /// Where a person's name is resolved from.
    pub(crate) people: &'a People,
}

/// What a dataset declares, asked for by name.
///
/// The prompt already names every dataset and its fields; this is what the
/// model asks when it wants one spelled out again mid-conversation.
#[async_trait::async_trait]
pub(crate) trait Schemas: Send + Sync + std::fmt::Debug {
    /// The named datasets, rendered for the model. A name it cannot resolve is
    /// reported as such rather than omitted, or the model reads silence as
    /// "no fields" and invents them.
    async fn describe(&self, datasets: &[String]) -> String;
}

#[derive(Debug, Error)]
pub(crate) enum ChatError {
    #[error("the model reply was not valid JSON")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Metric(#[from] MetricQueryError),
    #[error("there is no dataset named `{dataset}`; the datasets are: {known}")]
    UnknownDataset { dataset: String, known: String },
    #[error("a metric reads one of the datasets; the datasets are: {known}")]
    NoDataset { known: String },
    #[error("a create must carry at least one metric, widget or dashboard")]
    EmptyCreate,
    #[error("the model kept asking what the datasets declare instead of answering")]
    TooManyLookups,
    #[error("the key was rejected upstream")]
    TokenRejected,
    #[error("the model is unavailable right now")]
    Unavailable,
    #[error("the model did not answer in time")]
    Timeout,
    #[error("the model call failed")]
    Failed,
    #[error("this instance has no Anthropic key, so the assistant cannot answer")]
    NoKey,
}

impl ChatError {
    /// What the repair round tells the model. `Display` on the JSON variant
    /// names a category; the serde message names the offending field.
    pub(super) fn feedback(&self) -> String {
        match self {
            Self::Json(error) => error.to_string(),
            Self::Metric(_)
            | Self::UnknownDataset { .. }
            | Self::NoDataset { .. }
            | Self::EmptyCreate
            | Self::TooManyLookups
            | Self::TokenRejected
            | Self::Unavailable
            | Self::Timeout
            | Self::Failed
            | Self::NoKey => self.to_string(),
        }
    }
}

/// Proposes an answer or a creation from a chat message.
///
/// There is one backend that answers: the model. A blank key gives
/// [`ChatError::NoKey`] rather than a reply of our own invention, so a stand
/// that never got a key says so instead of looking answered.
#[derive(Debug, Clone)]
pub(crate) struct ChatClient {
    backend: ChatBackend,
}

#[derive(Debug, Clone)]
enum ChatBackend {
    Live {
        http: reqwest::Client,
        token: SecretString,
        model: String,
    },
    Keyless,
    #[cfg(test)]
    Scripted(fn() -> Proposal),
}

impl ChatClient {
    pub(crate) fn new(token: &SecretString, model: String) -> Self {
        if token.expose_secret().trim().is_empty() {
            return Self::keyless();
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(CHAT_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|error| panic!("the HTTP client must build: {error}"));

        Self {
            backend: ChatBackend::Live {
                http,
                token: token.clone(),
                model,
            },
        }
    }

    /// A client with nothing to call: every ask is [`ChatError::NoKey`].
    pub(crate) fn keyless() -> Self {
        Self {
            backend: ChatBackend::Keyless,
        }
    }

    /// Always answers with a fixed [`Proposal`] built by `build`, making no
    /// network call — lets a test drive `handle_chat` with a proposal shape
    /// of its own choosing.
    #[cfg(test)]
    pub(crate) fn scripted(build: fn() -> Proposal) -> Self {
        Self {
            backend: ChatBackend::Scripted(build),
        }
    }

    /// # Errors
    ///
    /// Returns [`ChatError`] describing what the upstream did, or why the
    /// reply could not be turned into a [`Proposal`].
    pub(crate) async fn propose(&self, ask: &Ask<'_>) -> Result<Proposal, ChatError> {
        match &self.backend {
            ChatBackend::Keyless => Err(ChatError::NoKey),
            #[cfg(test)]
            ChatBackend::Scripted(build) => Ok(build()),
            ChatBackend::Live { http, token, model } => {
                let transport = Anthropic::new(http, token, model);
                converse(
                    &transport,
                    ask.schemas,
                    &system_prompt(ask.datasets, ask.catalogue),
                    thread(ask.turns, ask.message),
                    ask.allowed,
                    ask.people,
                )
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests;
