//! A condition as a row keeps it: the JSON a draft takes, beside the kind of
//! every column it reads, so it is read back without the metric at hand.

use serde_json::{Map, Value, json};

use super::{Condition, Node, Test};
use crate::domain::query::metric_query::ColumnKind;

impl Condition {
    pub(crate) fn to_stored(&self) -> String {
        let mut kinds = Map::new();
        self.test.collect_kinds(&mut kinds);

        json!({ "condition": self.to_json(), "kinds": kinds }).to_string()
    }

    /// A condition as [`Condition::to_stored`] wrote it, or nothing for
    /// anything else.
    pub(crate) fn from_stored(text: &str) -> Option<Self> {
        let stored: Value = serde_json::from_str(text).ok()?;
        let kinds = stored
            .get("kinds")?
            .as_object()?
            .iter()
            .map(|(column, kind)| Some((column.clone(), kind_of(kind.as_str()?)?)))
            .collect::<Option<Vec<_>>>()?;

        Self::parse(stored.get("condition")?, &kinds).ok()
    }
}

impl Node {
    fn collect_kinds(&self, into: &mut Map<String, Value>) {
        match self {
            Self::All(children) | Self::Any(children) => {
                for child in children {
                    child.collect_kinds(into);
                }
            }
            Self::Not(child) => child.collect_kinds(into),
            Self::Leaf(leaf) => {
                let kind = match leaf.test {
                    Test::Number(_) => "number",
                    Test::Text(_) => "text",
                    Test::Date(_) => "date",
                };
                into.insert(leaf.column.clone(), Value::from(kind));
            }
        }
    }
}

fn kind_of(word: &str) -> Option<ColumnKind> {
    match word {
        "number" => Some(ColumnKind::Number),
        "text" => Some(ColumnKind::Text),
        "date" => Some(ColumnKind::Date),
        _ => None,
    }
}
