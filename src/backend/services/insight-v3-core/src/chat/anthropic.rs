//! Talking to the model, and reading what comes back.

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ChatError;
use super::conversation::ModelTransport;
use super::tools::{ANSWER_TOOL, CREATE_TOOL, LOOK_UP_TOOL, proposal_tools};

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const CHAT_MAX_TOKENS: u32 = 2048;

#[derive(Debug)]
pub(super) struct Anthropic<'a> {
    http: &'a reqwest::Client,
    token: &'a SecretString,
    model: &'a str,
}

impl<'a> Anthropic<'a> {
    pub(super) fn new(http: &'a reqwest::Client, token: &'a SecretString, model: &'a str) -> Self {
        Self { http, token, model }
    }
}

#[async_trait::async_trait]
impl ModelTransport for Anthropic<'_> {
    async fn send(
        &self,
        system: &str,
        messages: &[Message],
    ) -> Result<MessagesResponse, ChatError> {
        call_model(self.http, self.token, self.model, system, messages).await
    }
}

async fn call_model(
    http: &reqwest::Client,
    token: &SecretString,
    model: &str,
    system: &str,
    messages: &[Message],
) -> Result<MessagesResponse, ChatError> {
    let body = MessagesRequest {
        model,
        max_tokens: CHAT_MAX_TOKENS,
        system,
        messages,
        tools: proposal_tools(),
        tool_choice: json!({ "type": "any" }),
    };

    let response = http
        .post(format!("{ANTHROPIC_API_BASE}/v1/messages"))
        .header("x-api-key", token.expose_secret())
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|error| transport_error(&error))?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(ChatError::TokenRejected);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        tracing::warn!(status = %status, "the model call was refused upstream");
        return Err(ChatError::Unavailable);
    }
    if !status.is_success() {
        let said = response
            .text()
            .await
            .unwrap_or_else(|_| "<the body could not be read>".to_owned());
        tracing::error!(
            status = %status,
            said = %said.chars().take(400).collect::<String>(),
            "the model call failed upstream"
        );
        return Err(ChatError::Failed);
    }

    response.json().await.map_err(|error| {
        tracing::error!(error = %error, "the model answer could not be read");
        ChatError::Failed
    })
}

fn transport_error(error: &reqwest::Error) -> ChatError {
    if error.is_timeout() {
        return ChatError::Timeout;
    }
    tracing::error!(error = %error, "the model could not be reached");
    ChatError::Failed
}

#[derive(Debug, Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: &'a [Message],
    tools: Vec<Value>,
    tool_choice: Value,
}

/// One turn on the wire. `content` is a string for prose and an array of
/// content blocks when it carries a tool result, which is why it is a value
/// rather than a `&str`.
#[derive(Debug, Clone, Serialize)]
pub(super) struct Message {
    pub(super) role: &'static str,
    pub(super) content: Value,
}

impl Message {
    pub(super) fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user",
            content: Value::String(content.into()),
        }
    }

    /// The assistant's own prose, from a turn the reader's panel sent back.
    pub(super) fn said(content: impl Into<String>) -> Self {
        Self {
            role: "assistant",
            content: Value::String(content.into()),
        }
    }

    /// The assistant's own turn, echoed back verbatim.
    ///
    /// INVARIANT: the `tool_use` block it produced must precede the result we
    /// return for it.
    pub(super) fn assistant(blocks: Value) -> Self {
        Self {
            role: "assistant",
            content: blocks,
        }
    }

    /// The answer to one `tool_use`, addressed by its id.
    pub(super) fn tool_result(id: &str, content: &str) -> Self {
        Self {
            role: "user",
            content: json!([{
                "type": "tool_result",
                "tool_use_id": id,
                "content": content,
            }]),
        }
    }

    /// A refusal, addressed to the call that earned it.
    pub(super) fn tool_error(id: &str, content: &str) -> Self {
        Self {
            role: "user",
            content: json!([{
                "type": "tool_result",
                "tool_use_id": id,
                "is_error": true,
                "content": content,
            }]),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
}

impl MessagesResponse {
    /// The turn as the API needs it echoed back: a tool result must follow the
    /// assistant turn that asked for it, carrying the same `tool_use` block.
    pub(super) fn blocks(&self) -> Value {
        Value::Array(
            self.content
                .iter()
                .map(|block| {
                    if block.kind == "tool_use" {
                        json!({
                            "type": "tool_use",
                            "id": block.id,
                            "name": block.name,
                            "input": block.input.clone().unwrap_or(json!({})),
                        })
                    } else {
                        json!({ "type": "text", "text": block.text })
                    }
                })
                .collect(),
        )
    }

    /// The id of the call that ended the turn, so a refusal can be addressed
    /// to it. Absent when the model replied in prose instead of calling.
    pub(super) fn terminal_tool_id(&self) -> Option<&str> {
        self.content
            .iter()
            .find(|block| {
                block.kind == "tool_use" && (block.name == ANSWER_TOOL || block.name == CREATE_TOOL)
            })
            .map(|block| block.id.as_str())
    }

    /// The datasets this turn asks about, when it asks rather than answers.
    pub(super) fn look_up(&self) -> Option<LookUp> {
        let block = self
            .content
            .iter()
            .find(|block| block.kind == "tool_use" && block.name == LOOK_UP_TOOL)?;
        let tables = block
            .input
            .as_ref()?
            .get("datasets")?
            .as_array()?
            .iter()
            .filter_map(|name| name.as_str().map(str::to_owned))
            .collect::<Vec<_>>();

        (!tables.is_empty()).then_some(LookUp {
            id: block.id.clone(),
            tables,
        })
    }

    fn text(&self) -> String {
        self.content
            .iter()
            .filter(|block| block.kind == "text")
            .map(|block| block.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The forced tool call's arguments, tagged with the intent the chosen
    /// tool implies, in the shape `Proposal::parse` reads. Falls back to the
    /// text blocks when a reply arrives without a tool call at all.
    pub(super) fn proposal_json(&self) -> String {
        for block in &self.content {
            if block.kind != "tool_use" {
                continue;
            }
            let intent = match block.name.as_str() {
                ANSWER_TOOL => "answer",
                CREATE_TOOL => "create",
                _ => continue,
            };
            if let Some(Value::Object(fields)) = block.input.clone() {
                let mut tagged = fields;
                tagged.insert("intent".to_owned(), Value::String(intent.to_owned()));
                return Value::Object(tagged).to_string();
            }
        }

        self.text()
    }
}

/// A request for the columns of some tables.
#[derive(Debug)]
pub(super) struct LookUp {
    id: String,
    tables: Vec<String>,
}

impl LookUp {
    /// The `tool_use` id the result must be addressed to.
    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn tables(&self) -> &[String] {
        &self.tables
    }
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type", default)]
    kind: String,
    /// Present on a `tool_use`; a tool result is addressed by it.
    #[serde(default)]
    id: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    input: Option<Value>,
}
