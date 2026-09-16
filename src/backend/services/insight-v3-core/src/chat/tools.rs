//! The shapes the model must answer in.

use serde_json::{Value, json};

use crate::definitions::DefinitionName;

pub(super) const ANSWER_TOOL: &str = "answer";
pub(super) const CREATE_TOOL: &str = "create";
pub(super) const LOOK_UP_TOOL: &str = "look_up";
/// Definition names, as the store spells the rule.
pub(super) const NAME_PATTERN: &str = DefinitionName::PATTERN;

/// The structured query a metric carries. Shared by both tools: the
/// answer tool runs one, the create tool stores one.
pub(super) fn metric_query_schema() -> Value {
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

/// One field of a metric query.
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

/// The tools the model may call. The tool it picks IS the intent, so a
/// question cannot be mistaken for a creation.
///
/// WORKAROUND: `strict` is deliberately NOT set. The nested `MetricQuery`
/// shape exceeds the API's compiled-grammar budget and a strict request is
/// refused outright. What the schema cannot enforce, our own validation
/// refuses and the repair round fixes.
pub(super) fn proposal_tools() -> Vec<Value> {
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
