//! The turn: how many round trips the model gets, and what ends one.

use super::anthropic::{Message, MessagesResponse};
use super::proposal::Proposal;
use super::{ChatError, Schemas, Turn};
use crate::domain::query::metric_query::People;

/// How many times one message may ask what a table holds before answering.
/// Three is room to look at a handful of tables across two or three layers;
/// past that the model is circling rather than converging.
const MAX_LOOKUPS: usize = 3;

/// One round trip to the model.
#[async_trait::async_trait]
pub(super) trait ModelTransport: Send + Sync {
    async fn send(&self, system: &str, messages: &[Message])
    -> Result<MessagesResponse, ChatError>;
}

/// The conversation as the API takes it: the turns so far, then the new
/// message. A turn with any other role is dropped rather than trusted.
pub(super) fn thread(turns: &[Turn], message: &str) -> Vec<Message> {
    let mut messages: Vec<Message> = turns
        .iter()
        .filter_map(|turn| match turn.role.as_str() {
            "user" => Some(Message::user(turn.content.clone())),
            "assistant" => Some(Message::said(turn.content.clone())),
            _ => None,
        })
        .collect();

    messages.push(Message::user(message));

    messages
}

/// The turn, which may take several round trips.
///
/// The model sees the map of every table but not their columns, so it may ask
/// what a handful of them hold before it answers. Each answer is fed back as a
/// tool result and the conversation continues; the reply that is not a lookup
/// ends it. A reply this service refuses buys one repair round: the schema
/// stops malformed arguments, this catches what only our own validation knows.
pub(super) async fn converse(
    transport: &dyn ModelTransport,
    schemas: &dyn Schemas,
    system: &str,
    mut messages: Vec<Message>,
    allowed: &[String],
    people: &People,
) -> Result<Proposal, ChatError> {
    for _ in 0..=MAX_LOOKUPS {
        let response = transport.send(system, &messages).await?;

        if let Some(look_up) = response.look_up() {
            tracing::info!(datasets = ?look_up.tables(), "the model asked what these datasets declare");
            let described = schemas.describe(look_up.tables()).await;
            messages.push(Message::assistant(response.blocks()));
            messages.push(Message::tool_result(look_up.id(), &described));
            continue;
        }

        return match Proposal::checked(&response.proposal_json(), allowed, people) {
            Ok(proposal) => Ok(proposal),

            Err(rejection) => {
                let detail = rejection.feedback();
                tracing::info!(rejection = %detail, "asking the model to correct its proposal");

                messages.push(Message::assistant(response.blocks()));
                messages.push(refusal(response.terminal_tool_id(), &detail));

                let second = transport.send(system, &messages).await?;

                Proposal::checked(&second.proposal_json(), allowed, people)
            }
        };
    }

    Err(ChatError::TooManyLookups)
}

/// The rejection, addressed to the call that earned it when there was one.
///
/// INVARIANT: the API requires every `tool_use` to be answered by a
/// `tool_result`; following one with a plain message is a 400.
fn refusal(called: Option<&str>, detail: &str) -> Message {
    let correction = format!("That proposal was rejected: {detail}\nReturn a corrected proposal.");

    match called {
        Some(id) => Message::tool_error(id, &correction),
        None => Message::user(correction),
    }
}
