//! Against a real `MariaDB`, because the claims here are about rows: one
//! writer at a time, a check landing only on its revision, a notification
//! owed once. Run with `INTEGRATION_TESTS_MARIADB_URL` set, after `migrate`.

use std::error::Error;

use chrono::Utc;
use uuid::Uuid;

use super::*;
use crate::domain::alerts::rule::{AlertName, Condition, Operator, RuleSpec, Write};
use crate::domain::alerts::{Number, Outcome, UnknownReason};

type R = Result<(), Box<dyn Error>>;

const URL_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";
/// The rule bound where a case is not about it.
const UNBOUNDED: u64 = u64::MAX;

#[test]
fn a_provider_answer_is_cut_to_what_the_row_holds() {
    use crate::domain::alerts::delivery::{Attempted, Receipt};

    let long_error = "é".repeat(5000);
    let long_receipt = "r".repeat(300);
    let cases = [
        (
            "a short rejection as it was",
            Attempted::Failed("answered 401".to_owned()),
            AttemptColumns {
                status: NotificationStatus::Failed,
                last_error: Some("answered 401".to_owned()),
                provider_receipt: None,
            },
        ),
        (
            "a long unconfirmed answer cut by characters",
            Attempted::Retry(long_error.clone()),
            AttemptColumns {
                status: NotificationStatus::Pending,
                last_error: Some("é".repeat(LAST_ERROR_CHARS)),
                provider_receipt: None,
            },
        ),
        (
            "a long rejection cut by characters",
            Attempted::Failed(long_error),
            AttemptColumns {
                status: NotificationStatus::Failed,
                last_error: Some("é".repeat(LAST_ERROR_CHARS)),
                provider_receipt: None,
            },
        ),
        (
            "a long receipt cut to its column",
            Attempted::Sent(Receipt(long_receipt)),
            AttemptColumns {
                status: NotificationStatus::Sent,
                last_error: None,
                provider_receipt: Some("r".repeat(PROVIDER_RECEIPT_CHARS)),
            },
        ),
    ];

    for (case, attempted, expected) in cases {
        assert_eq!(
            attempt_columns(&attempted),
            expected,
            "should record: {case}"
        );
    }
}

/// The in-memory store, on the rules it shares with the SQL one.
mod memory {
    use std::error::Error;

    use chrono::Utc;
    use uuid::Uuid;

    use super::super::memory::MemoryAlerts;
    use crate::domain::alerts::rule::{
        AlertName, AlertStore, AlertStoreError, NotificationStatus, Recording,
    };
    use crate::domain::definition::Page;

    type R = Result<(), Box<dyn Error>>;

    #[tokio::test]
    async fn a_listing_finds_a_name_regardless_of_case() -> R {
        let store = MemoryAlerts::new();
        let mut named = super::write(10);
        named.spec.name = AlertName::parse("PR backlog")?;
        store.create(named, super::UNBOUNDED).await?;

        for needle in ["pr", "PR", "Backlog", "BACKLOG"] {
            let page = store.page(needle, Page::parse(None, None)?).await?;
            assert_eq!(page.total, 1, "should find by `{needle}`");
        }
        assert_eq!(
            store.page("nothing", Page::parse(None, None)?).await?.total,
            0
        );

        Ok(())
    }

    #[tokio::test]
    async fn the_kept_count_bounds_only_settled_history() -> R {
        let store = MemoryAlerts::keeping(3);
        let mut rule = store.create(super::write(10), super::UNBOUNDED).await?;

        for round in 0..5_i128 {
            store
                .record(super::recording(&rule, super::breach(11 + round)))
                .await?;
            store
                .record(super::recording(&rule, super::clear(1)))
                .await?;
        }
        let owed = store
            .notifications(rule.id, Page::parse(None, None)?)
            .await?;
        assert_eq!(owed.len(), 5, "a notification still owed is never trimmed");

        rule = store.set_enabled(rule.id, rule.revision, false).await?;
        rule = store.set_enabled(rule.id, rule.revision, true).await?;
        store
            .record(super::recording(&rule, super::breach(20)))
            .await?;
        let kept = store
            .notifications(rule.id, Page::parse(None, None)?)
            .await?;
        assert_eq!(kept.len(), 3, "the newest three stay: {kept:?}");
        assert_eq!(kept[0].status, NotificationStatus::Pending);
        assert!(
            kept[1..]
                .iter()
                .all(|notification| notification.status == NotificationStatus::Cancelled)
        );

        Ok(())
    }

