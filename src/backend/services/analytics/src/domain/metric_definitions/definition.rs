use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    HigherIsBetter,
    LowerIsBetter,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricFormat {
    Integer,
    Decimal,
    Currency,
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricComputation {
    Sum,
    Ratio,
    Median,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricInputRole {
    Value,
    Numerator,
    Denominator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    ManagedObservation,
    CustomObservationSql,
}

impl SourceKind {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::ManagedObservation => "managed_observation",
            Self::CustomObservationSql => "custom_observation_sql",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "managed_observation" => Some(Self::ManagedObservation),
            "custom_observation_sql" => Some(Self::CustomObservationSql),
            _ => None,
        }
    }
}

/// Warehouse relation an observation source reads from, stored as data in
/// `metric_sources.source_ref` and validated on load. The compile-time gate
/// is the shape of the name (`<family>_metric_observations` in the `insight`
/// database); the runtime gate is the schema probe, which checks the actual
/// columns before the source becomes available. Adding a source is therefore
/// a dbt model plus registry seed rows — no code change here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationRelation(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CohortSource {
    MetricEntityCohortsCurrent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricDefinition {
    pub base: MetricBase,
    pub spec: ComputationSpec,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricBase {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub explanation: Option<String>,
    pub entity_type: String,
    pub format: MetricFormat,
    pub unit: Option<String>,
    pub direction: MetricDirection,
    pub peer_cohort_key: Option<String>,
    pub allowed_dimensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComputationSpec {
    Sum {
        value: MetricInput,
    },
    Ratio {
        numerator: MetricInput,
        denominator: MetricInput,
        scale: f64,
    },
    /// Exact middle of per-event observation values. Median measures emit
    /// one row per source event (multiple rows per entity/day are the
    /// intended shape), so the aggregate is over events, not day totals.
    Median {
        value: MetricInput,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricInput {
    pub role: MetricInputRole,
    pub observation_relation: ObservationRelation,
    pub source_key: String,
    pub measure_key: String,
}

impl MetricDefinition {
    pub fn key(&self) -> &str {
        self.base.key.as_str()
    }

    pub fn allowed_dimension(&self, dimension: &str) -> Option<&str> {
        self.base
            .allowed_dimensions
            .iter()
            .map(String::as_str)
            .find(|d| *d == dimension)
    }

    // Only sums zero-fill: an absent entity genuinely summed to nothing.
    // Ratios and medians of no observations are unknowable, not zero.
    pub fn is_zero_filled(&self) -> bool {
        matches!(self.spec, ComputationSpec::Sum { .. })
    }

    pub fn observation_relation(&self) -> &ObservationRelation {
        match &self.spec {
            ComputationSpec::Sum { value } | ComputationSpec::Median { value } => {
                &value.observation_relation
            }
            ComputationSpec::Ratio { numerator, .. } => &numerator.observation_relation,
        }
    }
}

impl ObservationRelation {
    pub const DATABASE: &'static str = "insight";

    /// Accepts exactly the managed-observation naming shape:
    /// lowercase `snake_case` ending in `_metric_observations`, with a
    /// non-empty family prefix. Anything else is a configuration error.
    pub fn parse(value: &str) -> Option<Self> {
        let family = value.strip_suffix("_metric_observations")?;
        if family.is_empty() {
            return None;
        }
        let mut chars = family.chars();
        let starts_alpha = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        let rest_ok = family
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if starts_alpha && rest_ok {
            Some(Self(value.to_owned()))
        } else {
            None
        }
    }

    pub fn table_ref(&self) -> (&'static str, &str) {
        (Self::DATABASE, &self.0)
    }

    /// The stored relation name, as written to `metric_sources.source_ref`.
    /// Used to group same-source metrics for batched queries.
    pub fn source_ref(&self) -> &str {
        &self.0
    }
}

impl CohortSource {
    pub fn table_ref(self) -> (&'static str, &'static str) {
        match self {
            Self::MetricEntityCohortsCurrent => ("insight", "metric_entity_cohorts_current"),
        }
    }
}

impl MetricFormat {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Decimal => "decimal",
            Self::Currency => "currency",
            Self::Percent => "percent",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "integer" => Some(Self::Integer),
            "decimal" => Some(Self::Decimal),
            "currency" => Some(Self::Currency),
            "percent" => Some(Self::Percent),
            _ => None,
        }
    }
}

impl MetricDirection {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::HigherIsBetter => "higher_is_better",
            Self::LowerIsBetter => "lower_is_better",
            Self::Neutral => "neutral",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "higher_is_better" => Some(Self::HigherIsBetter),
            "lower_is_better" => Some(Self::LowerIsBetter),
            "neutral" => Some(Self::Neutral),
            _ => None,
        }
    }
}

impl MetricComputation {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Ratio => "ratio",
            Self::Median => "median",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "sum" => Some(Self::Sum),
            "ratio" => Some(Self::Ratio),
            "median" => Some(Self::Median),
            _ => None,
        }
    }
}

impl MetricInputRole {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Numerator => "numerator",
            Self::Denominator => "denominator",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "value" => Some(Self::Value),
            "numerator" => Some(Self::Numerator),
            "denominator" => Some(Self::Denominator),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_relation_pins_the_naming_shape() {
        for valid in [
            "ai_metric_observations",
            "git_metric_observations",
            "task_tracking2_metric_observations",
        ] {
            let relation =
                ObservationRelation::parse(valid).unwrap_or_else(|| panic!("{valid} must parse"));
            assert_eq!(relation.table_ref(), ("insight", valid));
        }
        for invalid in [
            "",
            "_metric_observations",
            "metric_observations",
            "ai_metric_observations2",
            "Ai_metric_observations",
            "2ai_metric_observations",
            "ai-metric_observations",
            "insight.ai_metric_observations",
            "ai_metric_observations; DROP TABLE x",
        ] {
            assert!(
                ObservationRelation::parse(invalid).is_none(),
                "{invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn db_strings_round_trip() {
        for format in [
            MetricFormat::Integer,
            MetricFormat::Decimal,
            MetricFormat::Currency,
            MetricFormat::Percent,
        ] {
            assert_eq!(MetricFormat::from_db(format.as_db()), Some(format));
        }
        for direction in [
            MetricDirection::HigherIsBetter,
            MetricDirection::LowerIsBetter,
            MetricDirection::Neutral,
        ] {
            assert_eq!(MetricDirection::from_db(direction.as_db()), Some(direction));
        }
        for computation in [
            MetricComputation::Sum,
            MetricComputation::Ratio,
            MetricComputation::Median,
        ] {
            assert_eq!(
                MetricComputation::from_db(computation.as_db()),
                Some(computation)
            );
        }
        for role in [
            MetricInputRole::Value,
            MetricInputRole::Numerator,
            MetricInputRole::Denominator,
        ] {
            assert_eq!(MetricInputRole::from_db(role.as_db()), Some(role));
        }
        let relation = ObservationRelation::parse("ai_metric_observations")
            .unwrap_or_else(|| panic!("builtin relation name must parse"));
        let (_, table) = relation.table_ref();
        assert_eq!(
            ObservationRelation::parse(table).as_ref(),
            Some(&relation),
            "table name must round-trip through parse"
        );
        for kind in [
            SourceKind::ManagedObservation,
            SourceKind::CustomObservationSql,
        ] {
            assert_eq!(SourceKind::from_db(kind.as_db()), Some(kind));
        }
    }
}
