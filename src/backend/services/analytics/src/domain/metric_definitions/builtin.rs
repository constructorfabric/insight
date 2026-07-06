use crate::domain::metric_definitions::definition::{
    MetricComputation, MetricDirection, MetricFormat, MetricInputRole, ObservationSource,
    SourceKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeedComputation {
    Sum,
    Ratio { scale: f64 },
}

impl SeedComputation {
    pub fn computation(self) -> MetricComputation {
        match self {
            Self::Sum => MetricComputation::Sum,
            Self::Ratio { .. } => MetricComputation::Ratio,
        }
    }

    pub fn scale(self) -> Option<f64> {
        match self {
            Self::Sum => None,
            Self::Ratio { scale } => Some(scale),
        }
    }
}

pub struct SourceSeed {
    pub key: &'static str,
    pub kind: SourceKind,
    pub source_ref: ObservationSource,
}

pub struct BuiltinSource {
    pub source: SourceSeed,
    pub measures: &'static [&'static str],
    pub dimensions: &'static [&'static str],
}

pub struct MetricSeed {
    pub metric_key: &'static str,
    pub source_key: &'static str,
    pub label: &'static str,
    pub description: Option<&'static str>,
    pub explanation: Option<&'static str>,
    pub unit: Option<&'static str>,
    pub format: MetricFormat,
    pub direction: MetricDirection,
    pub entity_type: EntityType,
    pub computation: SeedComputation,
    pub peer_cohort_key: Option<CohortKey>,
    pub inputs: &'static [InputSeed],
    pub dimensions: &'static [&'static str],
}

pub struct InputSeed {
    pub input_role: MetricInputRole,
    pub measure_key: &'static str,
}

pub const BUILTIN_SOURCES: &[BuiltinSource] = &[BuiltinSource {
    source: SourceSeed {
        key: "ai_usage",
        kind: SourceKind::ManagedObservation,
        source_ref: ObservationSource::AiMetricObservations,
    },
    measures: &[
        "accepted_lines",
        "removed_lines",
        "active_day",
        "cost_usd",
        "accepted_edit_actions",
        "tool_use_offered",
        "assistant_messages",
        "assistant_actions",
        "dev_conversations",
        "chat_assistant_conversations",
    ],
    dimensions: &["tool", "surface"],
}];

