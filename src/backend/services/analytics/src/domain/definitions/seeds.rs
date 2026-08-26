//! Product definitions: authored as repo YAML, embedded at compile time,
//! validated by the build, and reconciled into the store at startup. The repo
//! is how they are authored and reviewed; the store is the single runtime
//! source of truth for every reader.
//!
//! Datasets are not authored here. The field catalog already declares each
//! dataset's relation and the read discipline its engine implies, so dataset
//! rows are a projection of the catalog — one place to change a relation, not
//! two to keep in agreement.

use sea_orm::{DatabaseConnection, TransactionTrait};
use serde::Deserialize;

use super::definition::{
    DatasetDefinition, MeasureDefinition, MetricDefinition, Origin, ReadDiscipline,
};
use super::store::{StoreError, reconcile_dataset, reconcile_measure, reconcile_metric};
use super::validate::{ValidationError, validate_definitions};
use crate::domain::field_catalog::{
    self,
    model::{FieldCatalog, ReadDiscipline as CatalogReadDiscipline},
};

/// Who the audit trail records for a definition that came from the repo.
const ACTOR: &str = "product-seed";

const FAMILIES: &[(&str, &str)] = &[("git", include_str!("seeds/git.yaml"))];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyFile {
    #[serde(default)]
    measures: Vec<MeasureDefinition>,
    #[serde(default)]
    metrics: Vec<MetricDefinition>,
}

#[derive(Debug, Default)]
pub struct ProductDefinitions {
    pub datasets: Vec<DatasetDefinition>,
    pub measures: Vec<MeasureDefinition>,
    pub metrics: Vec<MetricDefinition>,
}

#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("catalog: {0}")]
    Catalog(String),
    #[error("`{family}` does not parse: {source}")]
    Parse {
        family: &'static str,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("product definitions are invalid:{}", format_errors(.0))]
    Invalid(Vec<ValidationError>),
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

fn format_errors(errors: &[ValidationError]) -> String {
    use std::fmt::Write;

    errors.iter().fold(String::new(), |mut message, error| {
        let _ = write!(message, "\n  - {error}");
        message
    })
}

/// Parse and validate every shipped definition against the field catalog.
pub fn product_definitions() -> Result<ProductDefinitions, SeedError> {
    let catalog =
        field_catalog::product_catalog().map_err(|error| SeedError::Catalog(error.to_string()))?;
    let mut definitions = ProductDefinitions {
        datasets: datasets_from_catalog(catalog),
        ..ProductDefinitions::default()
    };

    for (family, body) in FAMILIES {
        let parsed: FamilyFile =
            serde_yaml::from_str(body).map_err(|source| SeedError::Parse { family, source })?;
        definitions.measures.extend(parsed.measures);
        definitions.metrics.extend(parsed.metrics);
    }

    validate_definitions(catalog, &definitions.measures, &definitions.metrics)
        .map_err(SeedError::Invalid)?;
    Ok(definitions)
}

fn datasets_from_catalog(catalog: &FieldCatalog) -> Vec<DatasetDefinition> {
    catalog
        .datasets
        .iter()
        .map(|dataset| DatasetDefinition {
            key: dataset.key.clone(),
            relation: format!("{}.{}", dataset.database, dataset.relation),
            read_discipline: match dataset.read_discipline {
                CatalogReadDiscipline::Collapsing => ReadDiscipline::Final,
                CatalogReadDiscipline::Direct => ReadDiscipline::None,
            },
            description: None,
            retention_horizon: None,
        })
        .collect()
}

/// Converge the store on the shipped definitions. Datasets land before the
/// measures that read them and metrics after the measures they compose, so a
/// reader never sees a definition whose references are not yet written.
pub async fn reconcile_product_definitions(db: &DatabaseConnection) -> Result<(), SeedError> {
    let definitions = product_definitions()?;

    // One family's convergence is one transaction: a mid-way failure leaves the
    // store on the previous definitions rather than on half of the new ones.
    let txn = db.begin().await.map_err(StoreError::from)?;
    for dataset in &definitions.datasets {
        reconcile_dataset(&txn, dataset, Origin::Product, ACTOR).await?;
    }
    for measure in &definitions.measures {
        reconcile_measure(&txn, measure, Origin::Product, ACTOR).await?;
    }
    for metric in &definitions.metrics {
        reconcile_metric(&txn, metric, Origin::Product, ACTOR).await?;
    }
    txn.commit().await.map_err(StoreError::from)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_definition_is_valid() {
        let definitions = match product_definitions() {
            Ok(definitions) => definitions,
            Err(error) => panic!("{error}"),
        };
        assert!(!definitions.measures.is_empty());
        assert!(!definitions.metrics.is_empty());
    }

    #[test]
    fn every_dataset_a_measure_reads_is_seeded() {
        let definitions = product_definitions().expect("definitions are valid");
        for measure in &definitions.measures {
            assert!(
                definitions
                    .datasets
                    .iter()
                    .any(|dataset| dataset.key == measure.dataset),
                "measure `{}` reads unseeded dataset `{}`",
                measure.key,
                measure.dataset
            );
        }
    }

    #[test]
    fn a_dataset_carries_the_read_discipline_its_engine_demands() {
        let definitions = product_definitions().expect("definitions are valid");
        let replacing = definitions
            .datasets
            .iter()
            .find(|dataset| dataset.key == "git_pull_requests")
            .expect("git_pull_requests is catalogued");
        assert_eq!(replacing.read_discipline, ReadDiscipline::Final);
        assert_eq!(replacing.relation, "silver.class_git_pull_requests");
    }

    #[test]
    fn a_family_file_rejects_an_unknown_section() {
        assert!(serde_yaml::from_str::<FamilyFile>("datasets: []\n").is_err());
    }

    #[test]
    fn validation_failures_name_every_offender() {
        let errors = vec![
            ValidationError::KeyShape("Bad Key".to_owned()),
            ValidationError::DuplicateKey("prs_created".to_owned()),
        ];
        let message = SeedError::Invalid(errors).to_string();
        assert!(message.contains("Bad Key"), "{message}");
        assert!(message.contains("prs_created"), "{message}");
    }
}
