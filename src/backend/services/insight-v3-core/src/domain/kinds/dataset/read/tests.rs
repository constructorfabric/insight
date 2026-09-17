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
            PAYLOAD_COLUMN,
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
        PAYLOAD_COLUMN,
    );

    assert!(read.contains("parseDateTime64BestEffortOrNull"), "{read}");
    assert!(read.ends_with("3, 'UTC')"), "{read}");
}

#[test]
fn a_nested_key_is_read_one_segment_at_a_time() {
    let read = read(
        &field(json!({ "name": "lines", "path": "changed.lines", "type": "int" })),
        Form::Presented,
        PAYLOAD_COLUMN,
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

    let presented = read(&with_substitute, Form::Presented, PAYLOAD_COLUMN);
    let raw = read(&with_substitute, Form::Raw, PAYLOAD_COLUMN);

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
        PAYLOAD_COLUMN,
    );

    assert!(read.contains("'it\\'s'"), "{read}");
    assert!(read.contains("'o\\'brien'"), "{read}");
}

#[test]
fn a_dataset_naming_no_identity_leaves_every_record_its_own() {
    let declared = declaration(json!({
        "title": "Commits",
        "fields": [{ "name": "day", "path": "day", "type": "datetime" }]
    }));

    assert_eq!(collapsed(&declared, "ds_commits_1"), "`ds_commits_1`");
}

#[test]
fn records_sharing_an_identity_collapse_to_the_one_received_last() {
    let declared = declaration(json!({
        "title": "Commits",
        "fields": [
            { "name": "sha", "path": "sha", "type": "string" },
            { "name": "repo", "path": "repo", "type": "string" }
        ],
        "row_identity": ["sha", "repo"]
    }));

    let relation = collapsed(&declared, "ds_commits_1");

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
        "fields": [{ "name": "sha", "path": "sha", "type": "string" }],
        "row_identity": ["sha"]
    }));

    let relation = collapsed(&declared, "ds_commits_1");

    // An absent key is not evidence that two events are the same event, so
    // such a record stands alone under its own id.
    assert!(relation.contains("isNotNull("), "{relation}");
    assert!(relation.contains("'', toString(id))"), "{relation}");
}

#[test]
fn identity_reads_the_record_and_not_what_a_reader_is_shown() {
    let declared = declaration(json!({
        "title": "Commits",
        "fields": [{
            "name": "author", "path": "author", "type": "string", "absent_value": "unknown"
        }],
        "row_identity": ["author"]
    }));

    let relation = collapsed(&declared, "ds_commits_1");

    assert!(
        !relation.contains("unknown"),
        "a substitute would make two records with no author look like one: {relation}"
    );
}
