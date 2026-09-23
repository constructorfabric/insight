use serde_json::json;

use super::*;

fn declaration(value: serde_json::Value) -> Declaration {
    serde_json::from_value(value).unwrap_or_else(|error| panic!("the fixture parses: {error}"))
}

fn field(value: serde_json::Value) -> Field {
    serde_json::from_value(value).unwrap_or_else(|error| panic!("the fixture parses: {error}"))
}

#[test]
fn a_value_that_is_not_there_reads_as_empty_and_never_as_a_zero() {
    let cases = [
        ("int", "Nullable(Int64)"),
        ("float", "Nullable(Float64)"),
        ("string", "Nullable(String)"),
        ("bool", "Nullable(Bool)"),
    ];

    for (declared, extracted) in cases {
        let read = read(
            &field(json!({ "name": "value", "path": "value", "type": declared })),
            Form::Presented,
            None,
        );

        assert_eq!(
            read,
            format!("JSONExtract(raw_data, 'value', '{extracted}')"),
            "should read {declared} as empty when it is missing"
        );
    }
}

#[test]
fn a_date_is_read_leniently_because_a_record_carries_what_its_sender_wrote() {
    let read = read(
        &field(json!({ "name": "day", "path": "day", "type": "datetime" })),
        Form::Presented,
        None,
    );

    assert!(read.contains("parseDateTime64BestEffortOrNull"), "{read}");
    assert!(read.ends_with("3, 'UTC')"), "{read}");
}

#[test]
fn a_nested_key_is_read_one_segment_at_a_time() {
    let read = read(
        &field(json!({ "name": "lines", "path": "changed.lines", "type": "int" })),
        Form::Presented,
        None,
    );

    assert_eq!(
        read,
        "JSONExtract(raw_data, 'changed', 'lines', 'Nullable(Int64)')"
    );
}

#[test]
fn a_substitute_stands_in_only_for_what_a_reader_sees() {
    let with_substitute = field(json!({
        "name": "author", "path": "author", "type": "string", "absent_value": "unknown"
    }));

    let presented = read(&with_substitute, Form::Presented, None);
    let raw = read(&with_substitute, Form::Raw, None);

    assert_eq!(
        presented,
        "ifNull(JSONExtract(raw_data, 'author', 'Nullable(String)'), 'unknown')"
    );
    assert_eq!(raw, "JSONExtract(raw_data, 'author', 'Nullable(String)')");
}

#[test]
fn a_quote_in_a_substitute_or_a_key_cannot_end_the_literal_it_sits_in() {
    let read = read(
        &field(json!({
            "name": "author", "path": "it's", "type": "string", "absent_value": "o'brien"
        })),
        Form::Presented,
        None,
    );

    assert!(read.contains("'it\\'s'"), "{read}");
    assert!(read.contains("'o\\'brien'"), "{read}");
}

#[test]
fn a_dataset_naming_no_identity_leaves_every_record_its_own() {
    let declared = declaration(json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [{ "name": "day", "path": "day", "type": "datetime" }]
    }));

    assert_eq!(
        collapsed(&declared, "insight_datasets", "ds_commits_1"),
        "`insight_datasets`.`ds_commits_1`"
    );
}

#[test]
fn records_sharing_an_identity_collapse_to_the_one_received_last() {
    let declared = declaration(json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [
            { "name": "sha", "path": "sha", "type": "string" },
            { "name": "repo", "path": "repo", "type": "string" }
        ],
        "row_identity": ["sha", "repo"]
    }));

    let relation = collapsed(&declared, "insight_datasets", "ds_commits_1");

    assert!(
        relation.contains("ORDER BY received_at DESC, id DESC"),
        "{relation}"
    );
    assert!(relation.contains("LIMIT 1 BY"), "{relation}");
    assert!(
        relation.contains("JSONExtract(raw_data, 'sha', 'Nullable(String)')"),
        "{relation}"
    );
}

#[test]
fn a_record_whose_identity_is_incomplete_is_collapsed_with_nothing() {
    let declared = declaration(json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [{ "name": "sha", "path": "sha", "type": "string" }],
        "row_identity": ["sha"]
    }));

    let relation = collapsed(&declared, "insight_datasets", "ds_commits_1");

    // An absent key is not evidence that two events are the same event, so
    // such a record stands alone under its own id.
    assert!(relation.contains("isNotNull("), "{relation}");
    assert!(relation.contains("'', toString(id))"), "{relation}");
}

