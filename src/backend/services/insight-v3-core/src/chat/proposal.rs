//! What the model proposed, once it is something this service can act on.

use serde::Deserialize;
use serde_json::Value;

use super::ChatError;
use crate::domain::query::metric_query::{MetricQuery, People};

/// The whole stand is already in the prompt, and hundreds of names in an
/// error help nobody.
const TABLES_NAMED_IN_A_REFUSAL: usize = 12;

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
    /// The refusal goes back through the repair round, so the model gets the
    /// real table list. With no known tables at all the check stands aside.
    pub(super) fn checked(
        reply: &str,
        allowed: &[String],
        people: &People,
    ) -> Result<Self, ChatError> {
        let proposal = Self::parse(reply, people)?;

        if allowed.is_empty() {
            return Ok(proposal);
        }

        match proposal.table() {
            Some(named) if !allowed.contains(&named) => Err(ChatError::UnknownTable {
                table: named,
                known: allowed
                    .iter()
                    .take(TABLES_NAMED_IN_A_REFUSAL)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
            }),
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
    pub(super) fn parse(reply: &str, people: &People) -> Result<Self, ChatError> {
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

/// The reply as prose, unwrapped when the model encoded it a second time.
///
/// Only a value that is entirely one JSON string is unwrapped, so prose that
/// merely contains a quote is untouched.
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
