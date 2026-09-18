use serde_json::json;

use super::*;
use crate::domain::kinds::dataset::declaration::Declaration;

fn declaration(value: serde_json::Value) -> Declaration {
    serde_json::from_value(value).unwrap_or_else(|error| panic!("the fixture parses: {error}"))
}

/// A declaration with nothing wrong with it, for a test to spoil one field of.
fn sound() -> serde_json::Value {
    json!({
        "title": "QA test runs",
        "fields": [
            { "name": "run_id", "path": "run_id", "type": "string", "role": "dimension" },
            { "name": "test_name", "path": "test.name", "type": "string", "role": "dimension" },
            { "name": "duration_ms", "path": "duration_ms", "type": "int", "role": "measurable" },
            { "name": "started_at", "path": "started_at", "type": "datetime",
              "role": "time", "default_clock": true }
        ],
        "row_identity": ["run_id", "test_name"]
    })
}

fn violations(name: &str, body: serde_json::Value) -> Vec<Violation> {
    validate(name, &declaration(body))
}

fn fields_of(violations: &[Violation]) -> Vec<&str> {
    violations.iter().map(|v| v.field.as_str()).collect()
}

#[test]
fn a_sound_declaration_is_accepted() {
    assert_eq!(violations("qa_test_runs", sound()), Vec::new());
}

#[test]
fn every_problem_is_reported_at_once() {
    // The author corrects a declaration in one pass, so the first mistake must
    // not hide the rest.
    let found = violations(
        "qa_test_runs",
        json!({
            "title": "  ",
            "fields": [
                { "name": "bucket", "path": "", "type": "string" },
                { "name": "started_at", "path": "started_at", "type": "string",
                  "default_clock": true }
            ],
            "row_identity": ["run_id"]
        }),
    );

    assert_eq!(
        fields_of(&found),
        vec![
            "title",
            "fields[0].name",
            "fields[0].path",
            "fields[1].default_clock",
            "row_identity[0]",
        ],
        "{found:#?}"
    );
}

#[test]
fn a_name_that_is_not_an_identifier_or_is_a_catalogue_path_is_refused() {
    for name in ["", "qa runs", "qa/runs", &"a".repeat(129)] {
        let found = violations(name, sound());
        assert_eq!(fields_of(&found), vec!["name"], "should reject: {name:?}");
        assert_eq!(found[0].reason, Reason::Malformed);
    }

    for name in ["metrics", "widgets", "dashboards", "datasets"] {
        let found = violations(name, sound());
        assert_eq!(fields_of(&found), vec!["name"], "should reject: {name:?}");
        assert_eq!(found[0].reason, Reason::NotAdmissible);
    }
}

#[test]
fn a_dataset_nothing_can_be_asked_of_is_refused() {
    let found = violations(
        "qa_test_runs",
        json!({ "title": "QA test runs", "fields": [] }),
    );

    assert_eq!(fields_of(&found), vec!["fields"]);
    assert_eq!(found[0].reason, Reason::Missing);
}

#[test]
fn one_field_declared_twice_is_refused_once() {
    let found = violations(
        "qa_test_runs",
        json!({
            "title": "QA test runs",
            "fields": [
                { "name": "run_id", "path": "run_id", "type": "string" },
                { "name": "run_id", "path": "run.id", "type": "string" }
            ]
        }),
    );

    assert_eq!(fields_of(&found), vec!["fields[1].name"]);
    assert_eq!(found[0].reason, Reason::Duplicate);
}

#[test]
fn a_field_may_not_claim_the_column_a_windowed_run_injects() {
    let found = violations(
        "qa_test_runs",
        json!({
            "title": "QA test runs",
            "fields": [{ "name": "bucket", "path": "bucket", "type": "string" }]
        }),
    );

    assert_eq!(fields_of(&found), vec!["fields[0].name"]);
    assert!(
        found[0].detail.contains("windowed run injects"),
        "{found:#?}"
    );
}

