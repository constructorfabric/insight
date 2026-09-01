use serde_json::{Value, json};

use super::*;

fn probe_measure() -> Value {
    json!({
        "key": "probe_lines_touched",
        "dataset": "git_commits",
        "description": "Lines added plus removed, over the commits a person authored.",
        "aggregation": "sum",
        "value_expr": "lines_added + lines_removed",
        "event_time": "authored_at",
        "entity": "author_email",
        "dimensions": [
            { "key": "repository", "value_field": "repository", "label_field": "repository_label" },
        ],
    })
}

fn probe_metric(measure: &str) -> Value {
    json!({
        "key": "probe.lines_touched",
        "computation": { "type": "direct", "measure": measure },
        "format": "integer",
        "direction": "neutral",
        "entity_type": "person",
        "label": "Lines touched",
    })
}

fn judged(body: Value) -> ValidateDefinitionsResponse {
    let request: ValidateDefinitionsRequest =
        serde_json::from_value(body).expect("the wire shape parses");
    dry_run(request).expect("the shipped definitions load")
}

fn kinds(response: &ValidateDefinitionsResponse) -> Vec<ValidationErrorKind> {
    response.errors.iter().map(|error| error.kind).collect()
}

#[test]
fn the_request_is_the_shape_the_authored_definitions_are_written_in() {
    let response = judged(json!({
        "measures": [probe_measure()],
        "metrics": [probe_metric("probe_lines_touched")],
    }));

    assert_eq!(
        response,
        ValidateDefinitionsResponse {
            valid: true,
            errors: Vec::new(),
        }
    );
}

#[test]
fn a_request_naming_nothing_judges_the_shipped_definitions_alone() {
    assert!(judged(json!({})).valid);
}

#[test]
fn a_submitted_metric_reads_a_shipped_measure() {
    assert!(judged(json!({ "metrics": [probe_metric("commits")] })).valid);
}

#[test]
fn a_submitted_measure_is_read_by_a_submitted_metric_in_the_same_request() {
    let response = judged(json!({
        "measures": [probe_measure()],
        "metrics": [probe_metric("probe_lines_touched")],
    }));

    assert!(response.valid, "{:?}", response.errors);
}

#[test]
fn a_metric_reading_a_measure_no_side_of_the_union_defines_is_refused() {
    let response = judged(json!({ "metrics": [probe_metric("no_such_measure")] }));

    assert!(!response.valid);
    assert_eq!(kinds(&response), [ValidationErrorKind::MeasureNotFound]);
}

#[test]
fn a_submitted_key_a_shipped_definition_already_holds_collides() {
    let mut measure = probe_measure();
    measure["key"] = json!("commits");

    let response = judged(json!({ "measures": [measure] }));

    assert!(!response.valid);
    assert!(kinds(&response).contains(&ValidationErrorKind::DuplicateKey));
    assert!(
        response
            .errors
            .iter()
            .any(|error| error.message.contains("commits")),
        "{:?}",
        response.errors
    );
}

fn assert_each_kind_is_reported(cases: &[(ValidationErrorKind, Value)]) {
    for (kind, body) in cases {
        let response = judged(body.clone());

        assert!(!response.valid, "should refuse: {kind:?}");
        assert!(
            kinds(&response).contains(kind),
            "should report {kind:?}, reported {:?}",
            response.errors
        );
    }
}

#[test]
fn every_rule_a_measure_can_break_is_reported_under_its_own_discriminant() {
    let cases: [(ValidationErrorKind, Value); 9] = [
        (
            ValidationErrorKind::KeyShape,
            json!({ "measures": [with(probe_measure(), "key", json!("Not A Key"))] }),
        ),
        (
            ValidationErrorKind::DuplicateKey,
            json!({ "measures": [with(probe_measure(), "key", json!("commits"))] }),
        ),
        (
            ValidationErrorKind::DatasetNotFound,
            json!({ "measures": [with(probe_measure(), "dataset", json!("no_such_dataset"))] }),
        ),
        (
            ValidationErrorKind::FieldNotFound,
            json!({ "measures": [with(probe_measure(), "event_time", json!("no_such_field"))] }),
        ),
        (
            ValidationErrorKind::RoleMismatch,
            json!({ "measures": [with(probe_measure(), "entity", json!("lines_added"))] }),
        ),
        (
            ValidationErrorKind::Filter,
            json!({
                "measures": [with(
                    probe_measure(),
                    "filter",
                    json!({ "field": "Not A Field", "op": "eq", "value": "x" }),
                )],
            }),
        ),
        (
            ValidationErrorKind::Expression,
            json!({ "measures": [with(probe_measure(), "value_expr", json!("lines_added +"))] }),
        ),
        (
            ValidationErrorKind::Operand,
            json!({ "measures": [with(probe_measure(), "aggregation", json!("count"))] }),
        ),
        (
            ValidationErrorKind::DimensionBindingsDisagree,
            disagreeing_dimension_bindings(),
        ),
    ];

    assert_each_kind_is_reported(&cases);
}

#[test]
fn every_rule_a_metric_can_break_is_reported_under_its_own_discriminant() {
    let cases: [(ValidationErrorKind, Value); 9] = [
        (
            ValidationErrorKind::MetricKeyShape,
            json!({ "metrics": [with(probe_metric("commits"), "key", json!("undotted"))] }),
        ),
        (
            ValidationErrorKind::MeasureNotFound,
            json!({ "metrics": [probe_metric("no_such_measure")] }),
        ),
        (
            ValidationErrorKind::QuantileOutOfRange,
            computed(json!({
                "type": "percentile", "measure": "commit_change_size", "quantile": 1.5,
            })),
        ),
        (
            ValidationErrorKind::MixedDatasets,
            computed(json!({
                "type": "ratio", "numerator": "commits", "denominator": "prs_created",
            })),
        ),
        (
            ValidationErrorKind::DistributionWithoutValue,
            computed(json!({ "type": "percentile", "measure": "commits", "quantile": 0.5 })),
        ),
        (
            ValidationErrorKind::NoDerivedInputs,
            computed(json!({ "type": "derived", "inputs": {}, "expr": "1" })),
        ),
        (
            ValidationErrorKind::MetricExpression,
            computed(json!({ "type": "derived", "inputs": { "a": "commits" }, "expr": "a +" })),
        ),
        (
            ValidationErrorKind::UnknownDerivedInput,
            computed(json!({ "type": "derived", "inputs": { "a": "commits" }, "expr": "a + b" })),
        ),
        (
            ValidationErrorKind::UnusedDerivedInput,
            computed(json!({
                "type": "derived",
                "inputs": { "a": "commits", "b": "active_commit_days" },
                "expr": "a",
            })),
        ),
    ];

    assert_each_kind_is_reported(&cases);
}

/// A request carrying one metric, computed the way the case names.
fn computed(computation: Value) -> Value {
    json!({ "metrics": [with(probe_metric("commits"), "computation", computation)] })
}

/// Two measures binding one dimension key to different fields, composed by a
/// metric that reads both.
fn disagreeing_dimension_bindings() -> Value {
    let bound = |key: &str, field: &str| {
        with(
            with(probe_measure(), "key", json!(key)),
            "dimensions",
            json!([{ "key": "repository", "value_field": field }]),
        )
    };

    json!({
        "measures": [bound("probe_left", "repository"), bound("probe_right", "project")],
        "metrics": [with(
            probe_metric("probe_left"),
            "computation",
            json!({ "type": "ratio", "numerator": "probe_left", "denominator": "probe_right" }),
        )],
    })
}

fn with(mut definition: Value, field: &str, value: Value) -> Value {
    definition[field] = value;
    definition
}
