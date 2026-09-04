//! Every warehouse relation the service may read, with the type of every column.
//!
//! INVARIANT: `columns.snapshot.json` is compiled in, so a declaration naming a
//! column the warehouse does not carry fails a test run rather than a request.

pub mod loader;
pub mod model;

use std::sync::OnceLock;

use loader::CatalogError;
use model::FieldCatalog;

const COLUMN_SNAPSHOT: &str = include_str!("columns.snapshot.json");

pub fn product_catalog() -> Result<&'static FieldCatalog, &'static CatalogError> {
    static CATALOG: OnceLock<Result<FieldCatalog, CatalogError>> = OnceLock::new();
    CATALOG
        .get_or_init(|| loader::load(COLUMN_SNAPSHOT))
        .as_ref()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_snapshot_loads() {
        let catalog = product_catalog().expect("the shipped snapshot parses");
        assert!(!catalog.relations.is_empty());
    }

    #[test]
    fn every_catalogued_relation_names_at_least_one_column() {
        let catalog = product_catalog().expect("the shipped snapshot parses");
        for relation in &catalog.relations {
            assert!(
                !relation.columns.is_empty(),
                "{}.{} carries no columns",
                relation.database,
                relation.relation
            );
        }
    }
}