#[test]
fn a_path_that_cannot_address_a_key_is_refused() {
    for (path, reason) in [
        ("", Reason::Missing),
        (".", Reason::Malformed),
        ("test..name", Reason::Malformed),
        ("trailing.", Reason::Malformed),
        (r"half\", Reason::Malformed),
        ("a.b.c.d.e.f.g.h.i", Reason::NotAdmissible),
    ] {
        let found = violations(
            "qa_test_runs",
            json!({
                "title": "QA test runs",
                "fields": [{ "name": "value", "path": path, "type": "string" }]
            }),
        );

        assert_eq!(
            fields_of(&found),
            vec!["fields[0].path"],
            "should reject: {path:?}"
        );
        assert_eq!(found[0].reason, reason, "should reject: {path:?}");
    }
}

#[test]
fn the_type_decides_what_a_property_may_be_attached_to() {
    // The main date must be a datetime, a person a string, a substitute a
    // string — while the role stays advisory and constrains nothing.
    let cases = [
        (
            json!({ "name": "f", "path": "f", "type": "string", "default_clock": true }),
            "fields[0].default_clock",
        ),
        (
            json!({ "name": "f", "path": "f", "type": "int", "person": "email" }),
            "fields[0].person",
        ),
        (
            json!({ "name": "f", "path": "f", "type": "int", "absent_value": "-" }),
            "fields[0].absent_value",
        ),
    ];

    for (field, expected) in cases {
        let found = violations(
            "qa_test_runs",
            json!({ "title": "QA test runs", "fields": [field.clone()] }),
        );
        assert_eq!(fields_of(&found), vec![expected], "should reject: {field}");
    }
}

#[test]
fn a_role_that_disagrees_with_the_type_is_accepted_because_it_only_describes() {
    // A numeric identifier is grouped by, and calling it a measurable does not
    // stop that; the type is what decides.
    let found = violations(
        "qa_test_runs",
        json!({
            "title": "QA test runs",
            "fields": [{ "name": "project_id", "path": "project_id",
                         "type": "int", "role": "measurable" }]
        }),
    );

    assert_eq!(found, Vec::new(), "{found:#?}");
}

#[test]
fn a_record_has_one_main_date() {
    let found = violations(
        "qa_test_runs",
        json!({
            "title": "QA test runs",
            "fields": [
                { "name": "started_at", "path": "started_at", "type": "datetime",
                  "default_clock": true },
                { "name": "finished_at", "path": "finished_at", "type": "datetime",
                  "default_clock": true }
            ]
        }),
    );

    assert_eq!(fields_of(&found), vec!["fields"]);
    assert!(
        found[0].detail.contains("started_at, finished_at"),
        "{found:#?}"
    );
}

#[test]
fn a_dataset_may_declare_no_main_date_at_all() {
    let found = violations(
        "qa_test_runs",
        json!({
            "title": "QA test runs",
            "fields": [{ "name": "run_id", "path": "run_id", "type": "string" }]
        }),
    );

    assert_eq!(
        found,
        Vec::new(),
        "a dataset with no clock answers every record"
    );
}

#[test]
fn an_identity_naming_something_undeclared_is_refused_and_says_what_is_declared() {
    let found = violations(
        "qa_test_runs",
        json!({
            "title": "QA test runs",
            "fields": [{ "name": "run_id", "path": "run_id", "type": "string" }],
            "row_identity": ["run_id", "test_name"]
        }),
    );

    assert_eq!(fields_of(&found), vec!["row_identity[1]"]);
    assert_eq!(found[0].reason, Reason::Unknown);
    assert!(found[0].detail.contains("declared: run_id"), "{found:#?}");
}

#[test]
fn an_identity_naming_one_field_twice_is_refused() {
    let found = violations(
        "qa_test_runs",
        json!({
            "title": "QA test runs",
            "fields": [{ "name": "run_id", "path": "run_id", "type": "string" }],
            "row_identity": ["run_id", "run_id"]
        }),
    );

    assert_eq!(fields_of(&found), vec!["row_identity[1]"]);
    assert_eq!(found[0].reason, Reason::Duplicate);
}

#[test]
fn every_reason_reaches_the_caller_as_a_stable_code() {
    for (reason, code) in [
        (Reason::Missing, "MISSING"),
        (Reason::Unknown, "UNKNOWN"),
        (Reason::Duplicate, "DUPLICATE"),
        (Reason::Malformed, "MALFORMED"),
        (Reason::NotAdmissible, "NOT_ADMISSIBLE"),
    ] {
        let violation = Violation::new("f", reason, "d");
        assert_eq!(violation.reason_code(), code, "should map: {reason:?}");
    }
}
