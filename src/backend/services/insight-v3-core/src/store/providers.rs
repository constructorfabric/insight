mod discord;
mod slack;
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
pub(crate) use slack::Slack;
pub(crate) use telegram::Telegram;
pub(crate) use zulip::Zulip;

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

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
                DestinationConfig::Slack { bot_token, channel } => {
                    Arc::new(Slack::new(http.clone(), bot_token.clone(), channel.clone()))
                }
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

fn client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

fn transport(error: &reqwest::Error) -> SendError {
    if error.is_timeout() {
        return SendError::Unconfirmed("timed out".to_owned());
    }

    SendError::Unconfirmed("could not be reached".to_owned())
}

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
