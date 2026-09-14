//! The chat client: turns a message into either a one-time answer or a set
//! of metric/widget/dashboard definitions to store.

use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use utoipa::ToSchema;

use crate::metric_query::{MetricQuery, MetricQueryError, People};

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const CHAT_TIMEOUT_SECS: u64 = 30;
const CHAT_MAX_TOKENS: u32 = 2048;
const ANSWER_TOOL: &str = "answer";
const LOOK_UP_TOOL: &str = "look_up";
/// How many times one message may ask what a table holds before answering.
/// Three is room to look at a handful of tables across two or three layers;
/// past that the model is circling rather than converging.
const MAX_LOOKUPS: usize = 3;
const CREATE_TOOL: &str = "create";
/// Definition names: what `DefinitionName::parse` accepts.
const NAME_PATTERN: &str = "^[A-Za-z0-9_-]{1,128}$";

/// One turn of the conversation so far. The reader's panel keeps the thread
/// and sends it back, because the service stores no session.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(crate) struct Turn {
    /// `user` or `assistant`; anything else is dropped before the call.
    pub(crate) role: String,
    pub(crate) content: String,
}

impl Turn {
    #[cfg(test)]
    fn user(content: &str) -> Self {
        Self {
            role: "user".to_owned(),
            content: content.to_owned(),
        }
    }

    #[cfg(test)]
    fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".to_owned(),
            content: content.to_owned(),
        }
    }
}

/// What is already stored, so the model can name it, reuse it and replace it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Catalogue {
    pub(crate) metrics: Vec<String>,
    pub(crate) widgets: Vec<String>,
    pub(crate) dashboards: Vec<String>,
}

impl Catalogue {
    fn is_empty(&self) -> bool {
        self.metrics.is_empty() && self.widgets.is_empty() && self.dashboards.is_empty()
    }
}

/// One question, and everything the model needs to answer it.
///
/// A struct rather than eight arguments: what the model is told has grown from
/// the message alone to the thread, the stand's map, what is already built,
/// and what it may query.
pub(crate) struct Ask<'a> {
    pub(crate) message: &'a str,
    /// The turns before this one; the service keeps no session.
    pub(crate) turns: &'a [Turn],
    /// The tables ingested here, with the fields sampled from their payloads.
    pub(crate) tables: &'a [KnownTable],
    pub(crate) catalogue: &'a Catalogue,
    /// Every table on the stand by layer, names only.
    pub(crate) map: &'a str,
    /// Every table a query may name, qualified as it must be named.
    pub(crate) allowed: &'a [String],
    pub(crate) schemas: &'a dyn Schemas,
    /// Where a person's name is resolved from.
    pub(crate) people: &'a People,
}

/// What the columns of a table are, asked for by name.
///
/// The map in the system prompt names every table on the stand - a few
/// hundred - but their columns run to tens of thousands of tokens and would
/// go stale, so the model asks for the few it needs.
#[async_trait::async_trait]
pub(crate) trait Schemas: Send + Sync {
    /// The named tables, rendered for the model. A name it cannot resolve is
    /// reported as such rather than omitted, or the model reads silence as
    /// "no columns" and invents them.
    async fn describe(&self, tables: &[String]) -> String;
}

/// A table data has been ingested into, and what is in it.
#[derive(Debug, Clone)]
pub(crate) struct KnownTable {
    pub(crate) name: String,
    /// `day (string), lines (int)`, sampled from its rows.
    pub(crate) fields: String,
}

/// One of the two things the model can propose in reply to a chat message.
#[derive(Debug)]
pub(crate) enum Proposal {
    /// A one-time question. The service runs `query` when there is one and
    /// answers; nothing is stored. A question about what data exists needs no
    /// query, and forcing one got an invented query and a junk table with it.
    Answer {
        reply: String,
        query: Option<MetricQuery>,
    },
    /// A metric/widget/dashboard to store. Any of the three may be absent.
    Create {
        reply: String,
        metric: Option<(String, Value)>,
        widgets: Vec<(String, Value)>,
        dashboard: Option<(String, Value)>,
    },
}

impl Proposal {
    /// [`Proposal::parse`], then refuse a table the reader does not have.
    ///
    /// Asked what data exists, the model reaches for `information_schema` and
    /// friends; the charset check passes such a name and the query then fails
    /// in the database, which surfaced as an internal error. The refusal goes
    /// back through the repair round, so the model gets the real table list.
    /// With no known tables at all the check stands aside — refusing
    /// everything would be worse than the guess.
    pub(crate) fn checked(
        reply: &str,
        allowed: &[String],
        people: &People,
    ) -> Result<Self, ChatError> {
        let proposal = Self::parse(reply, people)?;

        if allowed.is_empty() {
            return Ok(proposal);
        }

        match proposal.table() {
            Some(named) if !allowed.contains(&named) => {
                Err(ChatError::UnknownTable {
                    table: named,
                    // Naming a few is enough to redirect the model; the whole
                    // stand is already in the prompt, and hundreds of names
                    // in an error help nobody.
                    known: allowed
                        .iter()
                        .take(12)
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                })
            }
            _ => Ok(proposal),
        }
    }

    /// The table this proposal reads, qualified by its database when it names
    /// one, so `silver.class_git_commits` is told apart from a table of the
    /// same name in another layer.
    fn table(&self) -> Option<String> {
        let (database, table) = match self {
            Self::Answer { query, .. } => {
                let query = query.as_ref()?;
                (query.database(), query.table())
            }
            Self::Create { metric, .. } => {
                let (_, body) = metric.as_ref()?;
                (
                    body.get("database").and_then(Value::as_str),
                    body.get("table").and_then(Value::as_str)?,
                )
            }
        };

        Some(match database {
            Some(database) => format!("{database}.{table}"),
            None => table.to_owned(),
        })
    }

