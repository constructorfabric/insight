//! The providers a notification can be handed to, one HTTP call each.
//!
//! Every adapter answers the same three ways: a receipt when the provider
//! confirmed the message, a rejection it will keep giving, or an outcome
//! that is not known — a timeout, a rate limit, a server error — which a
//! later attempt may repeat and which may therefore land twice.

mod discord;
mod telegram;
mod zulip;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crate::config::DestinationConfig;
use crate::domain::alerts::delivery::{Provider, SendError};

pub(crate) use discord::Discord;
pub(crate) use telegram::Telegram;
pub(crate) use zulip::Zulip;

/// The most of a provider's answer that is read into memory.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Every configured destination as the adapter that reaches it.
pub(crate) fn providers(
    destinations: &BTreeMap<String, DestinationConfig>,
    timeout: Duration,
) -> Result<BTreeMap<String, Arc<dyn Provider>>, reqwest::Error> {
    let http = client(timeout)?;

    Ok(destinations
        .iter()
        .map(|(name, destination)| {
            let provider: Arc<dyn Provider> = match destination {
                DestinationConfig::Discord { webhook_url } => {
                    Arc::new(Discord::new(http.clone(), webhook_url.clone()))
                }
                DestinationConfig::Telegram { bot_token, chat_id } => Arc::new(Telegram::new(
                    http.clone(),
                    bot_token.clone(),
                    chat_id.clone(),
                )),
                DestinationConfig::Zulip {
                    site_url,
                    bot_email,
                    api_key,
                    stream,
                    topic,
                } => Arc::new(Zulip::new(
                    http.clone(),
                    site_url,
                    bot_email.clone(),
                    api_key.clone(),
                    stream.clone(),
                    topic.clone(),
                )),
            };

            (name.clone(), provider)
        })
        .collect())
}

/// The one client every adapter posts through: a bounded wait, and no
/// following of redirects, since what a redirect's target answers is not
/// the provider's answer to the message.
fn client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

/// What a transport failure means: a timeout or a dropped connection
/// leaves the outcome unknown.
fn transport(error: &reqwest::Error) -> SendError {
    if error.is_timeout() {
        return SendError::Unconfirmed("timed out".to_owned());
    }

    SendError::Unconfirmed("could not be reached".to_owned())
}

/// What a status the provider chose means. Rate limits and server errors
/// are unknown outcomes; everything else that is not success, a redirect
/// included, is final.
fn status(response: &reqwest::Response) -> Result<(), SendError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return Err(SendError::Unconfirmed(format!("answered {status}")));
    }

    Err(SendError::Rejected(format!("answered {status}")))
}

fn too_large() -> SendError {
    SendError::Unconfirmed("answer too large".to_owned())
}

/// The body, read a chunk at a time and given up on as soon as it passes
/// the bound, or the reason it could not be read.
async fn body(mut response: reqwest::Response) -> Result<Vec<u8>, SendError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(too_large());
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| transport(&error))? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(bytes)
}
