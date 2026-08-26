//! Validates definitions against the field catalog — the stage that decides
//! whether a well-formed definition is a *true* one: does the field exist, may
//! it hold the role the definition uses it in, does the expression name only
//! catalogued columns, does every reference resolve.
//!
//! One implementation serves both write paths: product definitions run it at
//! build time (an invalid one fails CI and cannot ship) and authored ones run
//! it at write time, so an invalid definition is never stored.

use std::collections::BTreeSet;

use super::definition::{Computation, MeasureDefinition, MetricDefinition, Operand};
use super::expr::{ScalarExprError, validate_scalar_expr};
use super::filter::FilterError;
use crate::domain::field_catalog::model::{CatalogDataset, FieldCatalog, FieldRole};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValidationError {
    #[error("`{0}` must be lowercase snake_case starting with [a-z]")]
    KeyShape(String),
    #[error("metric key `{0}` must be `subject.name`")]
    MetricKeyShape(String),
    #[error("`{0}` is defined twice")]
    DuplicateKey(String),
    #[error("measure `{measure}` reads dataset `{dataset}`, which the catalog does not have")]
    DatasetNotFound { measure: String, dataset: String },
    #[error("measure `{measure}` names field `{field}`, absent from dataset `{dataset}`")]
    FieldNotFound {
        measure: String,
        dataset: String,
        field: String,
    },
    #[error("measure `{measure}` uses `{field}` as its {role}, a role that field does not carry")]
    RoleMismatch {
        measure: String,
        field: String,
        role: String,
    },
    #[error("measure `{measure}` filter is malformed: {source}")]
    Filter {
        measure: String,
        #[source]
        source: FilterError,
    },
    #[error("measure `{measure}` {slot} is not admissible: {source}")]
    Expression {
        measure: String,
        slot: &'static str,
        #[source]
        source: ScalarExprError,
    },
    #[error("measure `{measure}` aggregates with `{aggregation}`, which requires {requirement}")]
    Operand {
        measure: String,
        aggregation: &'static str,
        requirement: &'static str,
    },
    #[error("metric `{metric}` reads measure `{measure}`, which is not defined")]
    MeasureNotFound { metric: String, measure: String },
    #[error("metric `{metric}` asks for quantile {quantile}, which is not inside (0, 1)")]
    QuantileOutOfRange { metric: String, quantile: f64 },
    #[error("metric `{metric}` composes measures over different datasets: `{a}` and `{b}`")]
    MixedDatasets {
        metric: String,
        a: String,
        b: String,
    },
}

