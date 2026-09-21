use serde_json::json;

use super::*;

fn parse(value: serde_json::Value) -> Declaration {
    serde_json::from_value(value).unwrap_or_else(|error| panic!("the fixture parses: {error}"))
}

#[test]
fn a_declaration_needs_only_a_title_and_its_fields() {
    let declaration = parse(json!({
        "title": "QA test runs",
        "source": { "kind": "stream" },
        "fields": [{ "name": "run_id", "path": "run_id", "type": "string" }]
    }));

    assert_eq!(declaration.title, "QA test runs");
    assert!(declaration.description.is_none());
    assert!(declaration.row_identity.is_empty());
    assert!(declaration.default_clock().is_none());
}

#[test]
fn a_property_the_shape_does_not_carry_is_refused_rather_than_ignored() {
    // A mistyped property must reach the author by name; dropping it silently
    // is what makes a declaration disagree with what its author believes.
    let refused: Result<Declaration, _> = serde_json::from_value(json!({
        "title": "QA test runs",
        "source": { "kind": "stream" },
        "fields": [{ "name": "run_id", "path": "run_id", "type": "string" }],
        "rowIdentity": ["run_id"]
    }));

    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn the_main_date_is_the_field_that_claims_it() {
    let declaration = parse(json!({
        "title": "QA test runs",
        "source": { "kind": "stream" },
        "fields": [
            { "name": "run_id", "path": "run_id", "type": "string" },
            { "name": "started_at", "path": "started_at", "type": "datetime",
              "role": "time", "default_clock": true }
        ]
    }));

    let clock = declaration
        .default_clock()
        .unwrap_or_else(|| panic!("the fixture declares one"));
    assert_eq!(clock.name, "started_at");
}

#[test]
fn only_the_numeric_types_are_numeric() {
    for (declared, numeric) in [
        (FieldType::Int, true),
        (FieldType::Float, true),
        (FieldType::String, false),
        (FieldType::Bool, false),
        (FieldType::Datetime, false),
    ] {
        assert_eq!(
            declared.is_numeric(),
            numeric,
            "should be numeric: {numeric:?} for {declared:?}"
        );
    }
}

#[test]
fn a_path_addresses_a_nested_key_and_a_key_holding_a_dot() {
    for (path, expected) in [
        ("status", vec!["status"]),
        ("test.name", vec!["test", "name"]),
        ("commit.author.email", vec!["commit", "author", "email"]),
        // A source is free to use a dot inside a key, so one is escaped.
        (r"metrics\.p95", vec!["metrics.p95"]),
        (r"outer.metrics\.p95", vec!["outer", "metrics.p95"]),
        (r"a\\b", vec![r"a\b"]),
    ] {
        assert_eq!(path_segments(path), expected, "should split: {path:?}");
    }
}

#[test]
fn a_field_is_found_by_the_name_a_metric_uses() {
    let declaration = parse(json!({
        "title": "QA test runs",
        "source": { "kind": "stream" },
        "fields": [
            { "name": "run_id", "path": "run_id", "type": "string" },
            { "name": "duration_ms", "path": "duration_ms", "type": "int" }
        ]
    }));

    assert!(declaration.field("duration_ms").is_some());
    assert!(declaration.field("duration").is_none());
}

#[test]
fn what_is_stored_is_what_was_declared() {
    // The body is written back to the store and read by the portal, so a round
    // trip must not quietly gain or lose a property.
    let body = json!({
        "title": "QA test runs",
        "description": "One record per test in one CI run.",
        "source": { "kind": "stream" },
        "fields": [
            { "name": "branch", "path": "branch", "type": "string",
              "role": "dimension", "absent_value": "(no branch)" },
            { "name": "author", "path": "author", "type": "string", "person": "email" },
            { "name": "started_at", "path": "started_at", "type": "datetime",
              "role": "time", "default_clock": true }
        ],
        "row_identity": ["branch"]
    });

    let round_tripped = serde_json::to_value(parse(body.clone()))
        .unwrap_or_else(|error| panic!("a declaration serialises: {error}"));

    assert_eq!(round_tripped, body);
}
