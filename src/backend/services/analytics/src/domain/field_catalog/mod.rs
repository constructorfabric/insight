//! The field catalog: every dataset's typed, role-annotated fields — the
//! validation universe a definition is checked against and the compiler's
//! resolution table. Two inputs joined at load: `columns.snapshot.json`, dumped
//! from ClickHouse, and the authored `roles.yaml`.

#![allow(dead_code)] // tests are this module's only callers in the crate

pub mod loader;
pub mod model;

use std::sync::OnceLock;

use loader::CatalogError;
use model::FieldCatalog;

const COLUMN_SNAPSHOT: &str = include_str!("columns.snapshot.json");
const ROLES: &str = include_str!("roles.yaml");

/// Both inputs are compiled in, so a failure here is an authoring error, never
/// a warehouse-availability question.
pub fn product_catalog() -> Result<&'static FieldCatalog, &'static CatalogError> {
    static CATALOG: OnceLock<Result<FieldCatalog, CatalogError>> = OnceLock::new();
    CATALOG
        .get_or_init(|| loader::load(COLUMN_SNAPSHOT, ROLES))
        .as_ref()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use model::FieldRole;

    #[test]
    fn the_shipped_catalog_loads() {
        let catalog = product_catalog().expect("authored roles agree with the snapshot");
        assert!(!catalog.datasets.is_empty());
    }

    #[test]
    fn every_dataset_binds_the_roles_the_compiler_injects_on() {
        let catalog = product_catalog().expect("catalog loads");
        for dataset in &catalog.datasets {
            assert_eq!(
                dataset.fields_with_role(FieldRole::Tenant).count(),
                1,
                "{} must bind exactly one tenant field",
                dataset.key
            );
            assert!(
                dataset
                    .fields_with_role(FieldRole::EventTime)
                    .next()
                    .is_some(),
                "{} must bind an event time",
                dataset.key
            );
        }
    }
}
