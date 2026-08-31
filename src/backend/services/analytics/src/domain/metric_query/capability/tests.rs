use serde_json::{Value, json};

use super::super::catalog::product_metric_catalog;
use super::super::comparisons::{Population, population};
use super::super::distributions::distributable;
use super::super::fixtures::{SHIPPED_DISTRIBUTION_METRIC, SHIPPED_METRIC, SHIPPED_TENANT_METRIC};
use super::describe;

use crate::domain::definitions::definition::{
    Computation, Direction, Format, MetricDefinition, Transform,
};
use crate::domain::field_catalog::model::EntityType;

const SHIPPED_RATIO_METRIC: &str = "git.merge_rate";

fn catalogued() -> Value {
    let catalog = product_metric_catalog().expect("the shipped definitions load");
    serde_json::to_value(describe(catalog, true).expect("every shipped metric resolves its inputs"))
        .expect("the catalogue serializes")
}

fn entry(key: &str) -> Value {
    catalogued()["metrics"]
        .as_array()
        .expect("the catalogue lists metrics")
        .iter()
        .find(|metric| metric["key"] == key)
        .unwrap_or_else(|| panic!("`{key}` is catalogued"))
        .clone()
}

fn without_cohort() -> MetricDefinition {
    MetricDefinition {
        key: "probe.uncohorted".to_owned(),
        computation: Computation::Direct {
            measure: "commits".to_owned(),
        },
        transform: Option::<Transform>::None,
        format: Format::Integer,
        direction: Direction::Neutral,
        entity_type: EntityType::Person,
        cohort_key: None,
        label: None,
        description: None,
    }
}

#[test]
fn a_direct_metric_is_catalogued_with_every_question_it_admits() {
    assert_eq!(
        entry(SHIPPED_METRIC),
        json!({
            "key": "git.commits",
            "label": "Commits",
            "description": "Commits a person authored across connected git sources, excluding merge commits.",
            "format": "integer",
            "direction": "higher_is_better",
            "entity_type": "person",
            "computation": { "type": "direct" },
            "cohort_key": "org_unit",
            "dimensions": [
                { "key": "branch_scope", "label": "Branch Scope" },
                { "key": "repository", "label": "Repository" },
                { "key": "project", "label": "Project" },
                { "key": "source", "label": "Source" },
            ],
            "questions": {
                "values": {
                    "grains": ["total", "day", "week", "month"],
                    "folds": ["per_subject", "combined"],
                    "compare": ["previous_period", "month", "quarter", "year"],
                    "split": true,
                },
                "comparisons": {
                    "populations": [{ "type": "tenant" }, { "type": "cohort" }],
                },
                "distributions": { "admitted": false },
                "rows": { "inputs": ["value"] },
            },
        })
    );
}

#[test]
fn a_ratio_metric_names_one_page_of_rows_per_side_of_its_computation() {
    assert_eq!(
        entry(SHIPPED_RATIO_METRIC),
        json!({
            "key": "git.merge_rate",
            "label": "PR merge rate",
            "description": "Of the pull requests created in the period, the share that have merged. Requests opened near the end of the period may not have merged yet, which lowers the rate at period edges.",
            "format": "percent",
            "direction": "higher_is_better",
            "entity_type": "person",
            "computation": { "type": "ratio" },
            "cohort_key": "org_unit",
            "dimensions": [
                { "key": "branch_scope", "label": "Branch Scope" },
                { "key": "destination_branch", "label": "Destination Branch" },
                { "key": "repository", "label": "Repository" },
                { "key": "project", "label": "Project" },
                { "key": "source", "label": "Source" },
            ],
            "questions": {
                "values": {
                    "grains": ["total", "day", "week", "month"],
                    "folds": ["per_subject", "combined"],
                    "compare": ["previous_period", "month", "quarter", "year"],
                    "split": true,
                },
                "comparisons": {
                    "populations": [{ "type": "tenant" }, { "type": "cohort" }],
                },
                "distributions": { "admitted": false },
                "rows": { "inputs": ["numerator", "denominator"] },
            },
        })
    );
}

#[test]
fn a_percentile_metric_is_the_kind_that_admits_a_distribution() {
    assert_eq!(
        entry(SHIPPED_DISTRIBUTION_METRIC),
        json!({
            "key": "git.pr_size",
            "label": "PR size",
            "description": "Median diff size of authored pull requests \u{2014} lines added plus removed. Smaller requests are easier to review. Sources that do not report line counts contribute no values.",
            "format": "integer",
            "direction": "lower_is_better",
            "entity_type": "person",
            "computation": { "type": "percentile" },
            "cohort_key": "org_unit",
            "dimensions": [
                { "key": "branch_scope", "label": "Branch Scope" },
                { "key": "destination_branch", "label": "Destination Branch" },
                { "key": "repository", "label": "Repository" },
                { "key": "project", "label": "Project" },
                { "key": "source", "label": "Source" },
            ],
            "questions": {
                "values": {
                    "grains": ["total", "day", "week", "month"],
                    "folds": ["per_subject", "combined"],
                    "compare": ["previous_period", "month", "quarter", "year"],
                    "split": true,
                },
                "comparisons": {
                    "populations": [{ "type": "tenant" }, { "type": "cohort" }],
                },
                "distributions": { "admitted": true },
                "rows": { "inputs": ["value"] },
            },
        })
    );
}

