//! What a dashboard draws, and in what order.
//!
//! A board is a list of items: the widgets it holds, and the headings and
//! prose that introduce them. `widgets: ["a", "b"]` is the older shorthand for
//! a list of nothing but widgets, and every board written before there was
//! anything else to put in one says it that way — so it is still read, and
//! means the same thing.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{KindError, Reference};
use crate::definitions::DefinitionKind;
use crate::domain::query::time_window::{RequestedRange, WindowError};

/// A stored widget, by name.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct WidgetItem {
    pub(crate) widget: String,
}

/// A section title, over the widgets that follow it.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeadingItem {
    pub(crate) heading: String,
}

/// A line of prose between widgets — what a number means, what it leaves out.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextItem {
    pub(crate) text: String,
}

/// One thing a dashboard draws.
///
/// Each variant refuses fields that are not its own, so an item naming two
/// things at once is a refusal rather than one of them quietly dropped.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum Item {
    Widget(WidgetItem),
    Heading(HeadingItem),
    Text(TextItem),
}

impl Item {
    pub(crate) fn widget(&self) -> Option<&str> {
        match self {
            Self::Widget(item) => Some(&item.widget),
            Self::Heading(_) | Self::Text(_) => None,
        }
    }
}

/// The widgets a body names, in both forms.
///
/// Both, because this answers "is this widget still in use" — a board that
/// says it either way is still drawing it.
fn widgets(body: &Value) -> Vec<String> {
    let mut names = Vec::new();

    if let Some(Value::Array(items)) = body.get("items") {
        names.extend(
            items
                .iter()
                .filter_map(|item| item.get("widget")?.as_str())
                .map(str::to_owned),
        );
    }

    match body.get("widgets") {
        Some(Value::String(one)) => names.push(one.clone()),
        Some(Value::Array(many)) => {
            names.extend(many.iter().filter_map(Value::as_str).map(str::to_owned));
        }
        _ => {}
    }

    names
}

/// The same body, with every item drawing `from` drawing `to` instead.
fn renamed(mut body: Value, from: &str, to: &str) -> Value {
    if let Some(Value::Array(items)) = body.get_mut("items") {
        for item in items {
            let Some(widget) = item.get_mut("widget") else {
                continue;
            };
            if widget.as_str() == Some(from) {
                *widget = Value::String(to.to_owned());
            }
        }
    }

    body
}

/// The body a board has once it is laid out: its own title, and this list.
///
/// The list is the whole list, so the shorthand goes — left in place it would
/// still say what the board used to draw.
pub(crate) fn laid_out(previous: &Value, items: &[Item]) -> Result<Value, serde_json::Error> {
    let mut object = serde_json::Map::new();
    if let Some(title) = previous.get("title") {
        object.insert("title".to_owned(), title.clone());
    }
    object.insert("items".to_owned(), serde_json::to_value(items)?);

    Ok(Value::Object(object))
}

/// What a stored dashboard says about time, checked before it is stored: a
/// board offering a range the server cannot resolve draws a picker whose
/// buttons refuse every widget behind them.
pub(crate) fn check(body: &Value) -> Result<(), KindError> {
    let offered = body
        .get("time_ranges")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let default = body.get("default_range");

    for token in offered.iter().chain(default) {
        let Some(token) = token.as_str() else {
            return Err(KindError::Range(WindowError::Range(token.to_string())));
        };
        RequestedRange::parse(token).map_err(KindError::Range)?;
    }

    Ok(())
}

/// The widgets a board draws, in either form its body may name them.
pub(crate) fn refers_to(body: &Value) -> Vec<Reference> {
    widgets(body)
        .into_iter()
        .map(|widget| Reference::new(DefinitionKind::Widget, widget))
        .collect()
}

/// The same board, drawing `to` where it drew `from`, in both forms it may
/// name a widget.
pub(crate) fn rename_reference(body: Value, from: &str, to: &str) -> Value {
    let mut body = renamed(body, from, to);

    match body.get_mut("widgets") {
        Some(Value::String(one)) if one == from => to.clone_into(one),
        Some(Value::Array(many)) => {
            for entry in many {
                if entry.as_str() == Some(from) {
                    *entry = Value::String(to.to_owned());
                }
            }
        }
        _ => {}
    }

    body
}

#[cfg(test)]
mod tests;
