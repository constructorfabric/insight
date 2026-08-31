use std::collections::BTreeMap;

use chrono::NaiveDate;
use uuid::Uuid;

use super::dto::ReportGranularity;
use super::period::ReportBucket;
use super::planner::*;
use super::validation::{ReportSubjectSelection, ValidatedReportRecipe};
use crate::domain::metric_definitions::definition::{
    AliasCollapse, ComputationSpec, MetricBase, MetricDirection, MetricFormat, MetricInput,
    MetricInputRole, ObservationRelation, ObservationSource,
};
use crate::infra::identity::IdentityProfile;

fn date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .unwrap_or_else(|error| panic!("fixture date must parse: {error}"))
}

fn metric(index: usize, entity_type: &str) -> crate::domain::metric_definitions::MetricDefinition {
    crate::domain::metric_definitions::MetricDefinition {
        base: MetricBase {
            key: format!("test.metric_{index}"),
            label: format!("Metric {index}"),
            short_label: None,
            description: None,
            explanation: None,
            entity_type: entity_type.to_owned(),
            format: MetricFormat::Integer,
            unit: None,
            direction: MetricDirection::Neutral,
            peer_cohort_key: None,
            allowed_dimensions: vec![],
        },
        spec: ComputationSpec::Sum {
            value: MetricInput {
                role: MetricInputRole::Value,
                observation: ObservationSource::Managed(
                    ObservationRelation::parse("test_metric_observations")
                        .unwrap_or_else(|| panic!("fixture relation must parse")),
                ),
                source_key: "test".to_owned(),
                measure_key: "value".to_owned(),
                alias_collapse: AliasCollapse::Sum,
            },
        },
        transform: None,
    }
}

fn profile(id: u128) -> IdentityProfile {
    IdentityProfile {
        person_id: Uuid::from_u128(id),
        attributes: BTreeMap::new(),
        supervisor: None,
    }
}

fn people_recipe(
    ids: Vec<Uuid>,
    from: &str,
    to: &str,
    granularity: ReportGranularity,
    metric_count: usize,
) -> ValidatedReportRecipe {
    ValidatedReportRecipe {
        subject: ReportSubjectSelection::People { ids },
        from: date(from),
        to: date(to),
        granularity,
        metrics: (0..metric_count)
            .map(|index| metric(index, "person"))
            .collect(),
    }
}

#[test]
fn preserves_people_order_and_builds_sequential_cell_bounded_batches() {
    let profiles = vec![profile(3), profile(1), profile(2)];
    let recipe = people_recipe(
        profiles.iter().map(|profile| profile.person_id).collect(),
        "2026-01-15",
        "2026-03-20",
        ReportGranularity::Month,
        1,
    );

    let plan = plan_report(
        &recipe,
        &profiles,
        ReportPlannerLimits { max_batch_cells: 6 },
    )
    .unwrap_or_else(|error| panic!("report should plan: {error}"));

    let PlannedReportSubject::People { ids, batches } = &plan.subject else {
        panic!("expected people plan");
    };
    assert_eq!(
        ids,
        &[Uuid::from_u128(3), Uuid::from_u128(1), Uuid::from_u128(2)]
    );
    assert_eq!(
        batches,
        &[
            PersonQueryBatch {
                person_start: 0,
                person_end: 2,
            },
            PersonQueryBatch {
                person_start: 2,
                person_end: 3,
            },
        ]
    );
    assert_eq!(
        plan.periods
            .iter()
            .map(|period| period.label.as_str())
            .collect::<Vec<_>>(),
        ["2026-01", "2026-02", "2026-03"]
    );
    assert_eq!(plan.size.total_rows, 9);
    assert_eq!(plan.size.total_cells, 45);
    assert_eq!(plan.size.worksheet_rows, 10);
    assert_eq!(plan.size.worksheet_columns, 5);
    for batch in batches {
        let people = batch.person_end - batch.person_start;
        assert!(people * plan.periods.len() * recipe.metrics.len() <= 6);
    }
}

#[test]
fn every_person_batch_satisfies_metric_query_value_limit() {
    let profiles = vec![profile(1), profile(2), profile(3)];
    let recipe = people_recipe(
        profiles.iter().map(|profile| profile.person_id).collect(),
        "2020-01-01",
        "2025-06-22",
        ReportGranularity::Day,
        1,
    );

    let plan = plan_report(
        &recipe,
        &profiles,
        ReportPlannerLimits {
            max_batch_cells: usize::MAX,
        },
    )
    .unwrap_or_else(|error| panic!("report should plan: {error}"));
    let PlannedReportSubject::People { batches, .. } = &plan.subject else {
        panic!("expected people plan");
    };

    for batch in batches {
        let people = batch.person_end - batch.person_start;
        assert!(people * (plan.periods.len() + 1) <= METRIC_QUERY_VALUE_LIMIT);
    }
}

#[test]
fn tenant_plan_supports_more_than_fifty_metrics_without_profile_columns() {
    let tenant_id = Uuid::from_u128(9);
    let recipe = ValidatedReportRecipe {
        subject: ReportSubjectSelection::Tenant { id: tenant_id },
        from: date("2026-01-01"),
        to: date("2026-06-30"),
        granularity: ReportGranularity::Quarter,
        metrics: (0..64).map(|index| metric(index, "tenant")).collect(),
    };

    let plan = plan_report(
        &recipe,
        &[],
        ReportPlannerLimits {
            max_batch_cells: 100_000,
        },
    )
    .unwrap_or_else(|error| panic!("report should plan: {error}"));

    assert_eq!(plan.bucket, ReportBucket::Quarter);
    assert_eq!(plan.periods.len(), 2);
    assert_eq!(plan.columns.len(), 67);
    assert_eq!(plan.size.total_rows, 2);
    assert_eq!(plan.size.total_cells, 134);
    assert_eq!(plan.subject, PlannedReportSubject::Tenant { id: tenant_id });
}

#[test]
fn rejects_profiles_that_do_not_match_people_order() {
    let recipe = people_recipe(
        vec![Uuid::from_u128(1), Uuid::from_u128(2)],
        "2026-01-01",
        "2026-01-31",
        ReportGranularity::Month,
        1,
    );
    let profiles = vec![profile(2), profile(1)];

    let result = plan_report(
        &recipe,
        &profiles,
        ReportPlannerLimits {
            max_batch_cells: 100,
        },
    );
    let Err(error) = result else {
        panic!("reordered profiles must fail closed");
    };

    assert_eq!(error, ReportPlanningError::ProfileSetMismatch);
}
