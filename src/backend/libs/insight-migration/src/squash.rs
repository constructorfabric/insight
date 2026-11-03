//! Squashed migration history.
//!
//! A service squashes a prefix of its history into one baseline migration
//! without losing the names of what it squashed. The squashed names stay in
//! the migrator as [`Squashed`] entries that apply nothing, ordered after the
//! baseline, so:
//!
//! - a fresh database runs the baseline, which builds the whole squashed
//!   schema, and then records every squashed name — its ledger reads the same
//!   as one that walked the old history;
//! - a database that walked the whole old history already holds every
//!   squashed name; only the baseline is pending, and it applies nothing;
//! - a database that walked only part of it is refused by the baseline, before
//!   anything is written, because nothing left in the build can bring it
//!   forward — it must first upgrade through a release that still carries the
//!   old history;
//! - an older image rolled back onto a squashed database finds every name it
//!   embeds in the ledger, so the boot gate ([`crate::assert_compatible`])
//!   still lets it serve. Only the server rolls back: an older image's
//!   `migrate` refuses a ledger holding the baseline, as it refuses any
//!   migration newer than itself.
//!
//! The names must stay exactly as they were: sea-orm refuses to run against a
//! ledger naming a migration the build does not embed, and
//! [`crate::assert_compatible`] refuses to serve a build whose names are not
//! all recorded.

use sea_orm::{ColumnTrait, DbErr, EntityTrait, QueryFilter};
use sea_orm_migration::{
    MigrationName, MigrationTrait, MigratorTrait, SchemaManager, seaql_migrations,
};

/// A squashed migration: the name it was applied under, applying nothing.
///
/// Its schema is built by the baseline that squashed it; the entry exists only
/// so the ledger keeps the name.
pub struct Squashed(pub &'static str);

impl MigrationName for Squashed {
    fn name(&self) -> &str {
        self.0
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Squashed {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

/// The [`Squashed`] entries for `names`, in order, ready to follow the
/// baseline in a migrator's list.
#[must_use]
pub fn squashed(names: &'static [&'static str]) -> Vec<Box<dyn MigrationTrait>> {
    names
        .iter()
        .map(|name| Box::new(Squashed(name)) as Box<dyn MigrationTrait>)
        .collect()
}

/// What a baseline finds in the ledger about the history it squashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum History {
    /// None of the squashed names is recorded: the baseline builds the schema.
    Fresh,
    /// Every squashed name is recorded: the schema stands, and the baseline
    /// applies nothing.
    Complete,
}

/// Read which squashed names the ledger holds, and refuse a ledger holding
/// only some of them.
///
/// Called from a baseline's `up`. sea-orm has created the ledger table by
/// then, so an absent table is not a case here.
///
/// `upgrade_through` names the last release that still carries the unsquashed
/// history; it goes into the refusal so an operator knows where to step
/// through.
///
/// # Errors
///
/// Returns an error if `M` keeps its ledger under a custom table name, if the
/// ledger cannot be read, or if it records some but not all of `names`.
pub async fn history<M>(
    manager: &SchemaManager<'_>,
    names: &[&str],
    upgrade_through: &str,
) -> Result<History, DbErr>
where
    M: MigratorTrait,
{
    // INVARIANT: the entity below is hardwired to the default table name. A
    // migrator keeping its ledger elsewhere would read an empty history here
    // and rebuild over a schema that stands, so it is refused instead.
    let table = M::migration_table_name().to_string();
    if table != "seaql_migrations" {
        return Err(DbErr::Custom(
            crate::MigrationError::UnsupportedTableName(table).to_string(),
        ));
    }

    let recorded: Vec<String> = seaql_migrations::Entity::find()
        .filter(seaql_migrations::Column::Version.is_in(names.iter().copied()))
        .all(manager.get_connection())
        .await?
        .into_iter()
        .map(|model| model.version)
        .collect();

    classify(names, &recorded, upgrade_through)
}

/// The decision [`history`] makes, apart from the read.
fn classify(names: &[&str], recorded: &[String], upgrade_through: &str) -> Result<History, DbErr> {
    if recorded.is_empty() {
        return Ok(History::Fresh);
    }
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| !recorded.iter().any(|held| held == name))
        .collect();
    if missing.is_empty() {
        return Ok(History::Complete);
    }

    let first = missing.first().copied().unwrap_or_default();
    Err(DbErr::Custom(format!(
        "this database walked part of the migration history this build squashed: {} of {} \
         squashed migrations are recorded, the first one missing is `{first}`. This build \
         cannot bring it forward. First upgrade through {upgrade_through}, then deploy this \
         release.",
        names.len() - missing.len(),
        names.len(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[&str] = &["m1_a", "m2_b", "m3_c"];

    fn held(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn an_empty_ledger_is_fresh() {
        assert_eq!(classify(NAMES, &[], "r").ok(), Some(History::Fresh));
    }

    #[test]
    fn a_ledger_holding_every_name_is_complete() {
        assert_eq!(
            classify(NAMES, &held(NAMES), "r").ok(),
            Some(History::Complete)
        );
    }

    #[test]
    fn a_ledger_holding_some_names_is_refused_and_names_the_release() {
        let Err(error) = classify(NAMES, &held(&["m1_a", "m3_c"]), "release 0.0.1") else {
            panic!("a partial history must be refused");
        };
        let message = error.to_string();
        assert!(message.contains("2 of 3"), "{message}");
        assert!(message.contains("`m2_b`"), "{message}");
        assert!(message.contains("release 0.0.1"), "{message}");
    }

    #[test]
    fn a_squashed_entry_answers_to_its_name() {
        let entries = squashed(NAMES);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name()).collect();
        assert_eq!(names, NAMES);
    }
}