#[test]
fn identity_reads_the_record_and_not_what_a_reader_is_shown() {
    let declared = declaration(json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [{
            "name": "author", "path": "author", "type": "string", "absent_value": "unknown"
        }],
        "row_identity": ["author"]
    }));

    let relation = collapsed(&declared, "insight_datasets", "ds_commits_1");

    assert!(
        !relation.contains("unknown"),
        "a substitute would make two records with no author look like one: {relation}"
    );
}

/// A relation holds its values in columns of its own, so a field over one is
/// the column, not an extraction from a payload no such row carries.
#[test]
fn a_field_over_a_relation_reads_the_column_it_names() {
    let cases = [
        ("int", "accurateCastOrNull(`lines`, 'Int64')"),
        ("float", "accurateCastOrNull(`lines`, 'Float64')"),
        ("bool", "accurateCastOrNull(`lines`, 'Bool')"),
        ("datetime", "accurateCastOrNull(`lines`, 'DateTime64(3)')"),
    ];

    for (declared, expected) in cases {
        let read = read(
            &field(json!({ "name": "lines", "column": "lines", "type": declared })),
            Form::Presented,
            None,
        );

        assert_eq!(read, expected, "declared {declared}");
    }
}

/// Every value has a text form, including the composite ones — arrays, tuples,
/// maps — that no other declared type reaches. Declaring such a column a
/// string is how it is seen at all, so this cast may never fail.
#[test]
fn a_column_declared_a_string_is_read_as_its_text_whatever_it_holds() {
    let read = read(
        &field(json!({ "name": "tags", "column": "tags", "type": "string" })),
        Form::Presented,
        None,
    );

    assert_eq!(read, "toString(`tags`)");
}

/// A substitute stands in wherever a value is read, and a relation's column
/// is as capable of holding nothing as a payload's key.
#[test]
fn a_column_that_holds_nothing_takes_the_substitute_the_field_declares() {
    let read = read(
        &field(json!({
            "name": "team",
            "column": "team",
            "type": "string",
            "absent_value": "unassigned"
        })),
        Form::Presented,
        None,
    );

    assert_eq!(read, "ifNull(toString(`team`), 'unassigned')");
}

/// SAFETY: a column name is written into the statement as an identifier. The
/// validator admits only letters, digits and underscore, and the quoting here
/// is the belt that makes a backtick harmless even if that ever slipped.
#[test]
fn a_backtick_in_a_column_name_cannot_close_the_identifier() {
    let read = read(
        &field(json!({ "name": "odd", "column": "a`b", "type": "string" })),
        Form::Presented,
        None,
    );

    assert_eq!(read, "toString(`a``b`)");
}

/// An allow-list, like the engines a relation may be on: a type nobody
/// thought of is read as its text, which always works, rather than cast.
/// `accurateCastOrNull` refuses a composite outright rather than answering
/// nothing, and that refusal would meet the reader on every run.
#[test]
fn a_column_is_read_as_a_declared_type_only_where_the_cast_can_answer() {
    let cases = [
        ("Int64", FieldType::Int, true),
        ("Nullable(Float64)", FieldType::Float, true),
        ("LowCardinality(String)", FieldType::String, true),
        (
            "Nullable(LowCardinality(String))",
            FieldType::Datetime,
            true,
        ),
        ("DateTime64(3)", FieldType::Datetime, true),
        ("Decimal(18, 4)", FieldType::Float, true),
        ("Enum8('a' = 1)", FieldType::String, true),
        ("UUID", FieldType::String, true),
        // Every value has a text form, so a string reads anything at all.
        ("Array(UInt8)", FieldType::String, true),
        ("Map(String, String)", FieldType::String, true),
        // Nothing else reads a composite: the cast refuses it.
        ("Array(UInt8)", FieldType::Int, false),
        ("Tuple(String, String)", FieldType::Float, false),
        ("Map(String, String)", FieldType::Bool, false),
        ("Nullable(Array(String))", FieldType::Datetime, false),
        ("SomethingNobodyThoughtOf", FieldType::Int, false),
    ];

    for (held, declared, expected) in cases {
        assert_eq!(reads_as(held, declared), expected, "{held} as {declared:?}");
    }
}
