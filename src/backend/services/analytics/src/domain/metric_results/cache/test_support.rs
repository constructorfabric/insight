use chrono::NaiveDate;
use uuid::Uuid;

use crate::domain::metric_definitions::definition::{
    ComputationSpec, CustomObservationSql, MetricBase, MetricDefinition, MetricDirection,
    MetricFormat, MetricInput, MetricInputRole, ObservationRelation, ObservationSource,
};

use super::super::validation::{
    ValidatedDimensionFilter, ValidatedEntitySelection, ValidatedMetricRequest,
    ValidatedMetricResultsRequest, ValidatedMetricView,
};

pub const TENANT: Uuid = Uuid::from_u128(0x7a11);

pub fn entity(index: usize) -> Uuid {
    Uuid::from_u128(index as u128)
}

fn base() -> MetricBase {
    MetricBase {
        key: "ai.accepted_lines".to_owned(),
        label: "AI-added lines".to_owned(),
        short_label: None,
        description: None,
        explanation: None,
        entity_type: "person".to_owned(),
        format: MetricFormat::Integer,
        unit: None,
        direction: MetricDirection::HigherIsBetter,
        peer_cohort_key: None,
        allowed_dimensions: vec!["tool".to_owned()],
    }
}

fn managed_input() -> MetricInput {
    MetricInput {
        role: MetricInputRole::Value,
        observation: ObservationSource::Managed(
            ObservationRelation::parse("ai_metric_observations")
                .unwrap_or_else(|| panic!("fixture relation must parse")),
        ),
        source_key: "ai_usage".to_owned(),
        measure_key: "accepted_lines".to_owned(),
    }
}

pub fn sum_metric() -> MetricDefinition {
    MetricDefinition {
        transform: None,
        base: base(),
        spec: ComputationSpec::Sum {
            value: managed_input(),
        },
    }
}

/// A ratio whose denominator reads inline SQL: only the numerator names a
/// managed relation, so nothing about this metric can be pinned to an epoch.
pub fn split_source_ratio_metric() -> MetricDefinition {
    MetricDefinition {
        transform: None,
        base: base(),
        spec: ComputationSpec::Ratio {
            numerator: managed_input(),
            denominator: MetricInput {
                role: MetricInputRole::Denominator,
                observation: ObservationSource::Custom(CustomObservationSql::new(
                    "SELECT * FROM insight.ai_metric_observations".to_owned(),
                )),
                source_key: "ai_usage".to_owned(),
                measure_key: "tool_use_offered".to_owned(),
            },
            scale: 100.0,
        },
    }
}

pub fn custom_sql_metric() -> MetricDefinition {
    MetricDefinition {
        transform: None,
        base: base(),
        spec: ComputationSpec::Sum {
            value: MetricInput {
                role: MetricInputRole::Value,
                observation: ObservationSource::Custom(CustomObservationSql::new(
                    "SELECT * FROM insight.ai_metric_observations".to_owned(),
                )),
                source_key: "ai_usage".to_owned(),
                measure_key: "accepted_lines".to_owned(),
            },
        },
    }
}

pub fn metric_request(
    def: MetricDefinition,
    views: Vec<ValidatedMetricView>,
) -> ValidatedMetricRequest {
    ValidatedMetricRequest {
        def,
        filters: Vec::new(),
        views,
    }
}

pub fn filtered_metric_request(
    def: MetricDefinition,
    filters: Vec<ValidatedDimensionFilter>,
    views: Vec<ValidatedMetricView>,
) -> ValidatedMetricRequest {
    ValidatedMetricRequest {
        def,
        filters,
        views,
    }
}

pub fn request(
    ids: Vec<Uuid>,
    metrics: Vec<ValidatedMetricRequest>,
) -> ValidatedMetricResultsRequest {
    ValidatedMetricResultsRequest {
        tenant_id: TENANT,
        entity: ValidatedEntitySelection::Person { ids },
        from: date("2026-01-01"),
        to: date("2026-01-31"),
        metrics,
        enforce_tenant_scope: true,
    }
}

pub fn date(value: &str) -> NaiveDate {
    match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        Ok(date) => date,
        Err(error) => panic!("bad fixture date {value}: {error}"),
    }
}
