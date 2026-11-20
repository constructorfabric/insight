use std::fmt::Write as _;

use super::declaration::{Declaration, Field, FieldType, path_segments};

pub(crate) const PAYLOAD_COLUMN: &str = "raw_data";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    Raw,
    Presented,
}

pub(crate) fn read(field: &Field, form: Form, payload: &str) -> String {
    let extracted = extract(field, payload);

    match (form, field.absent_value.as_deref()) {
        (Form::Presented, Some(substitute)) => {
            format!("ifNull({extracted}, {})", literal(substitute))
        }
        (Form::Presented | Form::Raw, _) => extracted,
    }
}

pub(crate) fn collapsed(declaration: &Declaration, database: &str, table: &str) -> String {
    let relation = format!("`{database}`.`{table}`");
    let identity: Vec<String> = declaration
        .row_identity
        .iter()
        .filter_map(|named| declaration.field(named))
        .map(|field| read(field, Form::Raw, PAYLOAD_COLUMN))
        .collect();

    if identity.is_empty() {
        return relation;
    }

    let complete = identity
        .iter()
        .map(|expression| format!("isNotNull({expression})"))
        .collect::<Vec<_>>()
        .join(" AND ");

    format!(
        "(SELECT * FROM {relation} ORDER BY received_at DESC, id DESC LIMIT 1 BY {}, if({complete}, '', toString(id)))",
        identity.join(", ")
    )
}

fn extract(field: &Field, payload: &str) -> String {
    let keys = keys(&field.path);

    match field.r#type {
        FieldType::Datetime => format!(
            "parseDateTime64BestEffortOrNull(JSONExtract({payload}{keys}, 'Nullable(String)'), 3, 'UTC')"
        ),
        FieldType::String => format!("JSONExtract({payload}{keys}, 'Nullable(String)')"),
        FieldType::Int => format!("JSONExtract({payload}{keys}, 'Nullable(Int64)')"),
        FieldType::Float => format!("JSONExtract({payload}{keys}, 'Nullable(Float64)')"),
        FieldType::Bool => format!("JSONExtract({payload}{keys}, 'Nullable(Bool)')"),
    }
}

fn keys(path: &str) -> String {
    let mut written = String::new();
    for segment in path_segments(path) {
        let _ = write!(written, ", {}", literal(&segment));
    }

    written
}

/// WORKAROUND: the ClickHouse client scans the whole statement for `?` to
/// find its bind sites and does not know a string literal from anything else,
/// so a `?` inside one would swallow the next bound value. `??` is its escape
/// and emits a single `?`.
fn literal(value: &str) -> String {
    format!(
        "'{}'",
        value
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('?', "??")
    )
}

#[cfg(test)]
mod tests;
