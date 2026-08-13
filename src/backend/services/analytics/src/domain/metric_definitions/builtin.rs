use std::sync::OnceLock;

use serde::Deserialize;

use crate::domain::metric_definitions::definition::{
    EvidenceGranularity, MetricComputation, MetricDirection, MetricFormat, MetricInputRole,
    SourceKind, ValueTransform,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Person,
}

impl EntityType {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Person => "person",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CohortKey {
    OrgUnit,
}

impl CohortKey {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::OrgUnit => "org_unit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedComputation {
    Sum,
    Ratio { scale: f64 },
    Median,
    DistinctCount,
}

impl SeedComputation {
    pub fn computation(self) -> MetricComputation {
        match self {
            Self::Sum => MetricComputation::Sum,
            Self::Ratio { .. } => MetricComputation::Ratio,
            Self::Median => MetricComputation::Median,
            Self::DistinctCount => MetricComputation::DistinctCount,
        }
    }

    pub fn scale(self) -> Option<f64> {
        match self {
            Self::Sum | Self::Median | Self::DistinctCount => None,
            Self::Ratio { scale } => Some(scale),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSeed {
    pub key: String,
    pub kind: SourceKind,
    /// Managed-observation relation name; must satisfy
    /// `ObservationRelation::parse` (pinned by a registry test).
    pub source_ref: String,
    pub evidence_ref: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinSource {
    pub source: SourceSeed,
    pub measures: Vec<MeasureSeed>,
    #[serde(default)]
    pub dimensions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureSeed {
    pub key: String,
    pub evidence_granularity: EvidenceGranularity,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricSeed {
    pub metric_key: String,
    pub source_key: String,
    pub label: String,
    /// Compact label for dense surfaces (member grids, heatmap columns);
    /// None = the full label is already compact enough.
    #[serde(default)]
    pub short_label: Option<String>,
    /// The single topic this metric belongs to within its family, so a surface
    /// listing a family can partition it into topics. Required for builtins —
    /// exactly one per metric, which is the partition a source key cannot give.
    pub subject: String,
    /// Cross-cutting labels a surface can filter or search by; many per metric,
    /// unlike the singular `subject`.
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub explanation: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
    pub format: MetricFormat,
    pub direction: MetricDirection,
    pub entity_type: EntityType,
    pub computation: SeedComputation,
    /// Post-aggregation shaping (affine + clamp) applied by the compiler to
    /// every computed value; None = identity.
    #[serde(default)]
    pub transform: Option<ValueTransform>,
    #[serde(default)]
    pub peer_cohort_key: Option<CohortKey>,
    pub inputs: Vec<InputSeed>,
    #[serde(default)]
    pub dimensions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSeed {
    pub input_role: MetricInputRole,
    pub measure_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    sources: Vec<BuiltinSource>,
    metrics: Vec<MetricSeed>,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

const REGISTRY_YAML: &str = include_str!("registry.yaml");

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| {
        #[expect(
            clippy::expect_used,
            reason = "registry.yaml is embedded at compile time and parsed by the registry tests; a parse failure is a build defect, not a runtime condition"
        )]
        serde_yaml::from_str(REGISTRY_YAML).expect("builtin metric registry.yaml must parse")
    })
}

pub fn builtin_sources() -> &'static [BuiltinSource] {
    &registry().sources
}

pub fn builtin_metrics() -> &'static [MetricSeed] {
    &registry().metrics
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use super::*;

    fn is_snake_case(value: &str) -> bool {
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
    }

    fn is_metric_key(value: &str) -> bool {
        let parts = value.split('.').collect::<Vec<_>>();
        parts.len() == 2 && parts.iter().all(|part| is_snake_case(part))
    }

    #[test]
    fn registry_declares_the_expected_counts() {
        assert_eq!(builtin_sources().len(), 6, "builtin source count");
        assert_eq!(builtin_metrics().len(), 62, "builtin metric count");
    }

    #[test]
    fn source_keys_are_unique_and_shaped() {
        let mut seen = BTreeSet::new();
        for builtin_source in builtin_sources() {
            assert!(is_snake_case(&builtin_source.source.key));
            assert!(seen.insert(builtin_source.source.key.as_str()));
        }
    }

    #[test]
    fn source_refs_parse_as_observation_relations() {
        use crate::domain::metric_definitions::definition::ObservationRelation;
        for builtin_source in builtin_sources() {
            assert!(
                ObservationRelation::parse(&builtin_source.source.source_ref).is_some(),
                "builtin source {} declares an invalid observation relation {:?}",
                builtin_source.source.key,
                builtin_source.source.source_ref,
            );
        }
    }

    #[test]
    fn evidence_refs_parse_as_evidence_relations() {
        use crate::domain::metric_definitions::definition::EvidenceRelation;
        for builtin_source in builtin_sources() {
            assert!(
                EvidenceRelation::parse(&builtin_source.source.evidence_ref).is_some(),
                "builtin source {} declares an invalid evidence relation {:?}",
                builtin_source.source.key,
                builtin_source.source.evidence_ref,
            );
        }
    }

    #[test]
    fn every_source_declares_at_least_one_measure() {
        for builtin_source in builtin_sources() {
            assert!(
                !builtin_source.measures.is_empty(),
                "source {} declares no measures",
                builtin_source.source.key
            );
        }
    }

    #[test]
    fn measure_and_dimension_keys_are_unique_per_source() {
        for builtin_source in builtin_sources() {
            let mut measures = BTreeSet::new();
            for measure in &builtin_source.measures {
                assert!(is_snake_case(&measure.key));
                assert!(measures.insert(measure.key.as_str()));
            }
            let mut dimensions = BTreeSet::new();
            for dimension_key in &builtin_source.dimensions {
                assert!(is_snake_case(dimension_key));
                assert!(dimensions.insert(dimension_key.as_str()));
            }
        }
    }

    #[test]
    fn metric_keys_are_unique_and_shaped() {
        let mut seen = BTreeSet::new();
        for metric in builtin_metrics() {
            assert!(is_metric_key(&metric.metric_key), "{}", metric.metric_key);
            assert!(seen.insert(metric.metric_key.as_str()));
        }
    }

    #[test]
    fn metric_inputs_reference_declared_measures() {
        let measures_by_source: HashMap<&str, BTreeSet<&str>> = builtin_sources()
            .iter()
            .map(|builtin_source| {
                (
                    builtin_source.source.key.as_str(),
                    builtin_source
                        .measures
                        .iter()
                        .map(|measure| measure.key.as_str())
                        .collect(),
                )
            })
            .collect();

        for metric in builtin_metrics() {
            let measures = measures_by_source
                .get(metric.source_key.as_str())
                .unwrap_or_else(|| panic!("unknown source for {}", metric.metric_key));
            assert!(!metric.inputs.is_empty(), "{}", metric.metric_key);
            for input in &metric.inputs {
                assert!(
                    measures.contains(input.measure_key.as_str()),
                    "{} references undeclared measure {}",
                    metric.metric_key,
                    input.measure_key
                );
            }
        }
    }

    #[test]
    fn metric_dimensions_reference_declared_source_dimensions() {
        let dimensions_by_source: HashMap<&str, BTreeSet<&str>> = builtin_sources()
            .iter()
            .map(|builtin_source| {
                (
                    builtin_source.source.key.as_str(),
                    builtin_source
                        .dimensions
                        .iter()
                        .map(String::as_str)
                        .collect(),
                )
            })
            .collect();

        for metric in builtin_metrics() {
            let Some(dimensions) = dimensions_by_source.get(metric.source_key.as_str()) else {
                panic!("unknown source for {}", metric.metric_key);
            };
            for dimension in &metric.dimensions {
                assert!(
                    dimensions.contains(dimension.as_str()),
                    "{} references undeclared dimension {dimension}",
                    metric.metric_key
                );
            }
        }
    }

    // Width of metric_definitions.subject and metric_definition_tags.tag. A
    // longer authored value would pass shape checks but fail or truncate at
    // reconcile time, so the bound is enforced here at build time.
    const METADATA_MAX_LEN: usize = 64;

    #[test]
    fn every_metric_declares_a_shaped_subject() {
        for metric in builtin_metrics() {
            assert!(
                is_snake_case(&metric.subject),
                "{} declares an unshaped subject {:?}",
                metric.metric_key,
                metric.subject
            );
            assert!(
                metric.subject.len() <= METADATA_MAX_LEN,
                "{} subject {:?} exceeds {METADATA_MAX_LEN} chars",
                metric.metric_key,
                metric.subject
            );
        }
    }

    #[test]
    fn metric_tags_are_shaped_and_unique_per_metric() {
        for metric in builtin_metrics() {
            let mut seen = BTreeSet::new();
            for tag in &metric.tags {
                assert!(
                    is_snake_case(tag),
                    "{} declares an unshaped tag {tag:?}",
                    metric.metric_key
                );
                assert!(
                    tag.len() <= METADATA_MAX_LEN,
                    "{} tag {tag:?} exceeds {METADATA_MAX_LEN} chars",
                    metric.metric_key
                );
                assert!(
                    seen.insert(tag.as_str()),
                    "{} declares duplicate tag {tag:?}",
                    metric.metric_key
                );
            }
        }
    }

    #[test]
    fn ratio_metrics_have_numerator_and_denominator_roles() {
        for metric in builtin_metrics() {
            let SeedComputation::Ratio { .. } = metric.computation else {
                continue;
            };
            let has_role = |role| metric.inputs.iter().any(|input| input.input_role == role);
            assert!(
                has_role(MetricInputRole::Numerator),
                "{}",
                metric.metric_key
            );
            assert!(
                has_role(MetricInputRole::Denominator),
                "{}",
                metric.metric_key
            );
        }
    }

    #[test]
    fn median_metrics_have_single_value_role() {
        for metric in builtin_metrics() {
            if metric.computation != SeedComputation::Median {
                continue;
            }
            assert_eq!(metric.inputs.len(), 1, "{}", metric.metric_key);
            assert_eq!(
                metric.inputs[0].input_role,
                MetricInputRole::Value,
                "{}",
                metric.metric_key
            );
        }
    }

    #[test]
    fn distinct_count_metrics_have_single_value_role() {
        for metric in builtin_metrics() {
            if metric.computation != SeedComputation::DistinctCount {
                continue;
            }
            assert_eq!(metric.inputs.len(), 1, "{}", metric.metric_key);
            assert_eq!(
                metric.inputs[0].input_role,
                MetricInputRole::Value,
                "{}",
                metric.metric_key
            );
        }
    }

    // Percent and currency formats are presentation-complete: the FE's
    // formatMetricValue/metricDisplayUnit always render "%" or a currency
    // symbol from `format` alone and never consult `unit` for these two
    // formats. A unit string here is therefore dead config that only invites
    // drift (e.g. "percent" vs "%" for the same format) — keep it None.
    #[test]
    fn presentation_complete_formats_carry_no_unit() {
        for metric in builtin_metrics() {
            if !matches!(
                metric.format,
                MetricFormat::Percent | MetricFormat::Currency
            ) {
                continue;
            }
            assert!(
                metric.unit.is_none(),
                "{} has format {:?}, which renders without consulting unit; unit must be None",
                metric.metric_key,
                metric.format
            );
        }
    }
}
