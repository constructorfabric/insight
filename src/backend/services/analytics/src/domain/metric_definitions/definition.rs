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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationSource {
    AiMetricObservations,
}

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricInput {
    pub role: MetricInputRole,
    pub observation_source: ObservationSource,
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

    pub fn is_zero_filled(&self) -> bool {
        matches!(self.spec, ComputationSpec::Sum { .. })
    }

    pub fn observation_source(&self) -> ObservationSource {
        match &self.spec {
            ComputationSpec::Sum { value } => value.observation_source,
            ComputationSpec::Ratio { numerator, .. } => numerator.observation_source,
        }
    }
}

impl ObservationSource {
    pub fn source_ref(self) -> &'static str {
        match self {
            Self::AiMetricObservations => "ai_metric_observations",
        }
    }

    pub fn from_ref(value: &str) -> Option<Self> {
        match value {
            "ai_metric_observations" => Some(Self::AiMetricObservations),
            _ => None,
        }
    }

    pub fn table_ref(self) -> (&'static str, &'static str) {
        match self {
            Self::AiMetricObservations => ("insight", "ai_metric_observations"),
        }
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
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "sum" => Some(Self::Sum),
            "ratio" => Some(Self::Ratio),
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
        for computation in [MetricComputation::Sum, MetricComputation::Ratio] {
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
        let source = ObservationSource::AiMetricObservations;
        assert_eq!(
            ObservationSource::from_ref(source.source_ref()),
            Some(source)
        );
        for kind in [
            SourceKind::ManagedObservation,
            SourceKind::CustomObservationSql,
        ] {
            assert_eq!(SourceKind::from_db(kind.as_db()), Some(kind));
        }
    }
}