    /// Strips any prose or code fence around the JSON object, deserializes on
    /// `intent`, and compiles every query and every proposed metric with
    /// [`MetricQuery::compile`] — a refusal is [`ChatError::Metric`], and
    /// nothing runs or is stored.
    pub(crate) fn parse(reply: &str, people: &People) -> Result<Self, ChatError> {
        let wire: ProposalWire = serde_json::from_str(extract_json_object(reply))?;

        Ok(match wire {
            ProposalWire::Answer { reply, query } => {
                if let Some(query) = query.as_ref() {
                    query.compile(people)?;
                }
                Self::Answer {
                    reply: as_prose(reply),
                    query,
                }
            }
            ProposalWire::Create {
                reply,
                metric,
                widgets,
                dashboard,
            } => {
                if metric.is_none() && widgets.is_empty() && dashboard.is_none() {
                    return Err(ChatError::EmptyCreate);
                }

                Self::Create {
                    reply: as_prose(reply),
                    metric: metric
                        .map(|named| compile_named_metric(named, people))
                        .transpose()?,
                    widgets: widgets.into_iter().map(NamedBody::into_pair).collect(),
                    dashboard: dashboard.map(NamedBody::into_pair),
                }
            }
        })
    }
}

fn compile_named_metric(named: NamedBody, people: &People) -> Result<(String, Value), ChatError> {
    let metric: MetricQuery = serde_json::from_value(named.body.clone())?;
    metric.compile(people)?;
    Ok(named.into_pair())
}

/// The conversation as the API takes it: the turns so far, then the new
/// message. A turn with any other role is dropped rather than trusted.
fn thread(turns: &[Turn], message: &str) -> Vec<Message> {
    let mut messages: Vec<Message> = turns
        .iter()
        .filter_map(|turn| {
            let role = match turn.role.as_str() {
                "user" => "user",
                "assistant" => "assistant",
                _ => return None,
            };
            Some(Message {
                role,
                content: Value::String(turn.content.clone()),
            })
        })
        .collect();

    messages.push(Message::user(message));

    messages
}

/// The reply as prose.
///
/// Seen live: the model encodes the whole reply a second time, so the string
/// arrives quoted with its newlines escaped and the panel shows `\n` between
/// paragraphs and a trailing quote. Only a value that is entirely one JSON
/// string is unwrapped, so prose that merely contains a quote is untouched.
fn as_prose(reply: String) -> String {
    let trimmed = reply.trim();
    if !(trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() > 1) {
        return reply;
    }

    serde_json::from_str::<String>(trimmed).unwrap_or(reply)
}

fn extract_json_object(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if end >= start => &text[start..=end],
        _ => text,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "intent", rename_all = "lowercase")]
enum ProposalWire {
    Answer {
        reply: String,
        #[serde(default)]
        query: Option<MetricQuery>,
    },
    Create {
        reply: String,
        #[serde(default)]
        metric: Option<NamedBody>,
        #[serde(default)]
        widgets: Vec<NamedBody>,
        #[serde(default)]
        dashboard: Option<NamedBody>,
    },
}

#[derive(Debug, Deserialize)]
struct NamedBody {
    name: String,
    body: Value,
}

impl NamedBody {
    fn into_pair(self) -> (String, Value) {
        (self.name, self.body)
    }
}

