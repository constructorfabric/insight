//! Database migrations for the Identity Resolution service.
//!
//! The service owns its schema in the dedicated `MariaDB` database `identity`;
//! migrations are applied at startup via `Migrator::up(db, None)`. See
//! ADR-0006 for the service-owned-migrations decision.

mod m20260421_000001_persons;

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20260421_000001_persons::Migration)]
    }
}
