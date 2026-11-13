pub(crate) mod m20261110_000001_connector_schedules;

use sea_orm_migration::prelude::*;

#[derive(Debug)]
pub(crate) struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20261110_000001_connector_schedules::Migration)]
    }
}

pub(crate) const REQUIRED_CHECKS_BY_TABLE: &[(&str, &[&str])] = &[(
    "connector_schedules",
    m20261110_000001_connector_schedules::REQUIRED_CHECKS,
)];