impl ChatError {
    /// What the repair round tells the model. `Display` on the JSON variant
    /// names a category; the serde message names the offending field.
    fn feedback(&self) -> String {
        match self {
            Self::Json(error) => error.to_string(),
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum ChatError {
    #[error("the model reply was not valid JSON")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Metric(#[from] MetricQueryError),
    #[error("there is no table named `{table}`; the tables are: {known}")]
    UnknownTable { table: String, known: String },
    #[error("a create must carry at least one metric, widget or dashboard")]
    EmptyCreate,
    #[error("the model kept asking what tables hold instead of answering")]
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

/// Proposes an answer or a creation from a chat message.
///
/// There is one backend that answers: the model. A blank key gives
/// [`ChatError::NoKey`] rather than a reply of our own invention, so a stand
/// that never got a key says so instead of looking answered.
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

    /// A client with nothing to call: every ask is [`ChatError::NoKey`].
    pub(crate) fn keyless() -> Self {
        Self {
            backend: ChatBackend::Keyless,
        }
    }

    /// Always answers with a fixed [`Proposal`] built by `build`, making no
    /// network call — lets a test drive `handle_chat` with a proposal shape
    /// of its own choosing.
    #[cfg(test)]
    pub(crate) fn scripted(build: fn() -> Proposal) -> Self {
        Self {
            backend: ChatBackend::Scripted(build),
        }
    }

    /// # Errors
    ///
    /// Returns [`ChatError`] describing what the upstream did, or why the
    /// reply could not be turned into a [`Proposal`].
    pub(crate) async fn propose(&self, ask: &Ask<'_>) -> Result<Proposal, ChatError> {
        match &self.backend {
            ChatBackend::Keyless => Err(ChatError::NoKey),
            #[cfg(test)]
            ChatBackend::Scripted(build) => Ok(build()),
            ChatBackend::Live { http, token, model } => {
                let transport = Anthropic { http, token, model };
                converse(
                    &transport,
                    ask.schemas,
                    &system_prompt(ask.tables, ask.catalogue, ask.map),
                    thread(ask.turns, ask.message),
                    ask.allowed,
                    ask.people,
                )
                .await
            }
        }
    }
}

/// One forced tool call, returning the proposal JSON the model produced.
/// The turn, which may take several round trips.
///
/// The model sees the map of every table but not their columns, so it may ask
/// what a handful of them hold before it answers. Each answer is fed back as a
/// tool result and the conversation continues; the reply that is not a lookup
/// ends it.
async fn converse(
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
            tracing::info!(tables = ?look_up.tables, "the model asked what these tables hold");
            let described = schemas.describe(&look_up.tables).await;
            messages.push(Message::assistant(response.blocks()));
            messages.push(Message::tool_result(&look_up.id, &described));
            continue;
        }

        let proposed = response.proposal_json();
        return match Proposal::checked(&proposed, allowed, people) {
            Ok(proposal) => Ok(proposal),
            // One repair round: hand the model its own rejection and let it
            // correct itself. The schema stops malformed arguments; this
            // catches what only our own validation knows - an unknown table,
            // a column that is not there.
            Err(rejection) => {
                let detail = rejection.feedback();
                tracing::info!(rejection = %detail, "asking the model to correct its proposal");
                let correction =
                    format!("That proposal was rejected: {detail}\nReturn a corrected proposal.");
                let answer = match response.terminal_tool_id() {
                    Some(id) => Message::tool_error(id, &correction),
                    None => Message::user(correction),
                };
                messages.push(Message::assistant(response.blocks()));
                messages.push(answer);
                let second = transport.send(system, &messages).await?;
                Proposal::checked(&second.proposal_json(), allowed, people)
            }
        };
    }

    Err(ChatError::TooManyLookups)
}

/// One round trip to the model.
///
/// A trait rather than a function so the conversation below - which can take
/// several turns, because the model may ask what a table holds before it
/// answers - is exercised in a test without an HTTP server standing in for
/// the API.
#[async_trait::async_trait]
trait ModelTransport: Send + Sync {
    async fn send(&self, system: &str, messages: &[Message])
    -> Result<MessagesResponse, ChatError>;
}

struct Anthropic<'a> {
    http: &'a reqwest::Client,
    token: &'a SecretString,
    model: &'a str,
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

fn system_prompt(tables: &[KnownTable], catalogue: &Catalogue, map: &str) -> String {
    let mut prompt = String::from(
        "You are the Insight v3 chat assistant. Answer by calling exactly one tool.\n\
         Write replies as plain prose. No markdown: asterisks and hashes are shown as typed.\n\n\
         - Call `answer` to answer a question: it runs one query and stores nothing. Leave the query out when the question is about what data exists.\n\
         - Call `create` to build definitions to store. Pass the metric, the widgets and the dashboard as {\"name\":<string>,\"body\":<object>}, where the name is the identifier and the body is the definition. A create that carries none of the three is refused, and a dashboard needs the metric and widgets it draws.\n\n\
         A MetricQuery is {\"table\":<string>,\"fields\":[{\"json\":<string>,\"type\":\"string\"|\"int\"|\"float\",\"agg\":\"count\"|\"sum\"|\"avg\"|\"min\"|\"max\"|null,\"as_name\":<string>}],\"group_by\":[<string>],\"filters\":[{\"json\":<string>,\"type\":<field type>,\"op\":\"eq\"|\"ne\"|\"gt\"|\"gte\"|\"lt\"|\"lte\",\"value\":<value>}],\"order_by\":{\"field\":<as_name>,\"direction\":\"asc\"|\"desc\"}|null,\"limit\":<int>|null}.\n\
         Give a metric a \"time\" whenever its table carries a timestamp for when the thing happened: {\"time\":{\"column\":\"occurred_at\"}}, or {\"time\":{\"json\":\"committed_at\"}} for a key inside an ingested payload. Without one a reader cannot pick a window and the metric answers every row, whatever the board is set to. Never use the column that records when the row was loaded. Declare no grain: the picked range chooses it, and the rows come back with a `bucket` column a line widget draws on x.\n\
         A question about the most, the largest or the top of something needs order_by on the aggregated field with direction desc, and a limit. Without it the rows come back in the grouping's order and the first row is not the largest.\n\
         Every group_by entry must be spelled exactly like the as_name of a field in the same query.\n\
         A rate is two fields and a third that divides them: give each half its own `when` condition, then a field with \"divide\":[numerator,denominator] and \"percent\":true where a percentage is what the question asked for. A gate pass rate is sum(value) when measure_key is gate_passed, sum(value) when measure_key is gate_runs, then those two divided.\n\
         A column holding a person carries `person`: \"email\" for an address, \"id\" for a person id. The rows then read the name that person is known by rather than the handle a source system wrote, so group by people that way in preference to any name column on the table itself.\n\
         A widget draws its metric's columns by their as_name, never by the raw json field: a metric whose as_name is total_lines is drawn as y total_lines.\n\
         A widget is one of: {\"type\":\"table\",\"metric\":<metric name>,\"columns\":[<string>]}; {\"type\":\"line\"|\"bar\"|\"area\",\"metric\":<metric name>,\"x\":<string>,\"y\":<string>}; {\"type\":\"stat\",\"metric\":<metric name>,\"value\":<string>,\"label\":<string>}; {\"type\":\"pie\",\"metric\":<metric name>,\"label\":<string>,\"value\":<string>}.\n\
         Pick the one that answers the question: a count per category is a bar, a count over time is a line, a running total is an area, a single number is a stat, a share of a total is a pie, and anything with several columns worth reading is a table.\n\
         A dashboard is {\"title\":<string>,\"items\":[<item>]}, drawn top to bottom. An item is {\"widget\":<widget name>}, {\"heading\":<string>} for a section title over the widgets that follow, or {\"text\":<string>} for a line saying what a number means or leaves out. Group the widgets under headings when a board holds more than a handful.\n",
    );

    if tables.is_empty() {
        prompt.push_str("\nNo tables are known yet.\n");
    } else {
        prompt.push_str("\nKnown tables:\n");
        for table in tables {
            prompt.push_str("- ");
            prompt.push_str(&table.name);
            prompt.push_str(": ");
            prompt.push_str(&table.fields);
            prompt.push('\n');
        }
    }

    if !map.is_empty() {
        prompt.push_str(
            "\nEvery table on this stand, by layer. Bronze is a provider's raw \
             payloads, silver is cleaned per-source models, gold is the \
             published metrics, and identity is who people are. Columns are NOT \
             listed: call `look_up` for the tables you mean to query, then name \
             their columns exactly.\n\n",
        );
        prompt.push_str(map);
        prompt.push('\n');
        prompt.push_str(
            "\nA query on one of those tables names its `database` and reads \
             real columns, so each field and filter carries `column`. Only v3's \
             own ingest tables keep their payload in one JSON column, and there \
             a field carries `json` instead. A field may not carry both.\n",
        );
    }

    if catalogue.is_empty() {
        prompt.push_str("\nNothing is built yet.\n");
    } else {
        push_catalogue(&mut prompt, "Metrics", &catalogue.metrics);
        push_catalogue(&mut prompt, "Widgets", &catalogue.widgets);
        push_catalogue(&mut prompt, "Dashboards", &catalogue.dashboards);
        prompt.push_str(
            "\nReusing a name replaces what is stored under it, which is how a \
             dashboard is changed: build it again with the widgets it should \
             hold now.\n",
        );
    }

    prompt
}

fn push_catalogue(prompt: &mut String, label: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }

