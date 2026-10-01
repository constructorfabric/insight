//! Zulip, through a bot posting to a stream topic. The answer carries the
//! message id, which is the receipt.

use async_trait::async_trait;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::{body, status, transport};
use crate::domain::alerts::delivery::{Message, Provider, Receipt, SendError};

pub(crate) struct Zulip {
    http: reqwest::Client,
    site_url: String,
    bot_email: String,
    api_key: SecretString,
    stream: String,
    topic: String,
}

impl Zulip {
    pub(crate) fn new(
        http: reqwest::Client,
        site_url: &str,
        bot_email: String,
        api_key: SecretString,
        stream: String,
        topic: String,
    ) -> Self {
        Self {
            http,
            site_url: site_url.trim_end_matches('/').to_owned(),
            bot_email,
            api_key,
            stream,
            topic,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Answer {
    result: String,
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    msg: Option<String>,
}

#[async_trait]
impl Provider for Zulip {
    async fn send(&self, message: &Message) -> Result<Receipt, SendError> {
        let response = self
            .http
            .post(format!("{}/api/v1/messages", self.site_url))
            .basic_auth(&self.bot_email, Some(self.api_key.expose_secret()))
            .form(&[
                ("type", "stream"),
                ("to", self.stream.as_str()),
                ("topic", self.topic.as_str()),
                ("content", message.text.as_str()),
            ])
            .send()
            .await
            .map_err(|error| transport(&error))?;
        status(&response)?;

        let answer: Answer = serde_json::from_slice(&body(response).await?)
            .map_err(|_| SendError::Unconfirmed("answer not understood".to_owned()))?;
        match (answer.result.as_str(), answer.id) {
            ("success", Some(id)) => Ok(Receipt(id.to_string())),
            _ => Err(SendError::Rejected(
                answer.msg.unwrap_or_else(|| "not success".to_owned()),
            )),
        }
    }
}

impl std::fmt::Debug for Zulip {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Zulip")
            .field("site_url", &self.site_url)
            .field("stream", &self.stream)
            .field("topic", &self.topic)
            .finish_non_exhaustive()
    }
}