#[test]
fn every_shipped_metric_is_catalogued_and_none_discloses_a_dimension_value() {
    let catalog = product_metric_catalog().expect("the shipped definitions load");
    let described = catalogued();
    let metrics = described["metrics"].as_array().expect("metrics");

    assert_eq!(metrics.len(), catalog.metrics().count());
    for metric in metrics {
        let dimensions = metric["dimensions"].as_array().expect("dimensions");
        for dimension in dimensions {
            let fields: Vec<&String> = dimension
                .as_object()
                .expect("a dimension is an object")
                .keys()
                .collect();
            assert_eq!(fields, ["key", "label"], "in {metric:?}");
        }
    }
}

#[test]
fn a_metric_advertising_a_distribution_is_one_a_distribution_question_is_admitted_for() {
    let catalog = product_metric_catalog().expect("the shipped definitions load");

    for metric in catalogued()["metrics"].as_array().expect("metrics") {
        let key = metric["key"].as_str().expect("a key");
        let advertised = metric["questions"]["distributions"]["admitted"]
            .as_bool()
            .expect("a flag");

        assert_eq!(
            advertised,
            distributable(catalog, key).is_ok(),
            "`{key}` advertises {advertised} for a distribution"
        );
        assert_eq!(
            advertised,
            matches!(
                metric["computation"]["type"].as_str(),
                Some("percentile" | "stddev")
            ),
            "`{key}` advertises {advertised} for a distribution"
        );
    }
}

#[test]
fn a_metric_declaring_no_cohort_neither_advertises_one_nor_is_compared_within_one() {
    let metric = without_cohort();

    assert!(population(&metric, Population::Cohort {}).is_err());
    assert!(population(&metric, Population::Tenant {}).is_ok());
}

#[test]
fn a_metric_declaring_a_cohort_advertises_both_populations() {
    let catalog = product_metric_catalog().expect("the shipped definitions load");
    let metric = catalog.metric(SHIPPED_METRIC).expect("a shipped metric");

    assert!(metric.cohort_key.is_some());
    assert_eq!(
        entry(SHIPPED_METRIC)["questions"]["comparisons"]["populations"],
        json!([{ "type": "tenant" }, { "type": "cohort" }])
    );
    assert!(population(metric, Population::Cohort {}).is_ok());
}

#[test]
fn a_tenant_metric_is_catalogued_as_asked_about_the_tenant_and_compared_against_nobody() {
    let entry = entry(SHIPPED_TENANT_METRIC);

    assert_eq!(entry["entity_type"], json!("tenant"));
    assert_eq!(entry["cohort_key"], Value::Null);
    assert_eq!(entry["questions"]["comparisons"]["populations"], json!([]));
    assert_eq!(
        entry["questions"]["values"]["grains"],
        json!(["total", "day", "week", "month"])
    );
    assert_eq!(
        entry["questions"]["values"]["folds"],
        json!(["per_subject", "combined"])
    );
}

#[test]
fn the_catalogue_advertises_a_tenant_metric_only_where_the_installation_serves_one() {
    let catalog = product_metric_catalog().expect("the shipped definitions load");
    let keys = |tenant_metrics_enabled| -> Vec<String> {
        describe(catalog, tenant_metrics_enabled)
            .expect("every shipped metric resolves its inputs")
            .metrics
            .into_iter()
            .filter(|metric| metric.entity_type == EntityType::Tenant)
            .map(|metric| metric.key)
            .collect()
    };

    assert!(
        keys(true).contains(&SHIPPED_TENANT_METRIC.to_owned()),
        "an installation serving tenant metrics advertises them: {:?}",
        keys(true)
    );
    assert_eq!(
        keys(false),
        Vec::<String>::new(),
        "an installation that refuses a tenant question must not advertise one"
    );
}

/// The CI family is the first authored at tenant grain, and its metrics carry
/// exactly the dimension sets the family means to offer. A metric's capability
/// is the intersection of its inputs' declared keys, so this reads back what
/// the authored measures actually agree on rather than what they meant to.
#[test]
fn the_ci_family_is_catalogued_with_the_dimensions_its_measures_share() {
    let expected: [(&str, &[&str]); 11] = [
        (
            "ci.runs",
            &["hour_block", "outcome", "pipeline", "repository", "trigger"],
        ),
        (
            "ci.gate_pass_rate",
            &["hour_block", "pipeline", "repository"],
        ),
        ("ci.gate_first_try_pass_rate", &["pipeline", "repository"]),
        ("ci.gate_retry_share", &["pipeline", "repository"]),
        (
            "ci.run_duration_min",
            &["outcome", "pipeline", "repository"],
        ),
        (
            "ci.run_duration_min_p90",
            &["outcome", "pipeline", "repository"],
        ),
        (
            "ci.run_duration_min_stddev",
            &["outcome", "pipeline", "repository"],
        ),
        ("ci.run_hours", &["outcome", "pipeline", "repository"]),
        ("ci.runs_matched_commit", &[]),
        ("ci.commits_observed", &[]),
        (
            "ci.deployments",
            &["env_kind", "environment", "outcome", "repository"],
        ),
    ];

    for (key, dimensions) in expected {
        let entry = entry(key);
        let mut catalogued: Vec<String> = entry["dimensions"]
            .as_array()
            .expect("dimensions are a list")
            .iter()
            .map(|dimension| dimension["key"].as_str().expect("a key").to_owned())
            .collect();
        catalogued.sort();

        assert_eq!(catalogued, dimensions, "`{key}` offers {catalogued:?}");
        assert_eq!(entry["entity_type"], json!("tenant"), "`{key}`");
    }
}
