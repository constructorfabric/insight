use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::domain::metric_definitions::definition::{
    AliasCollapse, EvidenceGranularity, MetricComputation, MetricDirection, MetricFormat,
    MetricInputRole, RatioDenominatorAggregation, SourceKind, ValueTransform,
};
use crate::domain::metric_definitions::evidence_presentation::EvidencePresentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Person,
    Tenant,
}

impl EntityType {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Tenant => "tenant",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "person" => Some(Self::Person),
            "tenant" => Some(Self::Tenant),
            _ => None,
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
    Ratio {
        scale: f64,
        #[serde(default)]
        denominator_aggregation: RatioDenominatorAggregation,
    },
    Median,
    Percentile {
        q: f64,
    },
    Stddev,
    DistinctCount,
}

impl SeedComputation {
    pub fn computation(self) -> MetricComputation {
        match self {
            Self::Sum => MetricComputation::Sum,
            Self::Ratio { .. } => MetricComputation::Ratio,
            Self::Median => MetricComputation::Median,
            Self::Percentile { .. } => MetricComputation::Percentile,
            Self::Stddev => MetricComputation::Stddev,
            Self::DistinctCount => MetricComputation::DistinctCount,
        }
    }

    /// The definition's `scale` storage column: the ratio's scale factor, or
    /// the percentile's quantile `q` — one numeric slot, meaning keyed by
    /// `computation_type` (the table's CHECK constraint enforces the pairing).
    pub fn scale(self) -> Option<f64> {
        match self {
            Self::Sum | Self::Median | Self::Stddev | Self::DistinctCount => None,
            Self::Ratio { scale, .. } => Some(scale),
            Self::Percentile { q } => Some(q),
        }
    }