/// Validate a whole definition set together: every cross-reference resolves
/// within it, so a set that validates can be reconciled in any order.
pub fn validate_definitions(
    catalog: &FieldCatalog,
    measures: &[MeasureDefinition],
    metrics: &[MetricDefinition],
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    let mut seen = BTreeSet::new();
    for measure in measures {
        if !seen.insert(measure.key.as_str()) {
            errors.push(ValidationError::DuplicateKey(measure.key.clone()));
        }
        validate_measure(catalog, measure, &mut errors);
    }

    let mut seen = BTreeSet::new();
    for metric in metrics {
        if !seen.insert(metric.key.as_str()) {
            errors.push(ValidationError::DuplicateKey(metric.key.clone()));
        }
        validate_metric(metric, measures, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_measure(
    catalog: &FieldCatalog,
    measure: &MeasureDefinition,
    errors: &mut Vec<ValidationError>,
) {
    if !is_key(&measure.key) {
        errors.push(ValidationError::KeyShape(measure.key.clone()));
    }

    let Some(dataset) = catalog.dataset(&measure.dataset) else {
        errors.push(ValidationError::DatasetNotFound {
            measure: measure.key.clone(),
            dataset: measure.dataset.clone(),
        });
        return;
    };

    validate_roles(dataset, measure, errors);
    validate_filter(dataset, measure, errors);
    validate_expressions(dataset, measure, errors);
    validate_operand(measure, errors);
}

fn missing_field(
    measure: &MeasureDefinition,
    dataset: &CatalogDataset,
    field: String,
) -> ValidationError {
    ValidationError::FieldNotFound {
        measure: measure.key.clone(),
        dataset: dataset.key.clone(),
        field,
    }
}

fn validate_roles(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    errors: &mut Vec<ValidationError>,
) {
    let mut require =
        |field: &str, role: FieldRole, role_name: &'static str| match dataset.field(field) {
            None => errors.push(missing_field(measure, dataset, field.to_owned())),
            Some(catalogued) if catalogued.role != Some(role) => {
                errors.push(ValidationError::RoleMismatch {
                    measure: measure.key.clone(),
                    field: field.to_owned(),
                    role: role_name.to_owned(),
                });
            }
            Some(_) => {}
        };

    require(&measure.event_time, FieldRole::EventTime, "event time");
    require(&measure.entity, FieldRole::Entity, "entity");
    for binding in &measure.dimensions {
        require(&binding.value_field, FieldRole::Dimension, "dimension");
    }

    for binding in &measure.dimensions {
        if let Some(label) = &binding.label_field
            && dataset.field(label).is_none()
        {
            errors.push(missing_field(measure, dataset, label.clone()));
        }
    }
}

fn validate_filter(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    errors: &mut Vec<ValidationError>,
) {
    let Some(filter) = &measure.filter else {
        return;
    };
    match filter.validate() {
        Err(source) => errors.push(ValidationError::Filter {
            measure: measure.key.clone(),
            source,
        }),
        Ok(()) => {
            for field in filter.referenced_fields() {
                if dataset.field(field).is_none() {
                    errors.push(missing_field(measure, dataset, field.to_owned()));
                }
            }
        }
    }
}

fn validate_expressions(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    errors: &mut Vec<ValidationError>,
) {
    for (slot, expression) in [
        ("value_expr", measure.value_expr.as_deref()),
        ("subject_expr", measure.subject_expr.as_deref()),
    ] {
        let Some(expression) = expression else {
            continue;
        };
        match validate_scalar_expr(expression) {
            Err(source) => errors.push(ValidationError::Expression {
                measure: measure.key.clone(),
                slot,
                source,
            }),
            Ok(parsed) => {
                for column in parsed.columns {
                    if dataset.field(&column).is_none() {
                        errors.push(missing_field(measure, dataset, column));
                    }
                }
            }
        }
    }
}

fn validate_operand(measure: &MeasureDefinition, errors: &mut Vec<ValidationError>) {
    let operand = measure.aggregation.operand();
    let (has_value, has_subject) = (measure.value_expr.is_some(), measure.subject_expr.is_some());
    let satisfied = match operand {
        Operand::None => !has_value && !has_subject,
        Operand::Value => has_value && !has_subject,
        Operand::Subject => has_subject && !has_value,
    };
    if !satisfied {
        errors.push(ValidationError::Operand {
            measure: measure.key.clone(),
            aggregation: measure.aggregation.as_db(),
            requirement: match operand {
                Operand::None => "neither expression",
                Operand::Value => "a value expression and no subject expression",
                Operand::Subject => "a subject expression and no value expression",
            },
        });
    }
}

fn validate_metric(
    metric: &MetricDefinition,
    measures: &[MeasureDefinition],
    errors: &mut Vec<ValidationError>,
) {
    if !is_metric_key(&metric.key) {
        errors.push(ValidationError::MetricKeyShape(metric.key.clone()));
    }
    if let Some(cohort) = &metric.cohort_key
        && !is_key(cohort)
    {
        errors.push(ValidationError::KeyShape(cohort.clone()));
    }
    if let Computation::Percentile { quantile, .. } = metric.computation
        && !(quantile > 0.0 && quantile < 1.0)
    {
        errors.push(ValidationError::QuantileOutOfRange {
            metric: metric.key.clone(),
            quantile,
        });
    }

    let mut datasets: Vec<(&str, &str)> = Vec::new();
    for input in metric.input_measures() {
        match measures.iter().find(|measure| measure.key == input) {
            None => errors.push(ValidationError::MeasureNotFound {
                metric: metric.key.clone(),
                measure: input.to_owned(),
            }),
            Some(measure) => datasets.push((input, measure.dataset.as_str())),
        }
    }

    // Cross-dataset composition joins aggregates on entity, time bucket, and
    // conformed dimensions — capability the compiler does not have yet, so a
    // definition may not ask for it.
    if let Some((first_key, first_dataset)) = datasets.first()
        && let Some((other_key, _)) = datasets
            .iter()
            .find(|(_, dataset)| dataset != first_dataset)
    {
        errors.push(ValidationError::MixedDatasets {
            metric: metric.key.clone(),
            a: (*first_key).to_owned(),
            b: (*other_key).to_owned(),
        });
    }
}

fn is_key(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn is_metric_key(value: &str) -> bool {
    match value.split_once('.') {
        Some((subject, name)) => is_key(subject) && is_key(name),
        None => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::domain::definitions::definition::{
        Aggregation, DimensionBinding, Direction, Format,
    };
    use crate::domain::field_catalog::loader;

    const SNAPSHOT: &str = r#"[
      {
        "database": "silver",
        "relation": "class_git_pull_requests",
        "engine": "ReplacingMergeTree",
        "sorting_key": "unique_key",
        "columns": [
          {"name": "tenant_id", "type": "Nullable(String)"},
          {"name": "author_email", "type": "String"},
          {"name": "created_on", "type": "Nullable(DateTime)"},
          {"name": "state", "type": "String"},
          {"name": "repo_slug", "type": "String"},
          {"name": "lines_added", "type": "Nullable(Int64)"},
          {"name": "title", "type": "String"}
        ]
      }
    ]"#;

    const ROLES: &str = "
datasets:
  - key: git_pull_requests
    database: silver
    relation: class_git_pull_requests
    fields:
      tenant_id: tenant
      author_email: entity
      created_on: event_time
      state: dimension
      repo_slug: dimension
      lines_added: measurable
      title:
        display: [title]
";

    fn catalog() -> FieldCatalog {
        loader::load(SNAPSHOT, ROLES).expect("catalog loads")
    }

    fn measure() -> MeasureDefinition {
        MeasureDefinition {
            key: "prs_created".to_owned(),
            dataset: "git_pull_requests".to_owned(),
            description: None,
            filter: None,
            aggregation: Aggregation::Count,
            value_expr: None,
            subject_expr: None,
            event_time: "created_on".to_owned(),
            entity: "author_email".to_owned(),
            dimensions: vec![DimensionBinding {
                key: "repository".to_owned(),
                value_field: "repo_slug".to_owned(),
                label_field: None,
            }],
        }
    }

    fn metric(computation: Computation) -> MetricDefinition {
        MetricDefinition {
            key: "git.prs_created".to_owned(),
            computation,
            transform: None,
            format: Format::Integer,
            direction: Direction::HigherIsBetter,
            entity_type: "person".to_owned(),
            cohort_key: None,
            label: None,
            description: None,
        }
    }

    fn errors_for(
        measures: &[MeasureDefinition],
        metrics: &[MetricDefinition],
    ) -> Vec<ValidationError> {
        validate_definitions(&catalog(), measures, metrics).expect_err("expected errors")
    }

    #[test]
    fn a_definition_set_that_resolves_validates() {
        let direct = metric(Computation::Direct {
            measure: "prs_created".to_owned(),
        });
        validate_definitions(&catalog(), &[measure()], &[direct]).expect("validates");
    }

    #[test]
    fn a_measure_over_an_uncatalogued_dataset_is_rejected() {
        let mut measure = measure();
        measure.dataset = "git_tags".to_owned();
        assert_eq!(
            errors_for(&[measure], &[]),
            [ValidationError::DatasetNotFound {
                measure: "prs_created".to_owned(),
                dataset: "git_tags".to_owned(),
            }]
        );
    }

    #[test]
    fn a_field_used_in_the_wrong_role_is_rejected() {
        let mut measure = measure();
        measure.entity = "repo_slug".to_owned();
        assert_eq!(
            errors_for(&[measure], &[]),
            [ValidationError::RoleMismatch {
                measure: "prs_created".to_owned(),
                field: "repo_slug".to_owned(),
                role: "entity".to_owned(),
            }]
        );
    }

    #[test]
    fn a_filter_or_expression_naming_an_absent_column_is_rejected() {
        let mut filtered = measure();
        filtered.filter = serde_yaml::from_str("{ field: merged_at, op: not_null }").ok();
        assert!(errors_for(&[filtered], &[]).iter().any(|error| matches!(
            error,
            ValidationError::FieldNotFound { field, .. } if field == "merged_at"
        )));

        let mut summed = measure();
        summed.aggregation = Aggregation::Sum;
        summed.value_expr = Some("lines_added + lines_deleted".to_owned());
        assert!(errors_for(&[summed], &[]).iter().any(|error| matches!(
            error,
            ValidationError::FieldNotFound { field, .. } if field == "lines_deleted"
        )));
    }

    #[test]
    fn an_expression_the_allowlist_refuses_is_rejected() {
        let mut summed = measure();
        summed.aggregation = Aggregation::Sum;
        summed.value_expr = Some("sleep(1)".to_owned());
        assert!(
            errors_for(&[summed], &[])
                .iter()
                .any(|error| matches!(error, ValidationError::Expression { .. }))
        );
    }

    #[test]
    fn an_aggregation_without_its_operand_is_rejected() {
        let mut summed = measure();
        summed.aggregation = Aggregation::Sum;
        assert_eq!(
            errors_for(&[summed], &[]),
            [ValidationError::Operand {
                measure: "prs_created".to_owned(),
                aggregation: "sum",
                requirement: "a value expression and no subject expression",
            }]
        );

        let mut counted = measure();
        counted.value_expr = Some("lines_added".to_owned());
        assert!(
            errors_for(&[counted], &[])
                .iter()
                .any(|error| matches!(error, ValidationError::Operand { .. }))
        );
    }

    #[test]
    fn a_metric_reading_an_undefined_measure_is_rejected() {
        let orphan = metric(Computation::Direct {
            measure: "prs_reviewed".to_owned(),
        });
        assert_eq!(
            errors_for(&[measure()], &[orphan]),
            [ValidationError::MeasureNotFound {
                metric: "git.prs_created".to_owned(),
                measure: "prs_reviewed".to_owned(),
            }]
        );
    }

    #[test]
    fn a_quantile_outside_the_open_unit_interval_is_rejected() {
        for quantile in [0.0, 1.0, 1.5, -0.5] {
            let out_of_range = metric(Computation::Percentile {
                measure: "prs_created".to_owned(),
                quantile,
            });
            assert!(
                errors_for(&[measure()], &[out_of_range])
                    .iter()
                    .any(|error| matches!(error, ValidationError::QuantileOutOfRange { .. })),
                "{quantile}"
            );
        }
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        assert!(
            errors_for(&[measure(), measure()], &[])
                .iter()
                .any(|error| matches!(error, ValidationError::DuplicateKey(_)))
        );
    }

    #[test]
    fn malformed_keys_are_rejected() {
        let mut measure = measure();
        measure.key = "PRs Created".to_owned();
        assert!(
            errors_for(&[measure], &[])
                .iter()
                .any(|error| matches!(error, ValidationError::KeyShape(_)))
        );

        let mut metric = metric(Computation::Direct {
            measure: "prs_created".to_owned(),
        });
        metric.key = "prs_created".to_owned();
        assert!(
            errors_for(&[super::tests::measure()], &[metric])
                .iter()
                .any(|error| matches!(error, ValidationError::MetricKeyShape(_)))
        );
    }
}