    prompt.push('\n');
    prompt.push_str(label);
    prompt.push_str(" already built: ");
    prompt.push_str(&names.join(", "));
    prompt.push('\n');
}

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: &'a [Message],
    tools: Vec<Value>,
    tool_choice: Value,
}

/// The structured query a metric carries. Shared by both tools: the
/// answer tool runs one, the create tool stores one.
fn metric_query_schema() -> Value {
    let plain = json!({ "type": "string" });
    let field_type = json!({ "enum": ["string", "int", "float"] });

    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["table", "fields", "group_by", "filters"],
        "properties": {
            "table": plain,
            "database": {
                "type": "string",
                "description": "The database the table is in, from the map. Omit only for a table ingested here.",
            },
            "fields": { "type": "array", "items": metric_field_schema() },
            "time": {
                "type": "object",
                "additionalProperties": false,
                "description": "The timestamp a reader may window and bucket this metric by - the moment the thing happened, never the moment the row arrived. Exactly one of column or json.",
                "properties": {
                    "column": {
                        "type": "string",
                        "description": "A real date or datetime column of the table.",
                    },
                    "json": {
                        "type": "string",
                        "description": "A key inside the payload column holding a timestamp, for a table ingested here only.",
                    },
                    "type": { "enum": ["datetime"] },
                },
            },
            "max_range": {
                "type": "string",
                "description": "The widest window this metric will answer, as an ISO duration of whole days, months or years - P30D, P6M, P1Y. A wider request is refused rather than left to time out.",
            },
            "group_by": { "type": "array", "items": plain },
            "order_by": {
                "type": "object",
                "additionalProperties": false,
                "required": ["field"],
                "properties": {
                    "field": { "type": "string" },
                    "direction": { "enum": ["asc", "desc"] },
                },
            },
            "filters": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["type", "op", "value"],
                    "properties": {
                        "column": { "type": "string" },
                        "json": { "type": "string" },
                        "type": field_type,
                        "op": { "enum": ["eq", "ne", "gt", "gte", "lt", "lte"] },
                        "value": { "type": ["string", "number", "boolean"] },
                    },
                },
            },
            "limit": { "type": "integer" },
        },
    })
}

/// The two tools the model may call. The tool it picks IS the intent, so a
/// question cannot be mistaken for a creation. The schemas guide the shape and
/// document the name charset. `strict` is deliberately NOT set: the nested
/// [`MetricQuery`] shape exceeds the API's compiled-grammar budget and a strict
/// request is refused outright ("the compiled grammar is too large"). What the
/// schema cannot enforce, our own validation refuses and the repair round fixes.
fn metric_field_schema() -> Value {
    let plain = json!({ "type": "string" });
    let field_type = json!({ "enum": ["string", "int", "float"] });

    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["type", "as_name"],
        "properties": {
            "column": {
                "type": "string",
                "description": "A real column, for any table from the map. Exactly one of column or json.",
            },
            "json": {
                "type": "string",
                "description": "A key inside the payload column, for a table ingested here only.",
            },
            "type": field_type,
            "agg": { "enum": ["count", "sum", "avg", "min", "max"] },
            "as_name": plain,
            "person": {
                "enum": ["email", "id"],
                "description": "Set when this column holds a person: email for an address, id for a person id. The rows then carry the name they are known by.",
            },
            "when": {
                "type": "array",
                "description": "Conditions on this aggregate alone, for one half of a rate: a numerator and a denominator that live in the same column are told apart here.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["type", "op", "value"],
                    "properties": {
                        "column": plain,
                        "json": plain,
                        "type": field_type,
                        "op": { "enum": ["eq", "ne", "gt", "gte", "lt", "lte"] },
                        "value": { "type": ["string", "number", "boolean"] },
                    },
                },
            },
            "divide": {
                "type": "array",
                "description": "Two as_names of THIS query, [numerator, denominator], both selected before this field. The rate is their division.",
                "items": plain,
            },
            "percent": {
                "type": "boolean",
                "description": "Read that division as a percentage.",
            },
        },
    })
}

fn proposal_tools() -> Vec<Value> {
    let plain = json!({ "type": "string" });
    let name = json!({
        "type": "string",
        "pattern": NAME_PATTERN,
        "description": "letters, digits, underscore and dash only - never a space",
    });
    let metric_query = metric_query_schema();

    let widget = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["type", "metric"],
        "properties": {
            "type": { "enum": ["table", "line", "bar", "area", "stat", "pie"] },
            "metric": name,
            "columns": { "type": "array", "items": plain },
            "x": plain,
            "y": plain,
            "value": plain,
            "label": plain,
        },
    });
    let item = json!({
        "type": "object",
        "additionalProperties": false,
        "description": "Exactly one of widget, heading or text.",
        "properties": {
            "widget": name,
            "heading": { "type": "string" },
            "text": { "type": "string" },
        },
    });
    let dashboard = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "items"],
        "properties": {
            "title": { "type": "string" },
            "items": { "type": "array", "items": item },
        },
    });
    let named = |body: Value| {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "body"],
            "properties": { "name": name, "body": body },
        })
    };

    vec![
        json!({
            "name": LOOK_UP_TOOL,
            "description": "Read the columns of tables named in the map above, before querying them. Call this whenever you do not already know a table's exact column names - guessing them is the most common way a query fails.",
            "input_schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["tables"],
                "properties": {
                    "tables": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Tables as `database.table`, at most a handful at a time.",
                    },
                },
            },
        }),
        json!({
            "name": ANSWER_TOOL,
            "description": "Answer a question. Stores nothing. Include the query to read data; leave it out when the question is about what data exists, which the table list above already answers.",
            "input_schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["reply"],
                "properties": { "reply": { "type": "string" }, "query": metric_query.clone() },
            },
        }),
        json!({
            "name": CREATE_TOOL,
            "description": "Build metric, widget and dashboard definitions to store. Use only when asked to build or save something. Carry every definition the request needs: a dashboard request means the metric, the widgets that draw it, and the dashboard holding them.",
            "input_schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["reply"],
                "properties": {
                    "reply": { "type": "string" },
                    "metric": named(metric_query),
                    "widgets": { "type": "array", "items": named(widget) },
                    "dashboard": named(dashboard),
                },
            },
        }),
    ]
}

