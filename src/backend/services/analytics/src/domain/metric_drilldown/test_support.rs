use chrono::NaiveDate;
use uuid::Uuid;

use crate::domain::metric_definitions::definition::{
    MetricBase, MetricDirection, MetricFormat, MetricInput, MetricInputRole, ObservationRelation,
    ObservationSource,
};
use crate::domain::metric_definitions::{ComputationSpec, EvidenceRelation, MetricDefinition};

use super::cursor::selection_fingerprint;
use super::dto::{
    EvidenceInput, EvidencePlan, EvidenceQueryRow, MetricDrilldownEntity, MetricDrilldownFilter,
    MetricDrilldownPeriod, MetricDrilldownSelection, ValidatedMetricDrilldown,
};

pub(super) fn input(role: MetricInputRole, measure_key: &str) -> MetricInput {
    MetricInput {
        role,
        observation: ObservationSource::Managed(
            ObservationRelation::parse("git_metric_observations")
                .unwrap_or_else(|| panic!("observation relation must parse")),
        ),
        source_key: "git".to_owned(),
        measure_key: measure_key.to_owned(),
    }
}

pub(super) fn definition(spec: ComputationSpec, dimensions: &[&str]) -> MetricDefinition {
    MetricDefinition {
        base: MetricBase {
            key: "git.example".to_owned(),
            label: "Example".to_owned(),
            short_label: None,
            description: None,
            explanation: None,
            entity_type: "person".to_owned(),
            format: MetricFormat::Integer,
            unit: None,
            direction: MetricDirection::Neutral,
            peer_cohort_key: None,
            allowed_dimensions: dimensions.iter().map(|value| (*value).to_owned()).collect(),
        },
        spec,
        transform: None,
    }
}

pub(super) fn plan(spec: ComputationSpec, inputs: Vec<EvidenceInput>) -> EvidencePlan {
    EvidencePlan {
        definition: definition(spec, &["repository", "category"]),
        relation: EvidenceRelation::parse("git_metric_evidence")
            .unwrap_or_else(|| panic!("evidence relation must parse")),
        source_key: "git".to_owned(),
        inputs,
    }
}

pub(super) fn row() -> EvidenceQueryRow {
    EvidenceQueryRow {
            role: "value".to_owned(),
            metric_date: "2026-07-01".to_owned(),
            observed_at: "2026-07-01 10:00:00".to_owned(),
            source_key: "git".to_owned(),
            measure_key: "commit_count".to_owned(),
            record_id: "abc123".to_owned(),
            record_kind: "commit".to_owned(),
            contribution: Some(1.0),
            numerator: None,
            denominator: None,
            subject_key: String::new(),
            dimensions_json: r#"[{"key":"repository","value":"repo","label":"Repository"},{"key":"category","value":"code","label":null}]"#.to_owned(),
            details: serde_json::json!({
                "ref": "abc123",
                "title": "Change",
                "repository": "org/repo",
                "author": "Developer",
                "lines_added": "12",
                "lines_removed": "3"
            }),
        }
}

pub(super) const TEST_PERSON: Uuid = Uuid::from_u128(0x019e_2830_0000_7000_8000_0000_0000_0001);
pub(super) const TEST_TENANT: Uuid = Uuid::from_u128(0x019e_2830_0000_7000_8000_0000_0000_00aa);

pub(super) fn validated(plan: EvidencePlan) -> ValidatedMetricDrilldown {
    let selection = MetricDrilldownSelection {
        metric_key: plan.definition.key().to_owned(),
        entity: MetricDrilldownEntity::Person {
            id: TEST_PERSON.to_string(),
        },
        period: MetricDrilldownPeriod {
            from: "2026-07-01".to_owned(),
            to: "2026-07-31".to_owned(),
        },
        filters: vec![MetricDrilldownFilter {
            dimension: "repository".to_owned(),
            values: vec!["org/repo".to_owned()],
        }],
        display_dimensions: vec!["category".to_owned()],
    };
    ValidatedMetricDrilldown {
        tenant_id: TEST_TENANT,
        enforce_tenant_scope: true,
        fingerprint: selection_fingerprint(Uuid::nil(), &selection)
            .unwrap_or_else(|error| panic!("selection fingerprint must build: {error}")),
        selection,
        from: NaiveDate::from_ymd_opt(2026, 7, 1)
            .unwrap_or_else(|| panic!("valid test start date")),
        to: NaiveDate::from_ymd_opt(2026, 7, 31).unwrap_or_else(|| panic!("valid test end date")),
        limit: 1,
        cursor: None,
        plan,
        snapshot_id: "snapshot".to_owned(),
    }
}
