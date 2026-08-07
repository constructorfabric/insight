use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::apply_sql(manager, include_str!("sql/015_person_attributes.sql")).await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(super::irreversible())
    }
}

#[cfg(test)]
mod tests {
    const SCRIPT: &str = include_str!("sql/015_person_attributes.sql");

    #[test]
    fn ddl_declares_every_constraint_and_enum_variant() {
        for required in [
            "uq_definition",
            "uq_definition_revision",
            "chk_person_attribute_policy_revision_positive CHECK (revision >= 1)",
            "ENUM('single','multi')",
            "COLLATE utf8mb4_bin",
        ] {
            assert!(SCRIPT.contains(required), "missing from DDL: {required}");
        }
    }
}
