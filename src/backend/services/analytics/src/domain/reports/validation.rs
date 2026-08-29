use std::collections::{BTreeSet, HashMap};

use chrono::{Datelike, NaiveDate};
use sea_orm::DatabaseConnection;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::domain::metric_definitions::{MetricDefinition, load_definitions};
use crate::domain::metric_results::normalize_metric_key;

use super::dto::{
    ReportExportFormat, ReportExportRequest, ReportGranularity, ReportPreviewRequest, ReportRecipe,
    ReportSubject,
};

const MAX_REPORT_PEOPLE: usize = 1000;
const MAX_REPORT_METRICS: usize = 100;

#[derive(Debug)]
pub struct ValidatedReportRecipe {
    pub subject: ReportSubjectSelection,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub granularity: ReportGranularity,
    pub metrics: Vec<MetricDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportSubjectSelection {
    People { ids: Vec<Uuid> },
    Tenant { id: Uuid },
}

impl ReportSubjectSelection {
    fn entity_type(&self) -> &'static str {
        match self {
            Self::People { .. } => "person",
            Self::Tenant { .. } => "tenant",
        }
    }
}

struct RecipeShape {
    subject: ReportSubjectSelection,
    from: NaiveDate,
    to: NaiveDate,
    granularity: ReportGranularity,
    metric_keys: Vec<String>,
}

pub async fn validate_preview(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    request: ReportPreviewRequest,
) -> Result<ValidatedReportRecipe, CanonicalError> {
    validate_recipe(db, tenant_id, request).await
}

pub async fn validate_export(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    request: ReportExportRequest,
) -> Result<ValidatedReportRecipe, CanonicalError> {
    let shape = validate_export_shape(request, tenant_id)?;
    validate_recipe_definitions(db, tenant_id, shape).await
}

async fn validate_recipe(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    request: ReportRecipe,
) -> Result<ValidatedReportRecipe, CanonicalError> {
    let shape = validate_recipe_shape(request, tenant_id)?;
    validate_recipe_definitions(db, tenant_id, shape).await
}

async fn validate_recipe_definitions(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    shape: RecipeShape,
) -> Result<ValidatedReportRecipe, CanonicalError> {
    let definitions = load_definitions(db, tenant_id, &shape.metric_keys).await?;
    let metrics = validate_loaded_definitions(&shape, &definitions)?;

    Ok(ValidatedReportRecipe {
        subject: shape.subject,
        from: shape.from,
        to: shape.to,
        granularity: shape.granularity,
        metrics,
    })
}

fn validate_export_shape(
    request: ReportExportRequest,
    tenant_id: Uuid,
) -> Result<RecipeShape, CanonicalError> {
    let format = request.format;
    let shape = validate_recipe_shape(request.into_recipe(), tenant_id)?;
    if matches!(format, ReportExportFormat::Xlsx) && shape.from.year() < 1900 {
        return invalid(
            "period.from",
            "XLSX exports require period.from on or after 1900-01-01",
        );
    }

    Ok(shape)
}

fn validate_recipe_shape(
    request: ReportRecipe,
    tenant_id: Uuid,
) -> Result<RecipeShape, CanonicalError> {
    let subject = match request.subject {
        ReportSubject::People { ids } => ReportSubjectSelection::People {
            ids: validate_people(ids)?,
        },
        ReportSubject::Tenant {} => ReportSubjectSelection::Tenant { id: tenant_id },
    };
    let from = parse_date("period.from", &request.period.from)?;
    let to = parse_date("period.to", &request.period.to)?;
    if from > to {
        return invalid("period", "period.from must be before or equal to period.to");
    }

    Ok(RecipeShape {
        subject,
        from,
        to,
        granularity: request.granularity,
        metric_keys: validate_metric_keys(request.metric_keys)?,
    })
}

fn validate_people(ids: Vec<Uuid>) -> Result<Vec<Uuid>, CanonicalError> {
    if ids.is_empty() {
        return invalid("subject.ids", "subject.ids must not be empty");
    }
    if ids.len() > MAX_REPORT_PEOPLE {
        return invalid(
            "subject.ids",
            format!("subject.ids must contain at most {MAX_REPORT_PEOPLE} people"),
        );
    }

    let mut seen = BTreeSet::new();
    for id in &ids {
        if id.is_nil() {
            return invalid("subject.ids", "subject.ids must be person UUIDs");
        }
        if !seen.insert(*id) {
            return invalid("subject.ids", "subject.ids must not contain duplicates");
        }
    }
    Ok(ids)
}

