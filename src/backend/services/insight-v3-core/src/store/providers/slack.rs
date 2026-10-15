//! Slack, through a bot's `chat.postMessage`. The answer carries the message
//! timestamp, which is the receipt.

use async_trait::async_trait;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::{body, status, transport};
use crate::domain::alerts::delivery::{Message, Provider, Receipt, SendError};

const API: &str = "https://slack.com/api";

// Errors Slack answers inside a successful response that a later attempt
// may get past: a pause it asked for, a failure on its side, a request that
// arrived cut short, or a workspace mid-migration.
const UNCONFIRMED_ERRORS: [&str; 9] = [
    "ratelimited",
    "rate_limited",
    "message_limit_exceeded",
    "internal_error",
    "fatal_error",
    "service_unavailable",
    "request_timeout",
    "team_added_to_org",
    "org_login_required",
];

pub(crate) struct Slack {
    http: reqwest::Client,
    bot_token: SecretString,
    channel: String,
    api: String,
}

impl Slack {
    pub(crate) fn new(http: reqwest::Client, bot_token: SecretString, channel: String) -> Self {
        Self {
            http,
            bot_token,
            channel,
            api: API.to_owned(),
        }
    }

    #[cfg(test)]
    pub(crate) fn at(mut self, api: &str) -> Self {
        self.api = api.trim_end_matches('/').to_owned();
        self
    }
}

#[derive(Debug, Deserialize)]
struct Answer {
    ok: bool,
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

// Slack reads `&`, `<` and `>` as control characters in message text, inside
// code spans as well; anything else is written as is.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[async_trait]
impl Provider for Slack {
    async fn send(&self, message: &Message) -> Result<Receipt, SendError> {
        let response = self
            .http
            .post(format!("{}/chat.postMessage", self.api))
            .bearer_auth(self.bot_token.expose_secret())
            .json(&serde_json::json!({
                "channel": self.channel,
                "text": escaped(&message.markdown),
                "mrkdwn": true,
                "unfurl_links": false,
                "unfurl_media": false,
            }))
            .send()
            .await
            .map_err(|error| transport(&error))?;
        status(&response)?;

        let answer: Answer = serde_json::from_slice(&body(response).await?)
            .map_err(|_| SendError::Unconfirmed("answer not understood".to_owned()))?;
        match (answer.ok, answer.ts, answer.error) {
            (true, Some(ts), _) => Ok(Receipt(ts)),
            (true, None, _) => Err(SendError::Unconfirmed("answer not understood".to_owned())),
            (false, _, Some(error)) if UNCONFIRMED_ERRORS.contains(&error.as_str()) => {
                Err(SendError::Unconfirmed(error))
            }
            (false, _, error) => Err(SendError::Rejected(
                error.unwrap_or_else(|| "not ok".to_owned()),
            )),
        }
    }
}

impl std::fmt::Debug for Slack {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Slack")
            .field("channel", &self.channel)
            .finish_non_exhaustive()
    }
}