/// One turn on the wire. `content` is a string for prose and an array of
/// content blocks when it carries a tool result, which is why it is a value
/// rather than a `&str`.
#[derive(Clone, Serialize)]
struct Message {
    role: &'static str,
    content: Value,
}

impl Message {
    fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user",
            content: Value::String(content.into()),
        }
    }

    /// The assistant's own turn, echoed back verbatim. The API requires the
    /// `tool_use` block it produced to precede the result we return for it.
    fn assistant(blocks: Value) -> Self {
        Self {
            role: "assistant",
            content: blocks,
        }
    }

    /// The answer to one `tool_use`, addressed by its id.
    fn tool_result(id: &str, content: &str) -> Self {
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
    ///
    /// The API requires every `tool_use` to be answered by a `tool_result`;
    /// following one with a plain message is a 400, which is how the repair
    /// round used to fail instead of repairing.
    fn tool_error(id: &str, content: &str) -> Self {
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

#[derive(Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
}

/// A request for the columns of some tables.
struct LookUp {
    /// The `tool_use` id the result must be addressed to.
    id: String,
    tables: Vec<String>,
}

impl MessagesResponse {
    /// The turn as the API needs it echoed back: a tool result must follow the
    /// assistant turn that asked for it, carrying the same `tool_use` block.
    fn blocks(&self) -> Value {
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
    fn terminal_tool_id(&self) -> Option<&str> {
        self.content
            .iter()
            .find(|block| {
                block.kind == "tool_use" && (block.name == ANSWER_TOOL || block.name == CREATE_TOOL)
            })
            .map(|block| block.id.as_str())
    }

    /// The tables this turn asks about, when it asks rather than answers.
    fn look_up(&self) -> Option<LookUp> {
        let block = self
            .content
            .iter()
            .find(|block| block.kind == "tool_use" && block.name == LOOK_UP_TOOL)?;
        let tables = block
            .input
            .as_ref()?
            .get("tables")?
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
    fn proposal_json(&self) -> String {
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

#[derive(Deserialize)]
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn people() -> People {
        People::new("identity")
    }

    #[test]
    fn an_answer_intent_carries_a_query_and_stores_nothing() {
        let reply = r#"{"intent":"answer","reply":"About 59 lines on the first day","query":{"table":"events","fields":[{"json":"day","type":"string","as_name":"day"},{"json":"lines","type":"int","agg":"sum","as_name":"lines"}],"group_by":["day"],"filters":[]}}"#;

        match Proposal::parse(reply, &people()).unwrap_or_else(|error| panic!("parses: {error}")) {
            Proposal::Answer { reply, query } => {
                assert_eq!(reply, "About 59 lines on the first day");
                let Some(query) = query else {
                    panic!("this answer carries a query");
                };
                query
                    .compile(&people())
                    .unwrap_or_else(|error| panic!("the query compiles: {error}"));
            }
            Proposal::Create { .. } => panic!("expected an answer"),
        }
    }

    #[test]
    fn a_create_intent_is_read_out_of_the_model_reply() {
        let reply = r#"{"intent":"create","reply":"Here you go","metric":{"name":"commits_per_day","body":{"table":"events","fields":[{"json":"day","type":"string","as_name":"day"}],"group_by":["day"],"filters":[]}},"widgets":[{"name":"commits_table","body":{"type":"table","metric":"commits_per_day","columns":["day"]}}],"dashboard":{"name":"engineering","body":{"title":"Engineering","widgets":["commits_table"]}}}"#;

        match Proposal::parse(reply, &people()).unwrap_or_else(|error| panic!("parses: {error}")) {
            Proposal::Create { reply, widgets, .. } => {
                assert_eq!(reply, "Here you go");
                assert_eq!(widgets.len(), 1);
            }
            Proposal::Answer { .. } => panic!("expected a creation"),
        }
    }

    #[test]
    fn a_metric_the_compiler_refuses_is_not_stored() {
        let reply = r#"{"intent":"create","reply":"x","metric":{"name":"bad","body":{"table":"events`--","fields":[],"group_by":[],"filters":[]}},"widgets":[],"dashboard":null}"#;

        assert!(matches!(
            Proposal::parse(reply, &people()),
            Err(ChatError::Metric(_))
        ));
    }

    #[test]
    fn a_question_about_the_data_itself_answers_without_a_query() {
        // The tool used to require a query, so a question the data cannot
        // answer got an invented one - and the reply carried a table of 31
        // rows of `count: 0` beneath it.
        let reply = json!({
            "intent": "answer",
            "reply": "You have events, with author, day, event and lines."
        })
        .to_string();

        let proposal = Proposal::parse(&reply, &people())
            .unwrap_or_else(|error| panic!("an answer needs no query: {error}"));

        assert!(matches!(proposal, Proposal::Answer { query: None, .. }));
    }

    /// The tool of that name, so a test does not break when the list grows.
    fn tool(name: &str) -> Value {
        proposal_tools()
            .into_iter()
            .find(|tool| tool["name"] == json!(name))
            .unwrap_or_else(|| panic!("{name} is offered"))
    }

    #[test]
    fn the_query_schema_offers_an_ordering() {
        let answer = tool(ANSWER_TOOL);
        let order = &answer["input_schema"]["properties"]["query"]["properties"]["order_by"];

        assert_eq!(order["properties"]["field"]["type"], json!("string"));
        assert_eq!(
            order["properties"]["direction"]["enum"],
            json!(["asc", "desc"])
        );
    }

    #[test]
    fn the_answer_tool_asks_only_for_the_reply() {
        let answer = tool(ANSWER_TOOL);

        assert_eq!(answer["input_schema"]["required"], json!(["reply"]));
        // Still described, so the model knows a query is how it reads data.
        assert!(answer["input_schema"]["properties"]["query"].is_object());
    }

    #[test]
    fn a_reply_the_model_encoded_twice_is_read_back_as_prose() {
        // Seen live: the whole reply arrived as a quoted JSON string, so the
        // panel showed literal \n between paragraphs and a trailing quote.
        let reply = json!({
            "intent": "answer",
            "reply": r#""One.\n\nTwo.""#
        })
        .to_string();

        let proposal = Proposal::parse(&reply, &people())
            .unwrap_or_else(|error| panic!("the fixture parses: {error}"));

        let Proposal::Answer { reply, .. } = proposal else {
            panic!("expected an answer");
        };
        assert_eq!(reply, "One.\n\nTwo.");
    }

    #[test]
    fn prose_that_merely_contains_quotes_is_left_alone() {
        let reply = json!({
            "intent": "answer",
            "reply": "The column is called \"day\"."
        })
        .to_string();

        let proposal = Proposal::parse(&reply, &people())
            .unwrap_or_else(|error| panic!("the fixture parses: {error}"));

        let Proposal::Answer { reply, .. } = proposal else {
            panic!("expected an answer");
        };
        assert_eq!(reply, "The column is called \"day\".");
    }

    #[test]
    fn the_whole_thread_reaches_the_model_with_the_new_turn_last() {
        // Each request used to carry the newest message alone, so the model
        // answered "and by author?" with no idea what came before it.
        let turns = [
            Turn::user("how many lines per day?"),
            Turn::assistant("Here they are."),
        ];

        let messages = thread(&turns, "and by author?");

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "how many lines per day?");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[2].role, "user");
        assert_eq!(messages[2].content, "and by author?");
    }

    #[test]
    fn a_first_message_is_a_thread_of_one() {
        let messages = thread(&[], "hello");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
    }

    #[test]
    fn the_prompt_names_what_is_already_built() {
        let prompt = system_prompt(
            &[KnownTable {
                name: "events".to_owned(),
                fields: "day (string)".to_owned(),
            }],
            &Catalogue {
                metrics: vec!["lines_per_day".to_owned()],
                widgets: vec!["lines_chart".to_owned()],
                dashboards: vec!["engineering".to_owned()],
            },
            "",
        );

        assert!(prompt.contains("lines_per_day"), "{prompt}");
        assert!(prompt.contains("lines_chart"), "{prompt}");
        assert!(prompt.contains("engineering"), "{prompt}");
    }

    #[test]
    fn the_grammar_lets_a_metric_declare_the_clock_a_reader_windows_by() {
        let schema = metric_query_schema();
        let properties = &schema["properties"];

        assert!(properties.get("time").is_some(), "{schema}");
        assert!(properties.get("max_range").is_some(), "{schema}");
    }

    #[test]
    fn the_prompt_asks_for_a_clock_when_the_table_carries_one() {
        let prompt = system_prompt(
            &[KnownTable {
                name: "events".to_owned(),
                fields: "occurred_at (datetime)".to_owned(),
            }],
            &Catalogue::default(),
            "",
        );

        assert!(prompt.contains("\"time\""), "{prompt}");
    }

    #[test]
    fn an_empty_catalogue_says_nothing_is_built_yet() {
        let prompt = system_prompt(
            &[KnownTable {
                name: "events".to_owned(),
                fields: "day (string)".to_owned(),
            }],
            &Catalogue::default(),
            "",
        );

        assert!(prompt.contains("Nothing is built yet"), "{prompt}");
    }

    /// A model whose answers are decided in advance, so the conversation is
    /// exercised without an HTTP server standing in for the API.
    struct ScriptedModel {
        answers: std::sync::Mutex<std::collections::VecDeque<Value>>,
        seen: std::sync::Mutex<Vec<Vec<Message>>>,
    }

    impl ScriptedModel {
        fn new(answers: Vec<Value>) -> Self {
            Self {
                answers: std::sync::Mutex::new(answers.into_iter().collect()),
                seen: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn turns(&self) -> Vec<Vec<Message>> {
            self.seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl ModelTransport for ScriptedModel {
        async fn send(
            &self,
            _system: &str,
            messages: &[Message],
        ) -> Result<MessagesResponse, ChatError> {
            self.seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(messages.to_vec());
            let next = self
                .answers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front();
            let Some(next) = next else {
                panic!("the model was called more times than the test scripted");
            };

            Ok(serde_json::from_value(next)
                .unwrap_or_else(|error| panic!("the scripted answer parses: {error}")))
        }
    }

    struct FixedSchemas(&'static str);

    #[async_trait::async_trait]
    impl Schemas for FixedSchemas {
        async fn describe(&self, _tables: &[String]) -> String {
            self.0.to_owned()
        }
    }

    fn look_up_turn(id: &str, tables: &Value) -> Value {
        json!({ "content": [
            { "type": "tool_use", "id": id, "name": LOOK_UP_TOOL, "input": { "tables": tables } },
        ]})
    }

    fn answer_turn(reply: &str) -> Value {
        json!({ "content": [
            { "type": "tool_use", "id": "t2", "name": ANSWER_TOOL, "input": { "reply": reply } },
        ]})
    }

    #[tokio::test]
    async fn a_schema_lookup_is_answered_and_the_turn_continues() {
        let model = ScriptedModel::new(vec![
            look_up_turn("t1", &json!(["silver.git_commits"])),
            answer_turn("There are 12 commits."),
        ]);

        let proposal = converse(
            &model,
            &FixedSchemas("silver.git_commits\n  author_email String\n"),
            "system",
            vec![Message::user("how many commits?")],
            &[],
            &people(),
        )
        .await
        .unwrap_or_else(|error| panic!("the turn completes: {error}"));

        let Proposal::Answer { reply, .. } = proposal else {
            panic!("expected an answer");
        };
        assert_eq!(reply, "There are 12 commits.");

        // The second call carries the question, the model's own tool_use, and
        // the result addressed to it — the API refuses any other order.
        let second = &model.turns()[1];
        assert_eq!(second.len(), 3);
        assert_eq!(second[1].role, "assistant");
        assert_eq!(second[1].content[0]["type"], "tool_use");
        assert_eq!(second[1].content[0]["id"], "t1");
        assert_eq!(second[2].role, "user");
        assert_eq!(second[2].content[0]["type"], "tool_result");
        assert_eq!(second[2].content[0]["tool_use_id"], "t1");
        assert!(
            second[2].content[0]["content"]
                .as_str()
                .unwrap_or_default()
                .contains("author_email"),
            "the columns must reach the model"
        );
    }

    #[tokio::test]
    async fn a_rejected_proposal_is_refused_to_the_call_that_made_it() {
        // The API answers 400 when a tool_use is followed by anything but a
        // tool_result for it, which is how the repair round used to fail.
        let model = ScriptedModel::new(vec![
            json!({ "content": [
                { "type": "tool_use", "id": "bad", "name": ANSWER_TOOL, "input": {} },
            ]}),
            answer_turn("Corrected."),
        ]);

        let proposal = converse(
            &model,
            &FixedSchemas("unused"),
            "system",
            vec![Message::user("ask something")],
            &[],
            &people(),
        )
        .await
        .unwrap_or_else(|error| panic!("the repair round completes: {error}"));

        assert!(matches!(proposal, Proposal::Answer { .. }));

        let repair = &model.turns()[1];
        let result = &repair[2].content[0];
        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["tool_use_id"], "bad");
        assert_eq!(result["is_error"], true);
        assert!(
            result["content"]
                .as_str()
                .unwrap_or_default()
                .contains("rejected"),
            "the model must be told what was wrong"
        );
    }

    #[tokio::test]
    async fn a_model_that_only_ever_looks_up_is_stopped() {
        // Four lookups is one past the budget, so the turn ends rather than
        // spending the reader's money in a circle.
        let model = ScriptedModel::new(vec![
            look_up_turn("t1", &json!(["silver.a"])),
            look_up_turn("t2", &json!(["silver.b"])),
            look_up_turn("t3", &json!(["silver.c"])),
            look_up_turn("t4", &json!(["silver.d"])),
        ]);

        let refused = converse(
            &model,
            &FixedSchemas("columns"),
            "system",
            vec![Message::user("go round in circles")],
            &[],
            &people(),
        )
        .await;

        assert!(matches!(refused, Err(ChatError::TooManyLookups)));
    }

    #[tokio::test]
    async fn an_answer_needs_no_lookup_at_all() {
        let model = ScriptedModel::new(vec![answer_turn("Straight to it.")]);

        let proposal = converse(
            &model,
            &FixedSchemas("unused"),
            "system",
            vec![Message::user("what data is there?")],
            &[],
            &people(),
        )
        .await
        .unwrap_or_else(|error| panic!("the turn completes: {error}"));

        assert!(matches!(proposal, Proposal::Answer { .. }));
        assert_eq!(model.turns().len(), 1);
    }

    #[test]
    fn a_lookup_asking_for_nothing_is_not_a_lookup() {
        // An empty list would spend a round trip and teach the model nothing.
        let empty: MessagesResponse = serde_json::from_value(look_up_turn("t1", &json!([])))
            .unwrap_or_else(|error| panic!("the fixture parses: {error}"));

        assert!(empty.look_up().is_none());
    }

    #[test]
    fn the_map_and_both_field_shapes_are_explained() {
        let prompt = system_prompt(
            &[],
            &Catalogue::default(),
            "Gold (published metrics)\n  insight:\n    git_metric_observations\n",
        );

        assert!(prompt.contains("git_metric_observations"), "{prompt}");
        assert!(prompt.contains("look_up"), "{prompt}");
        // Which shape belongs to which table is the thing it gets wrong.
        assert!(prompt.contains("`column`"), "{prompt}");
        assert!(prompt.contains("`json`"), "{prompt}");
    }

    #[test]
    fn the_widget_schema_offers_exactly_the_kinds_the_renderer_draws() {
        // The schema the model writes against and the switch that draws the
        // result are two views of one vocabulary. Every bug here came from
        // those two drifting apart.
        let created = tool(CREATE_TOOL);
        let widget = &created["input_schema"]["properties"]["widgets"]["items"]["properties"]["body"]
            ["properties"];

        assert_eq!(
            widget["type"]["enum"],
            json!(["table", "line", "bar", "area", "stat", "pie"])
        );
        for field in ["metric", "columns", "x", "y", "value", "label"] {
            assert!(widget[field].is_object(), "{field} is offered");
        }
    }

    #[test]
    fn the_query_schema_offers_both_a_database_and_a_column() {
        let answer = tool(ANSWER_TOOL);
        let query = &answer["input_schema"]["properties"]["query"]["properties"];

        assert_eq!(query["database"]["type"], json!("string"));
        let field = &query["fields"]["items"]["properties"];
        assert_eq!(field["person"]["enum"], json!(["email", "id"]));
        assert!(
            field["when"]["items"].is_object(),
            "a field can carry its own condition"
        );
        assert!(
            field["divide"]["items"].is_object(),
            "a field can divide two others"
        );
        assert_eq!(field["percent"]["type"], json!("boolean"));
        assert_eq!(field["column"]["type"], json!("string"));
        // Neither is required: exactly one of them is, which no JSON schema
        // this API accepts can express, so the compiler refuses it instead.
        assert_eq!(
            query["fields"]["items"]["required"],
            json!(["type", "as_name"])
        );
    }

    #[test]
    fn a_lookup_tool_is_offered() {
        let look_up = tool(LOOK_UP_TOOL);

        assert_eq!(
            look_up["input_schema"]["properties"]["tables"]["type"],
            json!("array")
        );
        assert_eq!(look_up["input_schema"]["required"], json!(["tables"]));
    }

    #[test]
    fn the_prompt_names_every_table_with_its_fields() {
        let prompt = system_prompt(
            &[KnownTable {
                name: "events".to_owned(),
                fields: "day (string), lines (int)".to_owned(),
            }],
            &Catalogue::default(),
            "",
        );

        assert!(
            prompt.contains("- events: day (string), lines (int)"),
            "{prompt}"
        );
        assert!(!prompt.contains("No tables are known yet"), "{prompt}");
    }

    #[test]
    fn an_answer_naming_a_table_the_reader_does_not_have_is_refused() {
        let known = ["events".to_owned(), "silver.class_git_commits".to_owned()];
        let reply = json!({
            "intent": "answer",
            "reply": "here",
            "query": {
                "table": "information_schema_tables",
                "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
                "group_by": [],
                "filters": []
            }
        })
        .to_string();

        let Err(rejection) = Proposal::checked(&reply, &known, &people()) else {
            panic!("a table that does not exist must not reach the database");
        };

        // The message is what the repair round hands back, so it has to name
        // the tables that DO exist.
        let feedback = rejection.feedback();
        assert!(feedback.contains("information_schema_tables"), "{feedback}");
        assert!(feedback.contains("events"), "{feedback}");
    }

    #[test]
    fn a_known_table_passes_the_check() {
        let known = ["events".to_owned()];
        let reply = json!({
            "intent": "answer",
            "reply": "here",
            "query": {
                "table": "events",
                "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
                "group_by": [],
                "filters": []
            }
        })
        .to_string();

        assert!(matches!(
            Proposal::checked(&reply, &known, &people()),
            Ok(Proposal::Answer { .. })
        ));
    }

    #[test]
    fn a_layer_table_passes_when_its_database_qualifies_it() {
        let known = ["silver.class_git_commits".to_owned()];
        let reply = json!({
            "intent": "answer",
            "reply": "here",
            "query": {
                "database": "silver",
                "table": "class_git_commits",
                "fields": [{ "column": "author_email", "type": "string", "as_name": "author" }],
                "group_by": [],
                "filters": []
            }
        })
        .to_string();

        assert!(matches!(
            Proposal::checked(&reply, &known, &people()),
            Ok(Proposal::Answer { .. })
        ));
    }

    #[test]
    fn the_same_table_in_another_database_is_refused() {
        // Two layers can hold a table of one name, so the database is part of
        // what is checked rather than dropped.
        let known = ["silver.class_git_commits".to_owned()];
        let reply = json!({
            "intent": "answer",
            "reply": "here",
            "query": {
                "database": "bronze_github",
                "table": "class_git_commits",
                "fields": [{ "column": "author_email", "type": "string", "as_name": "author" }],
                "group_by": [],
                "filters": []
            }
        })
        .to_string();

        assert!(matches!(
            Proposal::checked(&reply, &known, &people()),
            Err(ChatError::UnknownTable { .. })
        ));
    }

    #[test]
    fn with_nothing_ingested_the_check_stands_aside() {
        // Refusing every table when we know of none would block the chat
        // outright on a stand whose listing failed.
        let reply = json!({
            "intent": "answer",
            "reply": "here",
            "query": {
                "table": "events",
                "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
                "group_by": [],
                "filters": []
            }
        })
        .to_string();

        assert!(matches!(
            Proposal::checked(&reply, &[], &people()),
            Ok(Proposal::Answer { .. })
        ));
    }

    #[test]
    fn one_tool_per_intent_carries_the_name_charset() {
        let tools = proposal_tools();

        // One tool per intent, plus the lookup that ends no turn.
        assert_eq!(tools.len(), 3);
        assert!(tools.iter().any(|tool| tool["name"] == json!(ANSWER_TOOL)));
        assert!(tools.iter().any(|tool| tool["name"] == json!(CREATE_TOOL)));

        // Not strict on purpose: the nested MetricQuery exceeds the API's
        // compiled-grammar budget and a strict request is refused outright.
        // Verified by hand against the live API before this was written.
        for offered in &tools {
            assert!(offered.get("strict").is_none(), "strict must stay off");
            assert_eq!(
                offered["input_schema"]["additionalProperties"],
                json!(false)
            );
        }

        // Every name the model invents carries the charset DefinitionName
        // enforces, stated where the model reads it.
        let created = tool(CREATE_TOOL);
        let create = &created["input_schema"]["properties"];
        for path in [
            &create["metric"]["properties"]["name"],
            &create["dashboard"]["properties"]["name"],
            &create["widgets"]["items"]["properties"]["name"],
        ] {
            assert_eq!(path["pattern"], json!(NAME_PATTERN), "missing name pattern");
        }
    }

    #[test]
    fn the_chosen_tool_becomes_the_intent() {
        let answered: MessagesResponse = serde_json::from_value(json!({
            "content": [
                { "type": "text", "text": "looking that up" },
                { "type": "tool_use", "name": ANSWER_TOOL,
                  "input": { "reply": "here", "query": {} } },
            ],
        }))
        .unwrap_or_else(|error| panic!("the fixture deserializes: {error}"));

        let json = answered.proposal_json();
        assert!(json.contains("\"intent\":\"answer\""), "got {json}");
        assert!(!json.contains("looking that up"), "text leaked in");

        let created: MessagesResponse = serde_json::from_value(json!({
            "content": [
                { "type": "tool_use", "name": CREATE_TOOL,
                  "input": { "reply": "made it", "widgets": [] } },
            ],
        }))
        .unwrap_or_else(|error| panic!("the fixture deserializes: {error}"));

        assert!(created.proposal_json().contains("\"intent\":\"create\""));
    }

    #[test]
    fn prose_around_the_json_is_tolerated() {
        let reply = "Sure!\n```json\n{\"intent\":\"create\",\"reply\":\"ok\",\"widgets\":[],\"dashboard\":{\"name\":\"lines\",\"body\":{\"title\":\"Lines\",\"widgets\":[]}}}\n```";

        assert!(matches!(
            Proposal::parse(reply, &people()),
            Ok(Proposal::Create { .. })
        ));
    }

    #[test]
    fn a_create_that_stores_nothing_is_refused() {
        let reply = json!({ "intent": "create", "reply": "done", "widgets": [] }).to_string();

        assert!(matches!(
            Proposal::parse(&reply, &people()),
            Err(ChatError::EmptyCreate)
        ));
    }

    #[test]
    fn an_answer_with_a_field_outside_a_selected_group_by_is_a_metric_refusal() {
        let reply = json!({
            "intent": "answer",
            "reply": "x",
            "query": {
                "table": "events",
                "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
                "group_by": ["not_selected"],
                "filters": []
            }
        })
        .to_string();

        assert!(matches!(
            Proposal::parse(&reply, &people()),
            Err(ChatError::Metric(MetricQueryError::GroupBy(_)))
        ));
    }

    #[tokio::test]
    async fn a_blank_key_says_so_instead_of_answering() {
        let client = ChatClient::new(&SecretString::from("   ".to_owned()), "model".to_owned());

        let refusal = client
            .propose(&Ask {
                message: "commits per day",
                turns: &[],
                tables: &[],
                catalogue: &Catalogue::default(),
                map: "",
                allowed: &[],
                schemas: &FixedSchemas("unused"),
                people: &people(),
            })
            .await;

        assert!(matches!(refusal, Err(ChatError::NoKey)));
    }
}
