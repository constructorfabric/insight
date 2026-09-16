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
pub(crate) fn widgets(body: &Value) -> Vec<String> {
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
pub(crate) fn renamed(mut body: Value, from: &str, to: &str) -> Value {
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

#[cfg(test)]
#[path = "dashboard/tests.rs"]
mod tests;
