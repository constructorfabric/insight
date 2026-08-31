//! The per-measure advisory lock. Every replica runs the refresh loop and they
//! share one staging relation, so two of them building the same measure would
//! swap in each other's rows on top of their own.
//!
//! INVARIANT: `GET_LOCK` is session-scoped, so this owns a pool pinned to one
//! connection — a shared pool would scatter acquire and release across sessions.

use std::time::Duration;

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement,
    Value,
};

/// sqlx's own idle and lifetime defaults would reap the pinned connection
/// between an acquire and its release, handing the release a session that never
/// held the lock.
const SESSION_LIFETIME: Duration = Duration::from_hours(24);

/// INVARIANT: MariaDB caps a lock name at 192 characters, which this prefix
/// plus a `VARCHAR(128)` measure key cannot reach.
const NAME_PREFIX: &str = "semantic-cache-";

/// What one acquisition attempt answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockOutcome {
    /// This session owns the measure until it releases.
    Held,
    /// Another session is building this measure right now.
    HeldElsewhere,
    /// The lock could not be asked for, which is not the same as being refused.
    Unknown,
}

pub struct LockSession {
    db: DatabaseConnection,
}

impl LockSession {
    /// # Errors
    ///
    /// Returns an error if the pinned session cannot be opened.
    pub async fn connect(database_url: &str) -> Result<Self, DbErr> {
        let mut opts = ConnectOptions::new(database_url.to_owned());
        opts.max_connections(1)
            .min_connections(1)
            .idle_timeout(SESSION_LIFETIME)
            .max_lifetime(SESSION_LIFETIME)
            .test_before_acquire(false)
            .sqlx_logging(false);

        Ok(Self {
            db: Database::connect(opts).await?,
        })
    }

    #[cfg(test)]
    pub(super) fn disconnected() -> Self {
        Self {
            db: DatabaseConnection::default(),
        }
    }

    /// Never waits: a measure another replica already owns is that replica's
    /// tick to spend, not this one's to queue behind.
    pub async fn acquire(&self, measure_key: &str) -> LockOutcome {
        // CAST so the column type is deterministic — the lock builtins are typed
        // per expression and an uncast result decodes differently per server.
        let answered = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT CAST(GET_LOCK(?, 0) AS SIGNED)",
                [Value::from(lock_name(measure_key))],
            ))
            .await;

        match answered {
            Ok(Some(row)) => outcome(row.try_get_by_index::<Option<i64>>(0).ok().flatten()),
            Ok(None) => LockOutcome::Unknown,
            Err(error) => {
                tracing::warn!(%error, measure = %measure_key, "the measure cache lock is unavailable");
                LockOutcome::Unknown
            }
        }
    }

    /// A release that does not land is not a lost build: the lock dies with the
    /// session, and the next tick asks again.
    pub async fn release(&self, measure_key: &str) {
        if let Err(error) = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT RELEASE_LOCK(?)",
                [Value::from(lock_name(measure_key))],
            ))
            .await
        {
            tracing::warn!(%error, measure = %measure_key, "the measure cache lock was not released");
        }
    }
}

fn lock_name(measure_key: &str) -> String {
    format!("{NAME_PREFIX}{measure_key}")
}

/// `GET_LOCK` answers 1 when this session took the lock and 0 when the wait
/// expired because another session holds it; anything else says nothing.
fn outcome(answer: Option<i64>) -> LockOutcome {
    match answer {
        Some(1) => LockOutcome::Held,
        Some(0) => LockOutcome::HeldElsewhere,
        Some(_) | None => LockOutcome::Unknown,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn one_lock_stands_for_one_measure() {
        assert_eq!(lock_name("commits"), "semantic-cache-commits");
        assert_ne!(lock_name("commits"), lock_name("default_commits"));
    }

    #[test]
    fn the_longest_measure_key_the_store_admits_still_names_a_lock_mariadb_accepts() {
        let longest = "m".repeat(128);

        assert!(lock_name(&longest).len() <= 192);
    }

    #[test]
    fn a_refused_lock_is_told_apart_from_a_lock_that_could_not_be_asked_for() {
        assert_eq!(outcome(Some(1)), LockOutcome::Held);
        assert_eq!(outcome(Some(0)), LockOutcome::HeldElsewhere);
        assert_eq!(outcome(None), LockOutcome::Unknown);
    }

    #[tokio::test]
    async fn an_unreachable_session_owns_nothing_rather_than_assuming_it_does() {
        let session = LockSession::disconnected();

        assert_eq!(session.acquire("commits").await, LockOutcome::Unknown);
        session.release("commits").await;
    }
}
