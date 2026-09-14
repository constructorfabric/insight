use std::collections::BTreeMap;

use chrono::NaiveDate;
use uuid::Uuid;

use super::dto::ReportGranularity;
use super::executor::ReportExecutionContext;
use super::row::{ReportCell, ReportMetricValue};
use super::validation::{ReportSubjectSelection, ValidatedReportRecipe};
use crate::domain::metric_definitions::MetricDefinition;
use crate::domain::metric_definitions::definition::{
    AliasCollapse, ComputationSpec, MetricBase, MetricDirection, MetricFormat, MetricInput,
    MetricInputRole, ObservationRelation, ObservationSource,
};
use crate::infra::identity::IdentityProfile;

pub(super) fn context() -> ReportExecutionContext {
    ReportExecutionContext {
        tenant_id: Uuid::from_u128(9),
        enforce_tenant_scope: true,
    }
}

pub(super) fn people_recipe(
    profiles: &[IdentityProfile],
    metric_count: usize,
) -> ValidatedReportRecipe {
    ValidatedReportRecipe {
        subject: ReportSubjectSelection::People {
            ids: profiles.iter().map(|profile| profile.person_id).collect(),
        },
        from: date("2026-01-01"),
        to: date("2026-02-28"),
        granularity: ReportGranularity::Month,
        metrics: (0..metric_count)
            .map(|index| metric(index, "person"))
            .collect(),
    }
}

pub(super) fn tenant_recipe(tenant_id: Uuid, metric_count: usize) -> ValidatedReportRecipe {
    ValidatedReportRecipe {
        subject: ReportSubjectSelection::Tenant { id: tenant_id },
        from: date("2026-01-01"),
        to: date("2026-02-28"),
        granularity: ReportGranularity::Month,
        metrics: (0..metric_count)
            .map(|index| metric(index, "tenant"))
            .collect(),
    }
}

pub(super) fn profile(id: u128, display_name: &str) -> IdentityProfile {
    IdentityProfile {
        person_id: Uuid::from_u128(id),
        attributes: BTreeMap::from([("display_name".to_owned(), display_name.to_owned())]),
        supervisor: None,
    }
}

fn metric(index: usize, entity_type: &str) -> MetricDefinition {
    MetricDefinition {
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

pub(super) fn date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .unwrap_or_else(|error| panic!("fixture date must parse: {error}"))
}

pub(super) fn value(entity_id: u128, bucket_start: &str, value: Option<f64>) -> ReportMetricValue {
    ReportMetricValue {
        entity_id: Uuid::from_u128(entity_id),
        bucket_start: date(bucket_start),
        value,
    }
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the positional row cell shape"
)]
pub(super) fn text(value: &str) -> Option<ReportCell> {
    Some(ReportCell::Text(value.to_owned()))
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the positional row cell shape"
)]
pub(super) fn number(value: f64) -> Option<ReportCell> {
    Some(ReportCell::Number(value))
}

pub(super) fn metric_number(index: usize) -> f64 {
    f64::from(
        u32::try_from(index).unwrap_or_else(|error| panic!("fixture index must fit u32: {error}")),
    )
}