fn validate_metric_keys(metric_keys: Vec<String>) -> Result<Vec<String>, CanonicalError> {
    if metric_keys.is_empty() {
        return invalid("metric_keys", "metric_keys must not be empty");
    }
    if metric_keys.len() > MAX_REPORT_METRICS {
        return invalid(
            "metric_keys",
            format!("metric_keys must contain at most {MAX_REPORT_METRICS} metrics"),
        );
    }

    let mut seen = BTreeSet::new();
    let mut normalized = Vec::with_capacity(metric_keys.len());
    for metric_key in metric_keys {
        let metric_key = normalize_metric_key("metric_keys", &metric_key)?;
        if !seen.insert(metric_key.clone()) {
            return invalid("metric_keys", format!("duplicate metric key: {metric_key}"));
        }
        normalized.push(metric_key);
    }
    Ok(normalized)
}

fn validate_loaded_definitions(
    shape: &RecipeShape,
    definitions: &HashMap<String, MetricDefinition>,
) -> Result<Vec<MetricDefinition>, CanonicalError> {
    let mut metrics = Vec::with_capacity(shape.metric_keys.len());
    for metric_key in &shape.metric_keys {
        let definition = definitions.get(metric_key).cloned().ok_or_else(|| {
            MetricError::invalid_argument()
                .with_field_violation(
                    "metric_keys",
                    format!("unknown or unavailable metric key: {metric_key}"),
                    "UNAVAILABLE",
                )
                .create()
        })?;
        if definition.base.entity_type != shape.subject.entity_type() {
            return invalid(
                "subject.type",
                format!(
                    "metric {metric_key} is defined for entity type {}",
                    definition.base.entity_type
                ),
            );
        }
        metrics.push(definition);
    }
    Ok(metrics)
}

fn parse_date(field: &'static str, value: &str) -> Result<NaiveDate, CanonicalError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        MetricError::invalid_argument()
            .with_field_violation(field, "expected YYYY-MM-DD", "INVALID")
            .create()
    })
}