pub const BUILTIN_METRICS: &[MetricSeed] = &[
    MetricSeed {
        metric_key: "ai.accepted_lines",
        source_key: "ai_usage",
        label: "AI-added lines",
        description: Some("Accepted added coding output"),
        explanation: Some("Accepted AI-generated added lines across coding AI tools."),
        unit: Some("lines"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "accepted_lines",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.removed_lines",
        source_key: "ai_usage",
        label: "AI-removed lines",
        description: Some("Accepted deleted coding output"),
        explanation: Some("Accepted AI-generated removed lines across coding AI tools."),
        unit: Some("lines"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "removed_lines",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.active_days",
        source_key: "ai_usage",
        label: "AI active days",
        description: Some("Days with any AI activity across dev and assistant tools"),
        explanation: Some(
            "Distinct days with person-attributed AI activity across dev and assistant tools.",
        ),
        unit: Some("days"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "active_day",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "ai.cost",
        source_key: "ai_usage",
        label: "AI cost",
        description: Some("Reported AI spend across dev and assistant tools"),
        explanation: Some(
            "Person-attributed AI spend across dev and assistant tools, where the connector reports cost.",
        ),
        unit: None,
        format: MetricFormat::Currency,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "cost_usd",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.accepted_edit_actions",
        source_key: "ai_usage",
        label: "Accepted AI edits",
        description: Some("Accepted tool or edit suggestions"),
        explanation: Some("Accepted AI edit or tool suggestions across supported coding AI tools."),
        unit: Some("actions"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "accepted_edit_actions",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.tool_acceptance_rate",
        source_key: "ai_usage",
        label: "AI tool acceptance",
        description: Some("Accepted divided by offered AI edits"),
        explanation: Some("Accepted AI edit or tool suggestions divided by offered suggestions."),
        unit: Some("percent"),
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "accepted_edit_actions",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "tool_use_offered",
            },
        ],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.assistant_messages",
        source_key: "ai_usage",
        label: "AI assistant messages",
        description: Some("Assistant messages"),
        explanation: Some(
            "Person-attributed assistant messages from supported AI assistant tools.",
        ),
        unit: Some("messages"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "assistant_messages",
        }],
        dimensions: &["tool", "surface"],
    },
    MetricSeed {
        metric_key: "ai.assistant_actions",
        source_key: "ai_usage",
        label: "AI assistant actions",
        description: Some("Assistant actions"),
        explanation: Some("Person-attributed assistant actions from supported AI assistant tools."),
        unit: Some("actions"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "assistant_actions",
        }],
        dimensions: &["tool", "surface"],
    },
    MetricSeed {
        metric_key: "ai.dev_conversations",
        source_key: "ai_usage",
        label: "AI dev conversations",
        description: Some("Coding tool conversations where the source reports them"),
        explanation: Some(
            "Person-attributed coding conversations from dev tools that report them.",
        ),
        unit: Some("conversations"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "dev_conversations",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.chat_assistant_conversations",
        source_key: "ai_usage",
        label: "AI chat conversations",
        description: Some("Chat assistant conversations"),
        explanation: Some(
            "Person-attributed chat assistant conversations from supported AI chat tools.",
        ),
        unit: Some("conversations"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "chat_assistant_conversations",
        }],
        dimensions: &["tool", "surface"],
    },
];

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
    fn source_keys_are_unique_and_shaped() {
        let mut seen = BTreeSet::new();
        for builtin_source in BUILTIN_SOURCES {
            assert!(is_snake_case(builtin_source.source.key));
            assert!(seen.insert(builtin_source.source.key));
        }
    }

    #[test]
    fn measure_and_dimension_keys_are_unique_per_source() {
        for builtin_source in BUILTIN_SOURCES {
            let mut measures = BTreeSet::new();
            for measure_key in builtin_source.measures {
                assert!(is_snake_case(measure_key));
                assert!(measures.insert(*measure_key));
            }
            let mut dimensions = BTreeSet::new();
            for dimension_key in builtin_source.dimensions {
                assert!(is_snake_case(dimension_key));
                assert!(dimensions.insert(*dimension_key));
            }
        }
    }

    #[test]
    fn metric_keys_are_unique_and_shaped() {
        let mut seen = BTreeSet::new();
        for metric in BUILTIN_METRICS {
            assert!(is_metric_key(metric.metric_key), "{}", metric.metric_key);
            assert!(seen.insert(metric.metric_key));
        }
    }

    #[test]
    fn metric_inputs_reference_declared_measures() {
        let measures_by_source: HashMap<&str, BTreeSet<&str>> = BUILTIN_SOURCES
            .iter()
            .map(|builtin_source| {
                (
                    builtin_source.source.key,
                    builtin_source.measures.iter().copied().collect(),
                )
            })
            .collect();

        for metric in BUILTIN_METRICS {
            let measures = measures_by_source
                .get(metric.source_key)
                .unwrap_or_else(|| panic!("unknown source for {}", metric.metric_key));
            assert!(!metric.inputs.is_empty(), "{}", metric.metric_key);
            for input in metric.inputs {
                assert!(
                    measures.contains(input.measure_key),
                    "{} references undeclared measure {}",
                    metric.metric_key,
                    input.measure_key
                );
            }
        }
    }

    #[test]
    fn metric_dimensions_reference_declared_source_dimensions() {
        let dimensions_by_source: HashMap<&str, BTreeSet<&str>> = BUILTIN_SOURCES
            .iter()
            .map(|builtin_source| {
                (
                    builtin_source.source.key,
                    builtin_source.dimensions.iter().copied().collect(),
                )
            })
            .collect();

        for metric in BUILTIN_METRICS {
            let Some(dimensions) = dimensions_by_source.get(metric.source_key) else {
                panic!("unknown source for {}", metric.metric_key);
            };
            for dimension in metric.dimensions {
                assert!(
                    dimensions.contains(dimension),
                    "{} references undeclared dimension {dimension}",
                    metric.metric_key
                );
            }
        }
    }

    #[test]
    fn ratio_metrics_have_numerator_and_denominator_roles() {
        for metric in BUILTIN_METRICS {
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
}