    pub fn denominator_aggregation(self) -> RatioDenominatorAggregation {
        match self {
            Self::Ratio {
                denominator_aggregation,
                ..
            } => denominator_aggregation,
            Self::Sum
            | Self::Median
            | Self::Percentile { .. }
            | Self::Stddev
            | Self::DistinctCount => RatioDenominatorAggregation::Sum,
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
    /// How many days a delivered day keeps changing before it settles, when the
    /// suppliers revise one. Absent where nothing revises, or where nobody has
    /// established it — a reader treats absence as "settles on arrival" rather
    /// than as zero uncertainty. Where several suppliers feed one source the
    /// longest window wins: calling a settled day provisional costs less than
    /// the reverse.
    #[serde(default)]
    pub revision_window_days: Option<u16>,
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
    /// Declared per (source, measure): `active_day` is a per-day flag in
    /// `ai_usage` and a distinct-count subject in `collab`.
    #[serde(default)]
    pub alias_collapse: AliasCollapse,
    /// How this measure's evidence rows read in the drilldown. Absent where
    /// the rows carry no human-facing details of their own.
    #[serde(default)]
    pub evidence_presentation: Option<EvidencePresentation>,
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
    fn ratio_denominator_aggregation_defaults_to_sum_and_accepts_distinct_count() {
        let default: SeedComputation = serde_yaml::from_str("!ratio\nscale: 1")
            .unwrap_or_else(|error| panic!("default ratio must parse: {error}"));
        let distinct: SeedComputation =
            serde_yaml::from_str("!ratio\nscale: 1\ndenominator_aggregation: distinct_count")
                .unwrap_or_else(|error| panic!("distinct ratio must parse: {error}"));

        assert_eq!(
            default.denominator_aggregation(),
            RatioDenominatorAggregation::Sum
        );
        assert_eq!(
            distinct.denominator_aggregation(),
            RatioDenominatorAggregation::DistinctCount
        );
    }

    #[test]
    fn registry_declares_the_expected_counts() {
        assert_eq!(builtin_sources().len(), 7, "builtin source count");
        assert_eq!(builtin_metrics().len(), 105, "builtin metric count");
    }

    #[test]
    fn entity_types_round_trip_through_storage_values() {
        for entity_type in [EntityType::Person, EntityType::Tenant] {
            assert_eq!(EntityType::from_db(entity_type.as_db()), Some(entity_type));
        }
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

    /// Column keys the drilldown fills from the evidence row rather than from
    /// its details map; a declaration claiming one would be overwritten.
    const STRUCTURAL_COLUMN_KEYS: &[&str] = &["date", "value", "numerator", "denominator"];

    #[test]
    fn every_event_measure_declares_the_columns_its_rows_carry() {
        // An event row stands for one thing that happened, and the drilldown
        // projects it through this declaration alone. A measure that declares
        // nothing hands the reader a page of identical dates.
        let mut undeclared = Vec::new();
        for builtin_source in builtin_sources() {
            for measure in &builtin_source.measures {
                if measure.evidence_granularity != EvidenceGranularity::Event {
                    continue;
                }
                let declares_columns = measure
                    .evidence_presentation
                    .as_ref()
                    .is_some_and(|presentation| !presentation.detail_columns.is_empty());
                if !declares_columns {
                    undeclared.push(format!("{}/{}", builtin_source.source.key, measure.key));
                }
            }
        }
        assert!(
            undeclared.is_empty(),
            "event measures declaring no detail columns: {undeclared:?}"
        );
    }

    #[test]
    fn a_counted_pull_request_reads_the_request_and_where_it_was_headed() {
        for measure_key in [
            "pr_created",
            "pr_merged",
            "default_pr_created",
            "default_pr_merged",
        ] {
            let presentation = declared_presentation("git", measure_key);
            // Where the work went is part of the record on every path that
            // opens the dialog: a column that appeared only for a reader who
            // arrived through a grouped table left the same request looking
            // like a different one.
            assert_eq!(
                presentation
                    .detail_columns
                    .iter()
                    .map(|column| column.key.as_str())
                    .collect::<Vec<_>>(),
                [
                    "ref",
                    "title",
                    "repository",
                    "author",
                    "branch_scope",
                    "destination_branch"
                ],
                "{measure_key} should read the request it counted"
            );
            // The row IS the request it counted, so a value column would be 1s.
            assert!(
                !presentation.show_value,
                "{measure_key} should show no value"
            );
        }
    }

    #[test]
    fn a_branch_scope_split_reads_the_same_columns_as_its_total() {
        // The split measures count the same commits and requests the totals do,
        // so a reader who drills into "landed" sees what they saw before, minus
        // the rows that did not land.
        for (total, split) in [
            ("commit_count", "default_commit_count"),
            ("commit_count", "non_default_commit_count"),
            ("pr_created", "default_pr_created"),
            ("pr_created", "non_default_pr_created"),
            ("pr_merged", "default_pr_merged"),
            ("pr_merged", "non_default_pr_merged"),
        ] {
            assert_eq!(
                declared_presentation("git", split),
                declared_presentation("git", total),
                "{split} should read the same rows as {total}"
            );
        }
    }

    #[test]
    fn a_pull_request_duration_keeps_its_numeric_column() {
        for measure_key in [
            "pr_cycle_hours",
            "pr_change_size",
            "pr_first_review_hours",
            "pr_review_wait_share",
            "pr_review_to_merge_hours",
            "pr_approval_to_merge_hours",
        ] {
            // A duration or a page count is only readable with its number.
            assert!(
                declared_presentation("git", measure_key).show_value,
                "{measure_key} should show its value"
            );
        }
    }

    #[test]
    fn a_seat_row_names_the_month_it_bills_for() {
        // A seat row is dated at the day its snapshot was last read, not at the
        // month it bills for, so the month has to be a column of its own or the
        // reader cannot tell which month the row is.
        for measure_key in [
            "extra_usage_usd",
            "extra_usage_limit_usd",
            "seat_cost_usd",
            "daily_extra_usage_usd",
        ] {
            let presentation = declared_presentation("ai_cost", measure_key);
            assert_eq!(
                presentation.detail_columns[0].key, "billing_month",
                "{measure_key} should lead with the month it bills for"
            );
            assert!(presentation.show_value, "{measure_key} is an amount");
        }
    }

    #[test]
    fn a_seat_day_step_names_the_span_it_covers() {
        // The step is measured from the previous reading, so above one day the
        // figure is a span rather than one day's spend, and the month-to-date
        // total it was taken from is what makes it legible.
        let keys = declared_presentation("ai_cost", "daily_extra_usage_usd")
            .detail_columns
            .iter()
            .map(|column| column.key.clone())
            .collect::<Vec<_>>();
        assert_eq!(keys, ["billing_month", "month_to_date_usd", "covers_days"]);
    }

    #[test]
    fn ci_run_measures_read_the_run_and_a_deployment_reads_its_environment() {
        let runs = declared_presentation("ci", "runs");
        assert_eq!(
            runs.detail_columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            ["repository", "pipeline", "branch", "outcome"]
        );
        assert!(!runs.show_value, "a counted run needs no value column");

        for measure_key in ["run_duration_min", "run_hours"] {
            assert!(
                declared_presentation("ci", measure_key).show_value,
                "{measure_key} is unreadable without its number"
            );
        }

        let deployments = declared_presentation("ci", "deployments");
        assert_eq!(
            deployments
                .detail_columns
                .iter()
                .map(|column| (column.key.as_str(), column.label.as_str()))
                .collect::<Vec<_>>(),
            [
                ("repository", "Repository"),
                ("environment", "Environment"),
                ("outcome", "Outcome"),
                // Prose, because `env_kind` humanizes to "Env kind".
                ("env_kind", "Environment type"),
            ]
        );
    }

    fn declared_presentation(source_key: &str, measure_key: &str) -> EvidencePresentation {
        builtin_sources()
            .iter()
            .filter(|builtin_source| builtin_source.source.key == source_key)
            .flat_map(|builtin_source| &builtin_source.measures)
            .find(|measure| measure.key == measure_key)
            .and_then(|measure| measure.evidence_presentation.clone())
            .unwrap_or_else(|| panic!("{source_key}/{measure_key} declares no presentation"))
    }

    #[test]
    fn declared_detail_columns_are_uniquely_keyed_and_labelled() {
        for builtin_source in builtin_sources() {
            for measure in &builtin_source.measures {
                let Some(presentation) = measure.evidence_presentation.as_ref() else {
                    continue;
                };
                let mut seen = BTreeSet::new();
                for column in &presentation.detail_columns {
                    assert!(
                        is_snake_case(&column.key),
                        "{}/{} declares detail key {:?}",
                        builtin_source.source.key,
                        measure.key,
                        column.key
                    );
                    assert!(
                        !column.label.trim().is_empty(),
                        "{}/{} leaves detail key {:?} unlabelled",
                        builtin_source.source.key,
                        measure.key,
                        column.key
                    );
                    assert!(
                        seen.insert(column.key.as_str()),
                        "{}/{} declares detail key {:?} twice",
                        builtin_source.source.key,
                        measure.key,
                        column.key
                    );
                    assert!(
                        !STRUCTURAL_COLUMN_KEYS.contains(&column.key.as_str()),
                        "{}/{} declares detail key {:?}, which the drilldown \
                         already fills from the row itself",
                        builtin_source.source.key,
                        measure.key,
                        column.key
                    );
                }
            }
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

    // INVARIANT: a distribution statistic may not declare a collapse — median,
    // percentile and standard deviation are event-grain, and merging same-day
    // rows moves the statistic. Additive computations collapse by design, and
    // distinct counts are unaffected: they read `uniqExact(subject_key)` and the
    // collapse groups by `subject_key`, so the collapse is a no-op there.
    #[test]
    fn alias_collapse_is_never_declared_on_distribution_inputs() {
        let collapse_by_source: HashMap<&str, HashMap<&str, AliasCollapse>> = builtin_sources()
            .iter()
            .map(|builtin_source| {
                (
                    builtin_source.source.key.as_str(),
                    builtin_source
                        .measures
                        .iter()
                        .map(|measure| (measure.key.as_str(), measure.alias_collapse))
                        .collect(),
                )
            })
            .collect();

        for metric in builtin_metrics() {
            let collapses = collapse_by_source
                .get(metric.source_key.as_str())
                .unwrap_or_else(|| panic!("unknown source for {}", metric.metric_key));
            if !matches!(
                metric.computation,
                SeedComputation::Median
                    | SeedComputation::Percentile { .. }
                    | SeedComputation::Stddev
            ) {
                continue;
            }
            for input in &metric.inputs {
                let collapse = collapses
                    .get(input.measure_key.as_str())
                    .copied()
                    .unwrap_or_default();
                assert!(
                    !collapse.needs_pre_collapse(),
                    "{} is {:?} but binds {}, which declares alias_collapse: {}",
                    metric.metric_key,
                    metric.computation,
                    input.measure_key,
                    collapse.as_db(),
                );
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
    fn percentile_metrics_have_single_value_role_and_an_inside_quantile() {
        let mut seen = 0;
        for metric in builtin_metrics() {
            let SeedComputation::Percentile { q } = metric.computation else {
                continue;
            };
            seen += 1;
            assert!(
                (0.0..=1.0).contains(&q),
                "{} declares q={q}, outside [0, 1]",
                metric.metric_key
            );
            assert_eq!(metric.inputs.len(), 1, "{}", metric.metric_key);
            assert_eq!(
                metric.inputs[0].input_role,
                MetricInputRole::Value,
                "{}",
                metric.metric_key
            );
        }
        assert!(
            seen >= 1,
            "registry declares at least one percentile metric"
        );
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
