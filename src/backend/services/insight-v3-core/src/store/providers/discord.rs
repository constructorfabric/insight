//! Discord, through an incoming webhook. `wait=true` makes the webhook
//! answer with the message it created rather than 204, which is the receipt.

use async_trait::async_trait;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::{body, status, transport};
use crate::domain::alerts::delivery::{Message, Provider, Receipt, SendError};

pub(crate) struct Discord {
    http: reqwest::Client,
    webhook_url: SecretString,
}

impl Discord {
    pub(crate) fn new(http: reqwest::Client, webhook_url: SecretString) -> Self {
        Self { http, webhook_url }
    }
}

#[derive(Debug, Deserialize)]
struct Created {
    id: String,
}

#[async_trait]
impl Provider for Discord {
    async fn send(&self, message: &Message) -> Result<Receipt, SendError> {
        let response = self
            .http
            .post(self.webhook_url.expose_secret())
            .query(&[("wait", "true")])
            .json(&serde_json::json!({
                "content": message.text,
                "allowed_mentions": { "parse": [] },
            }))
            .send()
            .await
            .map_err(|error| transport(&error))?;
        status(&response)?;

        let created: Created = serde_json::from_slice(&body(response).await?)
            .map_err(|_| SendError::Unconfirmed("answer not understood".to_owned()))?;

        Ok(Receipt(created.id))
    }
}

impl std::fmt::Debug for Discord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Discord").finish_non_exhaustive()
    }
}
