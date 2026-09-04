//! The datasets this build serves.
//!
//! INVARIANT: declarations are compiled in and validated once, so one that has
//! outgrown the warehouse fails a test run rather than a request.

pub mod declaration;
pub mod validate;

use std::sync::OnceLock;

use crate::domain::field_catalog::product_catalog;

use declaration::{Dataset, DatasetDocument};
use validate::DeclarationError;

const DECLARATIONS: &[&str] = &[
    include_str!("git_commits.yaml"),
    include_str!("git_file_changes.yaml"),
];

pub fn product_datasets() -> Result<&'static [Dataset], &'static DeclarationError> {
    static DATASETS: OnceLock<Result<Vec<Dataset>, DeclarationError>> = OnceLock::new();
    DATASETS
        .get_or_init(|| load(DECLARATIONS))
        .as_ref()
        .map(Vec::as_slice)
}

pub fn dataset(key: &str) -> Option<&'static Dataset> {
    product_datasets()
        .ok()?
        .iter()
        .find(|dataset| dataset.key.as_str() == key)
}

/// Every key a query may name, in declaration order.
pub fn declared_keys() -> Vec<&'static str> {
    product_datasets()
        .map(|datasets| {
            datasets
                .iter()
                .map(|dataset| dataset.key.as_str())
                .collect()
        })
        .unwrap_or_default()
}

fn load(documents: &[&str]) -> Result<Vec<Dataset>, DeclarationError> {
    let catalog =
        product_catalog().map_err(|error| DeclarationError::Document(error.to_string()))?;

    let mut datasets: Vec<Dataset> = Vec::with_capacity(documents.len());
    for raw in documents {
        let document: DatasetDocument = serde_yaml::from_str(raw)
            .map_err(|error| DeclarationError::Document(error.to_string()))?;
        let dataset = validate::validate(document, catalog)?;

        if datasets.iter().any(|seen| seen.key == dataset.key) {
            return Err(DeclarationError::DuplicateDataset(dataset.key.to_string()));
        }
        datasets.push(dataset);
    }

    Ok(datasets)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_declaration_agrees_with_the_field_catalog() {
        let datasets = product_datasets().expect("the shipped declarations are admissible");
        assert!(!datasets.is_empty());
    }

    #[test]
    fn the_commits_dataset_declares_what_a_query_over_it_needs() {
        let commits = dataset("git_commits").expect("git_commits is declared");

        assert_eq!(commits.database, "insight");
        assert_eq!(commits.relation, "git_commits");
        assert_eq!(commits.tenant_field, "tenant_id");
        assert_eq!(
            commits
                .default_time_field()
                .map(|field| field.field.as_str()),
            Some("authored_at")
        );
        assert!(commits.dimension("author_email").is_some());
        assert!(commits.measurable("lines_added").is_some());
        assert_eq!(commits.row_identity, ["tenant_id", "source", "commit_hash"]);
    }

    #[test]
    fn the_file_change_dataset_declares_the_grain_below_a_commit() {
        let changes = dataset("git_file_changes").expect("git_file_changes is declared");

        assert_eq!(changes.database, "insight");
        assert_eq!(changes.relation, "git_file_changes");
        assert_eq!(changes.tenant_field, "tenant_id");
        assert_eq!(
            changes
                .default_time_field()
                .map(|field| field.field.as_str()),
            Some("authored_at")
        );
        for field in ["file_extension", "change_type", "category"] {
            assert!(
                changes.dimension(field).is_some(),
                "{field} must be groupable"
            );
        }
        assert_eq!(
            changes.row_identity,
            [
                "tenant_id",
                "source",
                "commit_hash",
                "file_path",
                "change_type"
            ]
        );
    }

    #[test]
    fn a_column_one_dataset_groups_by_is_not_thereby_groupable_on_another() {
        let commits = dataset("git_commits").expect("git_commits is declared");
        let changes = dataset("git_file_changes").expect("git_file_changes is declared");

        assert!(changes.dimension("file_extension").is_some());
        assert!(
            commits.dimension("file_extension").is_none(),
            "a commit has no one file extension"
        );
        assert!(
            changes.dimension("file_path").is_none(),
            "a path is row identity, not a group axis"
        );
    }

    #[test]
    fn a_key_no_declaration_carries_resolves_to_nothing() {
        assert!(dataset("git_tags").is_none());
        assert_eq!(declared_keys(), ["git_commits", "git_file_changes"]);
    }

    #[test]
    fn a_repeated_declaration_key_is_refused() {
        let one = include_str!("git_commits.yaml");
        assert_eq!(
            load(&[one, one]),
            Err(DeclarationError::DuplicateDataset("git_commits".to_owned()))
        );
    }
}