    #[tokio::test]
    async fn creates_at_the_bound_let_exactly_one_through() -> R {
        let store = MemoryAlerts::new();
        store.create(super::write(10), 2).await?;

        let (left, right) = futures::join!(
            store.create(super::write(10), 2),
            store.create(super::write(10), 2)
        );
        let outcomes = [left, right];
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        assert!(
            outcomes
                .iter()
                .any(|outcome| matches!(outcome, Err(AlertStoreError::TooMany(2)))),
            "{outcomes:?}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn a_check_lands_only_on_its_revision() -> R {
        let store = MemoryAlerts::new();
        let first = store.create(super::write(10), super::UNBOUNDED).await?;
        let second = store.replace(first.id, 1, super::write(20)).await?;

        let stale = store
            .record(Recording {
                rule_id: first.id,
                revision: first.revision,
                outcome: super::breach(12),
                evaluated_at: Utc::now(),
            })
            .await?;
        assert_eq!(stale, crate::domain::alerts::rule::Recorded::Stale);
        assert_eq!(
            store.get(Uuid::now_v7()).await?,
            None,
            "an absent id reads as nothing"
        );
        assert_eq!(second.revision, 2);

        Ok(())
    }
}

/// The tests share one database and run at once, and the ledger takes one
/// writer, so the migrations are applied once per process.
static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn store() -> Option<MariaAlerts> {
    let url = std::env::var(URL_VAR).ok().filter(|url| !url.is_empty())?;
    let db = sea_orm::Database::connect(url)
        .await
        .unwrap_or_else(|error| panic!("{URL_VAR} must be reachable: {error}"));
    MIGRATED
        .get_or_init(|| async {
            <crate::store::definitions::migration::Migrator as sea_orm_migration::MigratorTrait>::up(
                &db, None,
            )
            .await
            .unwrap_or_else(|error| panic!("the migrations must apply: {error}"));
        })
        .await;

    Some(MariaAlerts::new(db, 3))
}

fn spec(threshold: i128) -> RuleSpec {
    RuleSpec {
        name: AlertName::parse("Too many open PRs").unwrap_or_else(|error| panic!("{error}")),
        metric: DefinitionName::parse("prs-open").unwrap_or_else(|error| panic!("{error}")),
        column: "total".to_owned(),
        condition: Condition {
            operator: Operator::Gt,
            threshold: Number::Int(threshold),
        },
        range: None,
        interval_secs: 300,
        destination: "ops".to_owned(),
    }
}

fn write(threshold: i128) -> Write {
    Write {
        spec: spec(threshold),
        enabled: true,
        actor: Some(Uuid::nil()),
    }
}

fn breach(value: i128) -> Outcome {
    Outcome::Valid {
        value: Number::Int(value),
        breached: true,
    }
}

fn clear(value: i128) -> Outcome {
    Outcome::Valid {
        value: Number::Int(value),
        breached: false,
    }
}

fn recording(rule: &AlertRule, outcome: Outcome) -> Recording {
    Recording {
        rule_id: rule.id,
        revision: rule.revision,
        outcome,
        evaluated_at: Utc::now(),
    }
}

#[test]
fn only_a_deadlock_sends_a_create_round_again() {
    #[derive(Debug)]
    struct Answered(&'static str);

    impl std::fmt::Display for Answered {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl Error for Answered {}

    impl sea_orm::sqlx::error::DatabaseError for Answered {
        fn message(&self) -> &str {
            self.0
        }

        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some(self.0.into())
        }

        fn kind(&self) -> sea_orm::sqlx::error::ErrorKind {
            sea_orm::sqlx::error::ErrorKind::Other
        }

        fn as_error(&self) -> &(dyn Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn Error + Send + Sync + 'static> {
            self
        }
    }

    let answered = |code| {
        AlertStoreError::Database(sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(
            std::sync::Arc::new(sea_orm::sqlx::Error::Database(Box::new(Answered(code)))),
        )))
    };
    let cases = [("40001", true), ("23000", false), ("HY000", false)];

    for (code, again) in cases {
        assert_eq!(
            is_deadlock(&answered(code)),
            again,
            "should go again: {code}"
        );
    }
    assert!(
        !is_deadlock(&AlertStoreError::TooMany(3)),
        "a refusal is final"
    );
}

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_MARIADB_URL"]
async fn a_rule_round_trips_and_every_write_bumps_its_revision() -> R {
    let _turn = crate::live_mariadb::TURN.lock().await;
    let Some(store) = store().await else {
        return Ok(());
    };
    let created = store.create(write(10), UNBOUNDED).await?;
    assert_eq!(created.revision, 1);
    assert_eq!(created.spec, spec(10));
    assert_eq!(store.get(created.id).await?, Some(created.clone()));

    let replaced = store.replace(created.id, 1, write(20)).await?;
    assert_eq!(replaced.revision, 2);
    assert_eq!(replaced.spec, spec(20));
    assert_eq!(replaced.id, created.id);

    let twin = store.create(write(10), UNBOUNDED).await?;
    assert_ne!(twin.id, created.id, "two alerts may share a name");

    let deleted = store.delete(created.id).await?;
    assert_eq!(deleted.map(|rule| rule.id), Some(created.id));
    assert_eq!(store.get(created.id).await?, None);
    store.delete(twin.id).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_MARIADB_URL"]
async fn a_write_at_the_wrong_revision_is_refused_and_an_absent_id_too() -> R {
    let _turn = crate::live_mariadb::TURN.lock().await;
    let Some(store) = store().await else {
        return Ok(());
    };
    let created = store.create(write(10), UNBOUNDED).await?;

    assert!(matches!(
        store.replace(created.id, 7, write(20)).await,
        Err(AlertStoreError::Conflict {
            current: 1,
            expected: 7,
            ..
        })
    ));
    assert!(matches!(
        store.replace(Uuid::now_v7(), 1, write(20)).await,
        Err(AlertStoreError::NotFound(_))
    ));
    assert_eq!(store.get(created.id).await?, Some(created.clone()));

    store.delete(created.id).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_MARIADB_URL"]
async fn one_logical_breach_owes_one_notification_until_a_valid_clear() -> R {
    let _turn = crate::live_mariadb::TURN.lock().await;
    let Some(store) = store().await else {
        return Ok(());
    };
    let rule = store.create(write(10), UNBOUNDED).await?;

    let first = store.record(recording(&rule, breach(12))).await?;
    let Recorded::Accepted(first) = first else {
        panic!("the first breach is recorded: {first:?}");
    };
    let Some(owed) = first.notification else {
        panic!("the first breach owes a notification");
    };
    assert_eq!(owed.value, Number::Int(12));
    assert_eq!(owed.status, NotificationStatus::Pending);
    assert_eq!(first.rule.state.last_valid_breached, Some(true));
    assert!(first.rule.state.breached_since.is_some());

    for outcome in [
        breach(13),
        Outcome::Unknown(UnknownReason::Timeout),
        breach(14),
    ] {
        let again = store.record(recording(&rule, outcome)).await?;
        let Recorded::Accepted(again) = again else {
            panic!("a check at the live revision is recorded: {again:?}");
        };
        assert_eq!(again.notification, None, "should stay silent: {outcome:?}");
        assert_eq!(
            again.rule.state.breached_since,
            first.rule.state.breached_since
        );
    }

    let cleared = store.record(recording(&rule, clear(3))).await?;
    let Recorded::Accepted(cleared) = cleared else {
        panic!("a clear is recorded: {cleared:?}");
    };
    assert_eq!(cleared.notification, None, "a clear owes nothing");
    assert_eq!(cleared.rule.state.last_valid_breached, Some(false));
    assert_eq!(cleared.rule.state.breached_since, None);

    let renewed = store.record(recording(&rule, breach(15))).await?;
    let Recorded::Accepted(renewed) = renewed else {
        panic!("a breach after a clear is recorded: {renewed:?}");
    };
    assert!(renewed.notification.is_some());

    let listed = store
        .notifications(rule.id, Page::parse(None, None)?)
        .await?;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].value, Number::Int(15));

    store.delete(rule.id).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_MARIADB_URL"]
async fn a_check_from_a_replaced_or_disabled_revision_is_not_recorded() -> R {
    let _turn = crate::live_mariadb::TURN.lock().await;
    let Some(store) = store().await else {
        return Ok(());
    };
    let first = store.create(write(10), UNBOUNDED).await?;
    let second = store.replace(first.id, 1, write(20)).await?;

    assert_eq!(
        store.record(recording(&first, breach(12))).await?,
        Recorded::Stale
    );

    let off = store.set_enabled(first.id, second.revision, false).await?;
    assert!(!off.enabled);
    assert_eq!(off.revision, 3);
    assert_eq!(
        store.record(recording(&second, breach(12))).await?,
        Recorded::Stale
    );
    assert_eq!(
        store.record(recording(&off, breach(12))).await?,
        Recorded::Stale
    );

    let unchanged = store.get(first.id).await?;
    assert_eq!(
        unchanged.and_then(|rule| rule.state.last_evaluated_at),
        None
    );

    store.delete(first.id).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_MARIADB_URL"]
async fn disabling_withdraws_what_is_pending_and_the_kept_count_bounds_only_settled_history() -> R {
    let _turn = crate::live_mariadb::TURN.lock().await;
    let Some(store) = store().await else {
        return Ok(());
    };
    let mut rule = store.create(write(10), UNBOUNDED).await?;

    for round in 0..5_i128 {
        store.record(recording(&rule, breach(11 + round))).await?;
        store.record(recording(&rule, clear(1))).await?;
    }
    let owed = store
        .notifications(rule.id, Page::parse(None, None)?)
        .await?;
    assert_eq!(owed.len(), 5, "a notification still owed is never trimmed");
    assert_eq!(owed[0].value, Number::Int(15));

    rule = store.set_enabled(rule.id, rule.revision, false).await?;
    let withdrawn = store
        .notifications(rule.id, Page::parse(None, None)?)
        .await?;
    assert!(
        withdrawn
            .iter()
            .all(|notification| notification.status == NotificationStatus::Cancelled),
        "{withdrawn:?}"
    );

    rule = store.set_enabled(rule.id, rule.revision, true).await?;
    store.record(recording(&rule, breach(20))).await?;
    let kept = store
        .notifications(rule.id, Page::parse(None, None)?)
        .await?;
    assert_eq!(kept.len(), 3, "the store keeps the newest three: {kept:?}");
    assert_eq!(kept[0].value, Number::Int(20));
    assert_eq!(kept[0].status, NotificationStatus::Pending);

    store.delete(rule.id).await?;
    assert!(
        store
            .notifications(rule.id, Page::parse(None, None)?)
            .await?
            .is_empty()
    );

    Ok(())
}

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_MARIADB_URL"]
async fn only_enabled_rules_are_listed_for_scheduling_and_a_listing_finds_by_name() -> R {
    let _turn = crate::live_mariadb::TURN.lock().await;
    let Some(store) = store().await else {
        return Ok(());
    };
    let needle = format!("listing {}", Uuid::now_v7().simple());
    let mut named = write(10);
    named.spec.name = AlertName::parse(&needle)?;
    let on_rule = store.create(named.clone(), UNBOUNDED).await?;
    let off_rule = store.create(named, UNBOUNDED).await?;
    store
        .set_enabled(off_rule.id, off_rule.revision, false)
        .await?;

    let enabled = store.enabled().await?;
    assert!(enabled.iter().any(|rule| rule.id == on_rule.id));
    assert!(enabled.iter().all(|rule| rule.id != off_rule.id));

    let page = store.page(&needle, Page::parse(None, None)?).await?;
    assert_eq!(page.total, 2);
    let mut ids: Vec<Uuid> = page.alerts.iter().map(|alert| alert.id).collect();
    ids.sort_unstable();
    let mut expected = vec![on_rule.id, off_rule.id];
    expected.sort_unstable();
    assert_eq!(ids, expected);
    assert!(
        page.alerts
            .iter()
            .all(|alert| alert.name.as_str() == needle)
    );

    store.delete(on_rule.id).await?;
    store.delete(off_rule.id).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_MARIADB_URL"]
async fn creates_racing_at_the_bound_never_exceed_it() -> R {
    let _turn = crate::live_mariadb::TURN.lock().await;
    let Some(store) = store().await else {
        return Ok(());
    };
    let before = TotalRow::find_by_statement(Statement::from_string(
        DbBackend::MySql,
        "SELECT COUNT(*) AS total FROM alert_rules",
    ))
    .one(&store.db)
    .await?
    .map_or(0, |row| u64::try_from(row.total).unwrap_or(0));
    let bound = before + 3;

    let raced = futures::future::join_all((0..6).map(|_| store.create(write(10), bound))).await;

    let created: Vec<AlertRule> = raced
        .iter()
        .filter_map(|outcome| outcome.as_ref().ok().cloned())
        .collect();
    let refused = raced
        .iter()
        .filter(
            |outcome| matches!(outcome, Err(AlertStoreError::TooMany(limit)) if *limit == bound),
        )
        .count();
    assert!(
        !created.is_empty() && created.len() <= 3,
        "at most three land: {raced:?}"
    );
    assert_eq!(
        created.len() + refused,
        6,
        "every other create is refused by the bound: {raced:?}"
    );

    for rule in created {
        store.delete(rule.id).await?;
    }

    Ok(())
}

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_MARIADB_URL"]
async fn a_send_is_recorded_only_on_a_notification_still_owed() -> R {
    use crate::domain::alerts::delivery::{Attempted, Receipt};

    let _turn = crate::live_mariadb::TURN.lock().await;

    let Some(store) = store().await else {
        return Ok(());
    };
    let rule = store.create(write(10), UNBOUNDED).await?;
    let Recorded::Accepted(first) = store.record(recording(&rule, breach(12))).await? else {
        panic!("the breach is recorded");
    };
    let owed = first
        .notification
        .unwrap_or_else(|| panic!("the breach owes a notification"));
    assert!(
        store
            .pending_notifications()
            .await?
            .iter()
            .any(|pending| pending.id == owed.id)
    );

    let retried = store
        .record_attempt(owed.id, &Attempted::Retry("answered 502".to_owned()))
        .await?
        .unwrap_or_else(|| panic!("a pending notification takes an attempt"));
    assert_eq!(retried.status, NotificationStatus::Pending);
    assert_eq!(retried.attempts, 1);
    assert_eq!(retried.last_error.as_deref(), Some("answered 502"));

    let verbose = store
        .record_attempt(owed.id, &Attempted::Retry("x".repeat(5000)))
        .await?
        .unwrap_or_else(|| panic!("a long answer is still recorded"));
    assert_eq!(
        verbose.last_error.map(|error| error.chars().count()),
        Some(LAST_ERROR_CHARS),
        "the row keeps what its column holds"
    );

    let sent = store
        .record_attempt(owed.id, &Attempted::Sent(Receipt("m-1".to_owned())))
        .await?
        .unwrap_or_else(|| panic!("a pending notification takes an attempt"));
    assert_eq!(sent.status, NotificationStatus::Sent);
    assert_eq!(sent.attempts, 3);
    assert_eq!(sent.provider_receipt.as_deref(), Some("m-1"));
    assert!(
        store
            .pending_notifications()
            .await?
            .iter()
            .all(|pending| pending.id != owed.id)
    );

    let again = store
        .record_attempt(owed.id, &Attempted::Failed("late".to_owned()))
        .await?;
    assert_eq!(again, None, "a sent notification takes no more attempts");
    assert_eq!(
        store.notification(owed.id).await?.map(|kept| kept.status),
        Some(NotificationStatus::Sent)
    );

    store.delete(rule.id).await?;

    Ok(())
}
