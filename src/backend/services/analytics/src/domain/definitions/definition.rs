//! The typed definition format. Authored YAML and stored rows are
//! serializations of these types, and every consumer — parser, validators,
//! store, editing API — reads the same ones, so the format is defined once.

use serde::{Deserialize, Serialize};

use super::filter::FilterTree;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    CountDistinct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Product,
    Custom,
}

/// Binds a dimension key to the fields that carry its value and, where the
/// value key is not presentable on its own, its label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionBinding {
    pub key: String,
    pub value_field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_field: Option<String>,
}

/// How a relation must be read for its rows to be the deduplicated truth. A
/// replacing engine keeps superseded rows until a merge that may never come,
/// so every read of one must collapse them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadDiscipline {
    /// Collapse duplicates at read time.
    Final,
    /// Rows are already unique; read them directly.
    None,
}

/// A queryable relation with guaranteed semantics — deduplicated,
/// tenant-scoped, stable columns — that measures aggregate over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetDefinition {
    pub key: String,
    /// Where the rows live, as `database.relation`.
    pub relation: String,
    pub read_discipline: ReadDiscipline,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// How far back the dataset's history reaches, as a served contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_horizon: Option<String>,
}

/// A declarative aggregation of one dataset — the lowest editable layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureDefinition {
    pub key: String,
    pub dataset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<FilterTree>,
    pub aggregation: Aggregation,
    /// The operand for the numeric folds; absent for `count`/`count_distinct`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_expr: Option<String>,
    /// What `count_distinct` counts one of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_expr: Option<String>,
    pub event_time: String,
    pub entity: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<DimensionBinding>,
}

/// How a metric turns measures into a served value. The `MetricFlow` metric
/// types are the reference vocabulary; percentiles are metric-level because a
/// percentile of pre-aggregated values is not a percentile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Computation {
    Direct {
        measure: String,
    },
    Ratio {
        numerator: String,
        denominator: String,
    },
    Percentile {
        measure: String,
        /// The quantile to serve, in `(0, 1)`; `0.5` is the median.
        quantile: f64,
    },
}

/// Post-aggregation shaping: `clamp(min, max, multiplier * value + offset)`.
/// Absent fields are the identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clamp_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clamp_max: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Integer,
    Decimal,
    Currency,
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    HigherIsBetter,
    LowerIsBetter,
    Neutral,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricDefinition {
    pub key: String,
    pub computation: Computation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    pub format: Format,
    pub direction: Direction,
    pub entity_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cohort_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Aggregation {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
            Self::CountDistinct => "count_distinct",
        }
    }

    /// Which expression this aggregation requires, mirroring the store's
    /// `chk_semantic_measures_aggregation_expr` biconditional.
    pub fn operand(self) -> Operand {
        match self {
            Self::Count => Operand::None,
            Self::CountDistinct => Operand::Subject,
            Self::Sum | Self::Avg | Self::Min | Self::Max => Operand::Value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    None,
    Value,
    Subject,
}

impl ReadDiscipline {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::None => "none",
        }
    }
}

impl Origin {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Custom => "custom",
        }
    }
}

impl Format {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Decimal => "decimal",
            Self::Currency => "currency",
            Self::Percent => "percent",
        }
    }
}

impl Direction {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::HigherIsBetter => "higher_is_better",
            Self::LowerIsBetter => "lower_is_better",
            Self::Neutral => "neutral",
        }
    }
}

impl MetricDefinition {
    /// Every measure key this metric reads, for reference checking.
    pub fn input_measures(&self) -> Vec<&str> {
        match &self.computation {
            Computation::Direct { measure } | Computation::Percentile { measure, .. } => {
                vec![measure.as_str()]
            }
            Computation::Ratio {
                numerator,
                denominator,
            } => vec![numerator.as_str(), denominator.as_str()],
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_measure_round_trips_through_yaml() {
        let yaml = r"
key: large_prs_merged
dataset: git_pull_requests
description: Merged pull requests changing at least 500 lines.
filter:
  all:
    - { field: state, op: eq, value: merged }
    - { field: lines_changed, op: gte, value: 500 }
aggregation: count
event_time: closed_on
entity: author_email
dimensions:
  - { key: repository, value_field: repo_slug }
  - { key: source, value_field: data_source, label_field: data_source_label }
";
        let measure: MeasureDefinition = serde_yaml::from_str(yaml).expect("parses");
        assert_eq!(measure.aggregation, Aggregation::Count);
        assert_eq!(measure.aggregation.operand(), Operand::None);
        assert_eq!(measure.dimensions.len(), 2);
        assert_eq!(
            measure.dimensions[1].label_field.as_deref(),
            Some("data_source_label")
        );

        let round_tripped: MeasureDefinition =
            serde_json::from_str(&serde_json::to_string(&measure).unwrap()).unwrap();
        assert_eq!(round_tripped, measure);
    }

    #[test]
    fn unknown_keys_are_rejected_in_every_definition_shape() {
        assert!(
            serde_yaml::from_str::<MeasureDefinition>(
                "key: k\ndataset: d\naggregation: count\nevent_time: t\nentity: e\nextra: 1\n"
            )
            .is_err()
        );
        assert!(serde_yaml::from_str::<Transform>("multiplier: 2\nclamp: 1\n").is_err());
        assert!(
            serde_yaml::from_str::<Computation>("type: direct\nmeasure: m\nscale: 2\n").is_err()
        );
    }

    #[test]
    fn computation_names_its_input_measures() {
        let ratio: Computation =
            serde_yaml::from_str("type: ratio\nnumerator: merged\ndenominator: created\n")
                .expect("parses");
        let metric = MetricDefinition {
            key: "git.merge_rate".to_owned(),
            computation: ratio,
            transform: None,
            format: Format::Percent,
            direction: Direction::HigherIsBetter,
            entity_type: "person".to_owned(),
            cohort_key: None,
            label: None,
            description: None,
        };
        assert_eq!(metric.input_measures(), ["merged", "created"]);
    }

    #[test]
    fn aggregation_operands_match_the_store_biconditional() {
        assert_eq!(Aggregation::Count.operand(), Operand::None);
        assert_eq!(Aggregation::CountDistinct.operand(), Operand::Subject);
        for aggregation in [
            Aggregation::Sum,
            Aggregation::Avg,
            Aggregation::Min,
            Aggregation::Max,
        ] {
            assert_eq!(aggregation.operand(), Operand::Value);
        }
    }
}
