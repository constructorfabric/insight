//! Telegram, through a bot's `sendMessage`. The answer carries the message
//! id, which is the receipt.

use async_trait::async_trait;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::{body, status, transport};
use crate::domain::alerts::delivery::{Message, Provider, Receipt, SendError};

const API: &str = "https://api.telegram.org";

pub(crate) struct Telegram {
    http: reqwest::Client,
    bot_token: SecretString,
    chat_id: String,
    api: String,
}

impl Telegram {
    pub(crate) fn new(http: reqwest::Client, bot_token: SecretString, chat_id: String) -> Self {
        Self {
            http,
            bot_token,
            chat_id,
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
    result: Option<Sent>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Sent {
    message_id: i64,
}

#[async_trait]
impl Provider for Telegram {
    async fn send(&self, message: &Message) -> Result<Receipt, SendError> {
        let url = format!(
            "{}/bot{}/sendMessage",
            self.api,
            self.bot_token.expose_secret()
        );
        let response = self
            .http
            .post(url)
            .json(&serde_json::json!({
                "chat_id": self.chat_id,
                "text": message.text,
                "disable_web_page_preview": true,
            }))
            .send()
            .await
            .map_err(|error| transport(&error))?;
        status(&response)?;

        let answer: Answer = serde_json::from_slice(&body(response).await?)
            .map_err(|_| SendError::Unconfirmed("answer not understood".to_owned()))?;
        match (answer.ok, answer.result) {
            (true, Some(sent)) => Ok(Receipt(sent.message_id.to_string())),
            _ => Err(SendError::Rejected(
                answer.description.unwrap_or_else(|| "not ok".to_owned()),
            )),
        }
    }
}

impl std::fmt::Debug for Telegram {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Telegram")
            .field("chat_id", &self.chat_id)
            .finish_non_exhaustive()
    }
}
