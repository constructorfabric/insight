//! The field catalog: every dataset's typed, role-annotated fields. It is the
//! validation universe a definition is checked against, the compiler's
//! resolution table, and an editor's palette — a field absent here does not
//! exist at any layer above.
//!
//! Two inputs, joined at load: `columns.snapshot.json`, dumped from a
//! ClickHouse the real pipeline built and gated against drift by the
//! connectors-ddl lane, and `roles.yaml`, authored here. The snapshot is a seam,
//! not an architecture — what the catalog consumes is the artifact's shape, so
//! another producer (typed ingestion-runtime schemas, captured projection
//! schemas) replaces it without touching this module.

#![allow(dead_code)] // tests are this module's only callers in the crate

pub mod loader;
pub mod model;

use std::sync::OnceLock;

use loader::CatalogError;
use model::FieldCatalog;

const COLUMN_SNAPSHOT: &str = include_str!("columns.snapshot.json");
const ROLES: &str = include_str!("roles.yaml");

/// The product catalog, parsed once. Both inputs are compiled in, so a failure
/// here is a build-time authoring error surfaced at first use — never a
/// warehouse-availability question.
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
