use serde::Deserialize;
use serde_json::Value;

use super::ChatError;
use crate::domain::query::metric_query::{MetricQuery, People};

const DATASETS_NAMED_IN_A_REFUSAL: usize = 12;

#[derive(Debug)]
enum Reads {
    Nothing,
    Dataset(String),
    SomethingElse,
}

#[derive(Debug)]
pub(crate) enum Proposal {
    Answer {
        reply: String,
        query: Option<MetricQuery>,
    },
    Create {
        reply: String,
        metric: Option<Proposed>,
        widgets: Vec<Proposed>,
        dashboard: Option<Proposed>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Proposed {
    #[serde(default)]
    pub(crate) id: Option<String>,
    pub(crate) name: String,
    pub(crate) body: Value,
}

impl Proposal {
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
                let Some(proposed) = metric.as_ref() else {
                    return Reads::Nothing;
                };
                &proposed.body
            }
        };

        match body.get("dataset").and_then(Value::as_str) {
            Some(named) => Reads::Dataset(named.to_owned()),
            None => Reads::SomethingElse,
        }
    }

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
                refuse_shared_names(&widgets)?;

                Self::Create {
                    reply: as_prose(reply),
                    metric: metric
                        .map(|proposed| readable_metric(proposed, people))
                        .transpose()?,
                    widgets,
                    dashboard,
                }
            }
        })
    }
}

fn readable_metric(proposed: Proposed, _people: &People) -> Result<Proposed, ChatError> {
    let _: MetricQuery = serde_json::from_value(proposed.body.clone())?;

    Ok(proposed)
}

fn refuse_shared_names(widgets: &[Proposed]) -> Result<(), ChatError> {
    for (place, widget) in widgets.iter().enumerate() {
        let shared = widgets[..place]
            .iter()
            .any(|earlier| earlier.name.trim() == widget.name.trim());
        if shared {
            return Err(ChatError::SharedName {
                kind: "widgets",
                name: widget.name.clone(),
            });
        }
    }

    Ok(())
}

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
        metric: Option<Proposed>,
        #[serde(default)]
        widgets: Vec<Proposed>,
        #[serde(default)]
        dashboard: Option<Proposed>,
    },
}
