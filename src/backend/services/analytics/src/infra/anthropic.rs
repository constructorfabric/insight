//! Outbound client for the Anthropic Messages API.
//!
//! The caller's own key pays for the call, so it arrives per request and is
//! never held on the client. Upstream failures are mapped to a small typed set
//! here; the API layer decides what each one means on the wire.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Wire version the Messages API requires on every request.
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, thiserror::Error)]
pub enum AnthropicError {
    /// Anthropic refused the key.
    #[error("the key was rejected upstream")]
    TokenRejected,
    /// Rate limited, overloaded, or a 5xx.
    #[error("the model is unavailable right now")]
    Unavailable,
    /// The request never completed in time.
    #[error("the model did not answer in time")]
    Timeout,
    /// Anything else — a malformed body, a transport failure.
    #[error("the model call failed")]
    Failed,
}

/// What one answer cost to produce, as the vendor reported it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct Answer {
    pub text: String,
    pub usage: Usage,
}

#[derive(Debug, Clone)]
pub struct AnthropicClient {
    http: reqwest::Client,
    api_base: String,
}

impl AnthropicClient {
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be built.
    pub fn new(api_base: &str, timeout: Duration) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().timeout(timeout).build()?;

        Ok(Self {
            http,
            api_base: api_base.trim_end_matches('/').to_owned(),
        })
    }

    /// Ask for one answer. `token` is used and dropped.
    ///
    /// # Errors
    ///
    /// Returns [`AnthropicError`] describing what the upstream did.
    pub async fn message(
        &self,
        token: &Zeroizing<String>,
        model: &str,
        max_tokens: u32,
        system: &str,
        user: &str,
    ) -> Result<Answer, AnthropicError> {
        let body = MessagesRequest {
            model,
            max_tokens,
            system,
            messages: vec![Message {
                role: "user",
                content: user,
            }],
        };

        let response = self
            .http
            .post(format!("{}/v1/messages", self.api_base))
            .header("x-api-key", token.as_str())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|error| transport_error(&error))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(AnthropicError::TokenRejected);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            tracing::warn!(status = %status, "the model call was refused upstream");
            return Err(AnthropicError::Unavailable);
        }
        if !status.is_success() {
            tracing::error!(status = %status, "the model call failed upstream");
            return Err(AnthropicError::Failed);
        }

        let parsed: MessagesResponse = response.json().await.map_err(|error| {
            tracing::error!(error = %error, "the model answer could not be read");
            AnthropicError::Failed
        })?;

        Ok(Answer {
            text: parsed.text(),
            usage: Usage {
                input_tokens: parsed.usage.input_tokens,
                output_tokens: parsed.usage.output_tokens,
            },
        })
    }
}

fn transport_error(error: &reqwest::Error) -> AnthropicError {
    if error.is_timeout() {
        return AnthropicError::Timeout;
    }
    tracing::error!(error = %error, "the model could not be reached");
    AnthropicError::Failed
}

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<Message<'a>>,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: UsageBody,
}

impl MessagesResponse {
    fn text(&self) -> String {
        self.content
            .iter()
            .filter(|block| block.kind == "text")
            .map(|block| block.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Default, Deserialize)]
struct UsageBody {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_joins_every_text_block_and_skips_the_rest() -> Result<(), serde_json::Error> {
        let parsed: MessagesResponse = serde_json::from_value(serde_json::json!({
            "content": [
                { "type": "text", "text": "first" },
                { "type": "thinking", "text": "hidden" },
                { "type": "text", "text": "second" },
            ],
            "usage": { "input_tokens": 12, "output_tokens": 7 },
        }))?;

        assert_eq!(parsed.text(), "first\n\nsecond");
        assert_eq!(parsed.usage.output_tokens, 7);
        Ok(())
    }

    #[test]
    fn an_answer_with_no_content_is_empty_rather_than_an_error() -> Result<(), serde_json::Error> {
        let parsed: MessagesResponse = serde_json::from_value(serde_json::json!({}))?;

        assert_eq!(parsed.text(), "");
        Ok(())
    }
}
