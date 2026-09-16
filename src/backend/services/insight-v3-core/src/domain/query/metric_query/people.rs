//! Reading a person out of the identity store, by whichever handle a metric
//! holds them under.

use serde::Deserialize;

use super::MetricQueryError;
use super::field::is_identifier;

const NAME_CTE: &str = "__person_name";
const EMAIL_CTE: &str = "__people_by_email";
const ID_CTE: &str = "__people_by_id";
const IDENTITY_TABLE: &str = "identity_persons";

/// Which handle a person column carries.
///
/// A fact table names a person the way its source system did - the address
/// they committed under, the id an API returned. Neither is the name anyone
/// would recognise them by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum PersonHandle {
    Email,
    Id,
}

impl PersonHandle {
    pub(super) fn cte(self) -> &'static str {
        match self {
            Self::Email => EMAIL_CTE,
            Self::Id => ID_CTE,
        }
    }

    /// The handle as the identity side spells it: ids are compared as text,
    /// because the fact table stores one as a UUID, a String, or neither.
    pub(super) fn key(self, read: &str) -> String {
        match self {
            Self::Email => read.to_owned(),
            Self::Id => format!("toString({read})"),
        }
    }
}

/// Where a person's name is resolved from.
///
/// The identity database on this stand. Only the database is configurable:
/// the table and the `value_type` rows inside it are identity's own schema,
/// not something a stand chooses.
#[derive(Debug, Clone)]
pub(crate) struct People {
    database: String,
}

impl People {
    pub(crate) fn new(database: impl Into<String>) -> Self {
        Self {
            database: database.into(),
        }
    }

    /// The `WITH` clauses the joins read from.
    ///
    /// INVARIANT: a person accumulates a row per name they have ever had, so
    /// the latest one wins by `argMax` before anything joins to it. Joining
    /// the rows directly multiplies every fact by that history.
    pub(super) fn prelude(&self, handles: &[PersonHandle]) -> Result<String, MetricQueryError> {
        if !is_identifier(&self.database) {
            return Err(MetricQueryError::Identifier(self.database.clone()));
        }
        let persons = format!("`{}`.`{IDENTITY_TABLE}`", self.database);

        let mut ctes = vec![format!(
            "{NAME_CTE} AS (SELECT `person_id`, argMax(`value_effective`, `created_at`) AS `display_name` FROM {persons} WHERE `value_type` = 'display_name' GROUP BY `person_id`)"
        )];
        for handle in handles {
            ctes.push(match handle {
                PersonHandle::Email => format!(
                    "{EMAIL_CTE} AS (SELECT `h`.`handle` AS `handle`, `n`.`display_name` AS `display_name` FROM (SELECT `value_effective` AS `handle`, argMax(`person_id`, `created_at`) AS `person_id` FROM {persons} WHERE `value_type` = 'email' GROUP BY `value_effective`) AS `h` INNER JOIN {NAME_CTE} AS `n` ON `n`.`person_id` = `h`.`person_id`)"
                ),
                PersonHandle::Id => format!(
                    "{ID_CTE} AS (SELECT toString(`person_id`) AS `handle`, `display_name` FROM {NAME_CTE})"
                ),
            });
        }

        Ok(format!("WITH {} ", ctes.join(", ")))
    }
}
