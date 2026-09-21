//! How a declared field is read out of a stored record, and which records a
//! run treats as one.

use std::fmt::Write as _;

use super::declaration::{At, Declaration, Field, FieldType, path_segments};

/// The column every record is stored in.
pub(crate) const PAYLOAD_COLUMN: &str = "raw_data";

/// Which form of a value the reader wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    /// The record's own value: no substitute and no identity lookup, so an
    /// empty value stays distinguishable from every real one.
    Raw,
    /// What a reader is shown.
    Presented,
}

/// The expression reading this field out of `payload`.
///
/// INVARIANT: a key that is absent, null, or present but unconvertible reads
/// as empty and never as a zero standing in for a number. An average over one
/// record holding ten and one holding nothing is ten, not five.
pub(crate) fn read(field: &Field, form: Form, payload: &str) -> String {
    let extracted = extract(field, payload);

    match (form, field.absent_value.as_deref()) {
        (Form::Presented, Some(substitute)) => {
            format!("ifNull({extracted}, {})", literal(substitute))
        }
        (Form::Presented | Form::Raw, _) => extracted,
    }
}

/// The relation a run reads: one record per identity.
///
/// A declaration naming no identity leaves the table as it is, every record
/// its own.
pub(crate) fn collapsed(declaration: &Declaration, database: &str, table: &str) -> String {
    let relation = format!("`{database}`.`{table}`");
    let identity: Vec<String> = declaration
        .row_identity
        .iter()
        .filter_map(|named| declaration.field(named))
        // Raw: a substitute would make two records with no key look like one
        // record sharing a value, and a resolved person would group two
        // accounts of one human into one event.
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
    match &field.at {
        At::Path(path) => out_of_payload(field.r#type, &keys(path), payload),
        At::Column(column) => out_of_column(field.r#type, column),
    }
}

/// A value the record carries in its payload, read out by its key path.
fn out_of_payload(declared: FieldType, keys: &str, payload: &str) -> String {
    match declared {
        // Lenient, because a record carries whatever its sender wrote: a
        // timestamp this cannot read is empty rather than a refusal.
        FieldType::Datetime => format!(
            "parseDateTime64BestEffortOrNull(JSONExtract({payload}{keys}, 'Nullable(String)'), 3, 'UTC')"
        ),
        FieldType::String => format!("JSONExtract({payload}{keys}, 'Nullable(String)')"),
        FieldType::Int => format!("JSONExtract({payload}{keys}, 'Nullable(Int64)')"),
        FieldType::Float => format!("JSONExtract({payload}{keys}, 'Nullable(Float64)')"),
        FieldType::Bool => format!("JSONExtract({payload}{keys}, 'Nullable(Bool)')"),
    }
}

/// A value a relation holds in a column of its own.
///
/// INVARIANT: the relation's column keeps its own type wherever it already
/// reads as the declared one, so a number stays a number to the warehouse and
/// an index over it is still usable. Only where the two disagree is a cast
/// written, and a value that cannot be cast reads as empty rather than as a
/// zero standing in for one.
fn out_of_column(declared: FieldType, column: &str) -> String {
    let held = format!("`{}`", column.replace('`', "``"));

    match declared {
        // Lenient for the same reason a payload's is: a column this cannot
        // read is empty rather than a run that fails.
        FieldType::Datetime => format!("accurateCastOrNull({held}, 'DateTime64(3)')"),
        // Every value has a text form, including the composite ones no other
        // declared type reaches, so this is the one cast that cannot fail.
        FieldType::String => format!("toString({held})"),
        FieldType::Int => format!("accurateCastOrNull({held}, 'Int64')"),
        FieldType::Float => format!("accurateCastOrNull({held}, 'Float64')"),
        FieldType::Bool => format!("accurateCastOrNull({held}, 'Bool')"),
    }
}

/// The key path as the extraction takes it: one quoted segment per key.
fn keys(path: &str) -> String {
    let mut written = String::new();
    for segment in path_segments(path) {
        let _ = write!(written, ", {}", literal(&segment));
    }

    written
}

/// A value as a string literal the warehouse reads back unchanged.
///
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
