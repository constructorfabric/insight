//! What the model proposed, once it is something this service can act on.

use serde::Deserialize;
use serde_json::Value;

use super::ChatError;
use crate::domain::query::metric_query::{MetricQuery, People};

/// Every dataset is already in the prompt, and a long list in an error helps
/// nobody.
const DATASETS_NAMED_IN_A_REFUSAL: usize = 12;

/// What a proposal reads, before it is known whether the stand has it.
#[derive(Debug)]
enum Reads {
    /// No query and no metric: a reply that stands on its own.
    Nothing,
    Dataset(String),
    /// A metric that names no dataset, which nothing here can run.
    SomethingElse,
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
    /// [`Proposal::parse`], then refuse anything the stand cannot answer.
    ///
    /// A proposal that names no dataset, or names one nobody declared, goes
    /// back through the repair round with the list, so the model corrects it
    /// in the conversation rather than the reader meeting a refusal.
    pub(super) fn checked(
        reply: &str,
        allowed: &[String],
        people: &People,
    ) -> Result<Self, ChatError> {
        let proposal = Self::parse(reply, people)?;
        let known = || {
            allowed
                .iter()
                .take(DATASETS_NAMED_IN_A_REFUSAL)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        };

        match proposal.reads() {
            Reads::Nothing => Ok(proposal),
            Reads::Dataset(named) if allowed.contains(&named) => Ok(proposal),
            Reads::Dataset(named) => Err(ChatError::UnknownDataset {
                dataset: named,
                known: known(),
            }),
            Reads::SomethingElse => Err(ChatError::NoDataset { known: known() }),
        }
    }

    /// What this proposal reads, which every metric must answer with a
    /// dataset the stand declares.
    fn reads(&self) -> Reads {
        let body = match self {
            Self::Answer { query, .. } => {
                let Some(query) = query.as_ref() else {
                    return Reads::Nothing;
                };

                return match query.dataset() {
                    Some(named) => Reads::Dataset(named.to_owned()),
                    None => Reads::SomethingElse,
                };
            }
            Self::Create { metric, .. } => {
                let Some((_, body)) = metric.as_ref() else {
                    return Reads::Nothing;
                };
                body
            }
        };

        match body.get("dataset").and_then(Value::as_str) {
            Some(named) => Reads::Dataset(named.to_owned()),
            None => Reads::SomethingElse,
        }
    }

    /// Strips any prose or code fence around the JSON object and deserializes
    /// on `intent`.
    ///
    /// A query is not compiled here: compiling one over a dataset needs the
    /// declaration, which the run and the write both read. What this catches
    /// is a reply that is not a proposal at all.
    pub(super) fn parse(reply: &str, people: &People) -> Result<Self, ChatError> {
        let wire: ProposalWire = serde_json::from_str(extract_json_object(reply))?;

        Ok(match wire {
            ProposalWire::Answer { reply, query } => Self::Answer {
                reply: as_prose(reply),
                query,
            },
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
                        .map(|named| readable_metric(named, people))
                        .transpose()?,
                    widgets: widgets.into_iter().map(NamedBody::into_pair).collect(),
                    dashboard: dashboard.map(NamedBody::into_pair),
                }
            }
        })
    }
}

/// A proposed metric that can at least be read as one, so the repair round
/// hears about a malformed body rather than the write path.
fn readable_metric(named: NamedBody, _people: &People) -> Result<(String, Value), ChatError> {
    let _: MetricQuery = serde_json::from_value(named.body.clone())?;

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
