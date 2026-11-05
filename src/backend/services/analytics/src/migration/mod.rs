//! Database migrations for the Analytics API service.

pub(crate) mod ai_assist_schema;
pub(crate) mod feedback_schema;
mod m20261014_000001_drop_unread_catalog_tables;
mod m20261030_000001_baseline;
pub(crate) mod m20261103_000001_connector_schedules;

use insight_migration::squash;
use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        // INVARIANT: the baseline runs before the squashed names are recorded;
        // recorded first, they would read as a complete history and the
        // baseline would skip building the schema.
        let mut migrations: Vec<Box<dyn MigrationTrait>> =
            vec![Box::new(m20261030_000001_baseline::Migration)];
        migrations.extend(squash::squashed(m20261030_000001_baseline::SQUASHED));
        migrations.push(Box::new(
            m20261014_000001_drop_unread_catalog_tables::Migration,
        ));
        migrations.push(Box::new(m20261103_000001_connector_schedules::Migration));
        migrations
    }
}

/// Per-table CHECK constraint names that the startup probe asserts present.
///
/// Source of truth for the probe in `infra/db/check_probe.rs`. Each entry maps
/// a table name to the CHECK names the baseline creates.
pub const REQUIRED_CHECKS_BY_TABLE: &[(&str, &[&str])] = &[
    (
        "connector_schedules",
        m20261103_000001_connector_schedules::REQUIRED_CHECKS,
    ),
    (
        "metric_sources",
        m20261030_000001_baseline::REQUIRED_SOURCE_CHECKS,
    ),
    (
        "metric_source_measures",
        m20261030_000001_baseline::REQUIRED_SOURCE_MEASURE_CHECKS,
    ),
    (
        "metric_source_dimensions",
        m20261030_000001_baseline::REQUIRED_SOURCE_DIMENSION_CHECKS,
    ),
    (
        "metric_definitions",
        m20261030_000001_baseline::REQUIRED_DEFINITION_CHECKS,
    ),
    (
        "metric_definition_dimensions",
        m20261030_000001_baseline::REQUIRED_DIMENSION_CHECKS,
    ),
    (
        "metric_definition_tags",
        m20261030_000001_baseline::REQUIRED_TAG_CHECKS,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baseline_runs_first_and_the_squashed_names_follow_in_order() {
        let migrations = Migrator::migrations();
        let names: Vec<&str> = migrations.iter().map(|m| m.name()).collect();
        let squashed = m20261030_000001_baseline::SQUASHED;

        assert_eq!(names.first(), Some(&"m20261030_000001_baseline"));
        assert_eq!(&names[1..=squashed.len()], squashed);
        assert_eq!(
            names.get(squashed.len() + 1),
            Some(&"m20261014_000001_drop_unread_catalog_tables")
        );
        assert_eq!(names.last(), Some(&"m20261103_000001_connector_schedules"));
    }

    /// A table whose migration declares CHECKs must be registered in
    /// [`REQUIRED_CHECKS_BY_TABLE`], or the probe never reports them missing.
    #[test]
    fn required_checks_by_table_covers_every_catalog_table() {
        // Wire each migration's CHECK list by its own const
        // pointer so renaming the const on one side fails compilation here
        // first — there is no string-matching slop in this comparison.
        let expected: &[(&str, &[&str])] = &[
            (
                "metric_sources",
                m20261030_000001_baseline::REQUIRED_SOURCE_CHECKS,
            ),
            (
                "metric_source_measures",
                m20261030_000001_baseline::REQUIRED_SOURCE_MEASURE_CHECKS,
            ),
            (
                "metric_source_dimensions",
                m20261030_000001_baseline::REQUIRED_SOURCE_DIMENSION_CHECKS,
            ),
            (
                "metric_definitions",
                m20261030_000001_baseline::REQUIRED_DEFINITION_CHECKS,
            ),
            (
                "metric_definition_dimensions",
                m20261030_000001_baseline::REQUIRED_DIMENSION_CHECKS,
            ),
            (
                "metric_definition_tags",
                m20261030_000001_baseline::REQUIRED_TAG_CHECKS,
            ),
        ];

        for &(table, checks) in expected {
            let Some(&(_, registered)) =
                REQUIRED_CHECKS_BY_TABLE.iter().find(|&&(t, _)| t == table)
            else {
                panic!(
                    "catalog table `{table}` is not registered in \
                     REQUIRED_CHECKS_BY_TABLE — the startup probe will \
                     silently skip its CHECKs"
                );
            };
            assert_eq!(
                registered.as_ptr(),
                checks.as_ptr(),
                "REQUIRED_CHECKS_BY_TABLE entry for `{table}` points at a \
                 different REQUIRED_CHECKS slice than the migration module \
                 exports — keep them in sync"
            );
        }
    }
}
