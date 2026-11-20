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
pub(crate) use proposal::{Proposal, Proposed};

use crate::domain::definition::{DefinitionKind, Listed};
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

#[derive(Debug, Clone, Default)]
pub(crate) struct Catalogue {
    built: Vec<(DefinitionKind, Vec<Listed>)>,
}

impl Catalogue {
    pub(crate) fn new(built: Vec<(DefinitionKind, Vec<Listed>)>) -> Self {
        Self { built }
    }

    fn is_empty(&self) -> bool {
        self.built.iter().all(|(_, listed)| listed.is_empty())
    }

    fn built(&self) -> &[(DefinitionKind, Vec<Listed>)] {
        &self.built
    }
}

#[derive(Debug)]
pub(crate) struct Ask<'a> {
    pub(crate) message: &'a str,
    pub(crate) turns: &'a [Turn],
    pub(crate) datasets: &'a str,
    pub(crate) catalogue: &'a Catalogue,
    pub(crate) allowed: &'a [String],
    pub(crate) schemas: &'a dyn Schemas,
    pub(crate) people: &'a People,
}

#[async_trait::async_trait]
pub(crate) trait Schemas: Send + Sync + std::fmt::Debug {
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
    #[error("two {kind} in one create are both named `{name}`; give each its own name")]
    SharedName { kind: &'static str, name: String },
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
    pub(super) fn feedback(&self) -> String {
        match self {
            Self::Json(error) => error.to_string(),
            Self::Metric(_)
            | Self::UnknownDataset { .. }
            | Self::NoDataset { .. }
            | Self::EmptyCreate
            | Self::SharedName { .. }
            | Self::TooManyLookups
            | Self::TokenRejected
            | Self::Unavailable
            | Self::Timeout
            | Self::Failed
            | Self::NoKey => self.to_string(),
        }
    }
}

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

    pub(crate) fn keyless() -> Self {
        Self {
            backend: ChatBackend::Keyless,
        }
    }

    #[cfg(test)]
    pub(crate) fn scripted(build: fn() -> Proposal) -> Self {
        Self {
            backend: ChatBackend::Scripted(build),
        }
    }

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
