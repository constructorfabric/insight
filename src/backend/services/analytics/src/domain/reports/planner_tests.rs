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

const XLSX_MAX_ROWS: u64 = 1_048_576;
const XLSX_MAX_COLUMNS: u64 = 16_384;

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
        ReportPlannerLimits {
            max_batch_cells: 6,
            max_total_cells: u64::MAX,
        },
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
            max_total_cells: u64::MAX,
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
            max_total_cells: u64::MAX,
        },
    )
    .unwrap_or_else(|error| panic!("report should plan: {error}"));

    assert_eq!(plan.bucket, ReportBucket::Quarter);
    assert_eq!(plan.periods.len(), 2);
    assert_eq!(plan.columns.len(), 67);
    assert_eq!(plan.size.total_rows, 2);
    assert_eq!(plan.size.total_cells, 134);
    assert_eq!(
        plan.subject,
        PlannedReportSubject::Tenant {
            id: tenant_id,
            batches: vec![PeriodQueryBatch {
                period_start: 0,
                period_end: 2,
            }],
        }
    );
}

#[test]
fn tenant_batches_are_bounded_by_resident_metric_cells() {
    let tenant_id = Uuid::from_u128(9);
    let recipe = ValidatedReportRecipe {
        subject: ReportSubjectSelection::Tenant { id: tenant_id },
        from: date("2026-01-01"),
        to: date("2026-01-10"),
        granularity: ReportGranularity::Day,
        metrics: (0..2).map(|index| metric(index, "tenant")).collect(),
    };

    let plan = plan_report(
        &recipe,
        &[],
        ReportPlannerLimits {
            max_batch_cells: 6,
            max_total_cells: u64::MAX,
        },
    )
    .unwrap_or_else(|error| panic!("report should plan: {error}"));
    let PlannedReportSubject::Tenant { batches, .. } = &plan.subject else {
        panic!("expected tenant plan");
    };

    assert_eq!(
        batches,
        &[
            PeriodQueryBatch {
                period_start: 0,
                period_end: 3,
            },
            PeriodQueryBatch {
                period_start: 3,
                period_end: 6,
            },
            PeriodQueryBatch {
                period_start: 6,
                period_end: 9,
            },
            PeriodQueryBatch {
                period_start: 9,
                period_end: 10,
            },
        ]
    );
}

#[test]
fn rejects_more_periods_than_one_metric_query_can_return() {
    let tenant_id = Uuid::from_u128(9);
    let recipe = ValidatedReportRecipe {
        subject: ReportSubjectSelection::Tenant { id: tenant_id },
        from: date("2000-01-01"),
        to: date("2013-09-08"),
        granularity: ReportGranularity::Day,
        metrics: vec![metric(0, "tenant")],
    };

    let result = plan_report(
        &recipe,
        &[],
        ReportPlannerLimits {
            max_batch_cells: usize::MAX,
            max_total_cells: u64::MAX,
        },
    );

    assert!(matches!(
        result,
        Err(ReportPlanningError::PeriodLimitExceeded)
    ));
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
            max_total_cells: u64::MAX,
        },
    );
    let Err(error) = result else {
        panic!("reordered profiles must fail closed");
    };

    assert_eq!(error, ReportPlanningError::ProfileSetMismatch);
}

#[test]
fn rejects_total_cells_before_materializing_periods() {
    let profiles = vec![profile(1)];
    let recipe = people_recipe(
        vec![profiles[0].person_id],
        "2020-01-01",
        "2026-12-31",
        ReportGranularity::Day,
        64,
    );

    let result = plan_report(
        &recipe,
        &profiles,
        ReportPlannerLimits {
            max_batch_cells: 100_000,
            max_total_cells: 10_000,
        },
    );

    assert!(matches!(
        result,
        Err(ReportPlanningError::CellLimitExceeded)
    ));
}

#[test]
fn checks_xlsx_dimensions_against_format_limits() {
    let supported = ReportSize {
        total_rows: XLSX_MAX_ROWS - 1,
        total_cells: 1,
        worksheet_rows: XLSX_MAX_ROWS,
        worksheet_columns: XLSX_MAX_COLUMNS,
    };
    assert_eq!(
        supported.xlsx_dimensions(),
        Ok(XlsxDimensions {
            rows: 1_048_576,
            columns: 16_384,
        })
    );

    for exceeded in [
        ReportSize {
            worksheet_rows: XLSX_MAX_ROWS + 1,
            ..supported
        },
        ReportSize {
            worksheet_columns: XLSX_MAX_COLUMNS + 1,
            ..supported
        },
    ] {
        assert_eq!(
            exceeded.xlsx_dimensions(),
            Err(ReportPlanningError::XlsxDimensionsExceeded)
        );
    }
}