fn invalid<T>(field: &'static str, message: impl Into<String>) -> Result<T, CanonicalError> {
    Err(MetricError::invalid_argument()
        .with_field_violation(field, message.into(), "INVALID")
        .create())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::definition::{
        MetricBase, MetricInputRole, ObservationRelation,
    };
    use crate::domain::metric_definitions::{
        AliasCollapse, ComputationSpec, MetricDirection, MetricFormat, MetricInput,
        ObservationSource,
    };
    use crate::domain::reports::dto::{ReportExportFormat, ReportPeriod, ReportSubject};

    const FIRST_PERSON: Uuid = Uuid::from_u128(0x019e_27bc_dec0_7626_81a9_c552_4662_a6a9);
    const SECOND_PERSON: Uuid = Uuid::from_u128(0x019e_27bc_dec0_7626_81a9_c552_4662_a6aa);

    fn recipe(subject: ReportSubject, metric_keys: &[&str]) -> ReportRecipe {
        ReportRecipe {
            subject,
            period: ReportPeriod {
                from: "2026-01-01".to_owned(),
                to: "2026-03-31".to_owned(),
            },
            granularity: ReportGranularity::Month,
            metric_keys: metric_keys.iter().map(|key| (*key).to_owned()).collect(),
        }
    }

    fn definition(metric_key: &str, entity_type: &str) -> MetricDefinition {
        MetricDefinition {
            base: MetricBase {
                key: metric_key.to_owned(),
                label: "Metric".to_owned(),
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

    #[test]
    fn preserves_people_and_metric_order_after_validation() {
        let shape = validate_recipe_shape(
            recipe(
                ReportSubject::People {
                    ids: vec![FIRST_PERSON, SECOND_PERSON],
                },
                &["git.commits", "tasks.closed"],
            ),
            Uuid::nil(),
        )
        .unwrap_or_else(|error| panic!("expected valid recipe: {error}"));

        let ReportSubjectSelection::People { ids } = shape.subject else {
            panic!("expected people subject");
        };
        assert_eq!(ids, [FIRST_PERSON, SECOND_PERSON]);
        assert_eq!(shape.metric_keys, ["git.commits", "tasks.closed"]);
    }

    #[test]
    fn rejects_empty_or_duplicate_people() {
        for ids in [vec![], vec![FIRST_PERSON, FIRST_PERSON]] {
            assert!(
                validate_recipe_shape(
                    recipe(ReportSubject::People { ids }, &["git.commits"]),
                    Uuid::nil(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn rejects_more_people_than_identity_can_hydrate() {
        let ids = (0..=MAX_REPORT_PEOPLE)
            .map(|offset| Uuid::from_u128(offset as u128 + 1))
            .collect();

        assert!(
            validate_recipe_shape(
                recipe(ReportSubject::People { ids }, &["git.commits"]),
                Uuid::nil(),
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_empty_or_duplicate_metric_keys() {
        for metric_keys in [
            vec![],
            vec![""],
            vec!["git.commits", "git.commits"],
            vec!["Git.Commits", "git.commits"],
        ] {
            let keys = metric_keys.clone();
            assert!(
                validate_recipe_shape(recipe(ReportSubject::Tenant {}, &keys), Uuid::nil(),)
                    .is_err()
            );
        }
    }

    #[test]
    fn rejects_more_than_one_hundred_metric_keys() {
        let mut accepted = recipe(ReportSubject::Tenant {}, &["git.commits"]);
        accepted.metric_keys = (0..MAX_REPORT_METRICS)
            .map(|index| format!("metric.key{index}"))
            .collect();
        assert!(validate_recipe_shape(accepted, Uuid::nil()).is_ok());

        let mut rejected = recipe(ReportSubject::Tenant {}, &["git.commits"]);
        rejected.metric_keys = (0..=MAX_REPORT_METRICS)
            .map(|index| format!("metric.key{index}"))
            .collect();
        assert!(validate_recipe_shape(rejected, Uuid::nil()).is_err());
    }

    #[test]
    fn accepts_pre_excel_periods_except_for_xlsx_exports() {
        let mut recipe = recipe(ReportSubject::Tenant {}, &["git.commits"]);
        recipe.period.from = "1899-12-31".to_owned();
        assert!(validate_recipe_shape(recipe.clone(), Uuid::nil()).is_ok());

        for format in [ReportExportFormat::Csv, ReportExportFormat::Xlsx] {
            let request = ReportExportRequest {
                subject: recipe.subject.clone(),
                period: recipe.period.clone(),
                granularity: recipe.granularity,
                metric_keys: recipe.metric_keys.clone(),
                format,
            };
            assert_eq!(
                validate_export_shape(request, Uuid::nil()).is_ok(),
                matches!(format, ReportExportFormat::Csv),
                "should accept {format:?}"
            );
        }
    }

    #[test]
    fn rejects_malformed_or_reversed_dates() {
        let mut malformed = recipe(ReportSubject::Tenant {}, &["git.commits"]);
        malformed.period.from = "not-a-date".to_owned();
        assert!(validate_recipe_shape(malformed, Uuid::nil()).is_err());

        let mut reversed = recipe(ReportSubject::Tenant {}, &["git.commits"]);
        reversed.period.from = "2026-04-01".to_owned();
        assert!(validate_recipe_shape(reversed, Uuid::nil()).is_err());
    }

    #[test]
    fn rejects_unknown_subject_type_during_deserialization() {
        let request = r#"{"subject":{"type":"group"},"period":{"from":"2026-01-01","to":"2026-01-31"},"granularity":"day","metric_keys":["git.commits"]}"#;
        assert!(serde_json::from_str::<ReportRecipe>(request).is_err());
    }

    #[test]
    fn rejects_unknown_or_subject_incompatible_metrics() {
        let shape = validate_recipe_shape(
            recipe(
                ReportSubject::People {
                    ids: vec![FIRST_PERSON],
                },
                &["git.commits"],
            ),
            Uuid::nil(),
        )
        .unwrap_or_else(|error| panic!("expected valid recipe: {error}"));
        assert!(validate_loaded_definitions(&shape, &HashMap::new()).is_err());

        let definitions = HashMap::from([(
            "git.commits".to_owned(),
            definition("git.commits", "tenant"),
        )]);
        assert!(validate_loaded_definitions(&shape, &definitions).is_err());
    }
}
