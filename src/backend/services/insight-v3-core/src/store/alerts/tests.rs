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

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_MARIADB_URL"]
async fn a_rule_round_trips_and_every_write_bumps_its_revision() -> R {
    let Some(store) = store().await else {
        return Ok(());
    };
    let created = store.create(write(10)).await?;
    assert_eq!(created.revision, 1);
    assert_eq!(created.spec, spec(10));
    assert_eq!(store.get(created.id).await?, Some(created.clone()));

    let replaced = store.replace(created.id, 1, write(20)).await?;
    assert_eq!(replaced.revision, 2);
    assert_eq!(replaced.spec, spec(20));
    assert_eq!(replaced.id, created.id);

    let twin = store.create(write(10)).await?;
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
    let Some(store) = store().await else {
        return Ok(());
    };
    let created = store.create(write(10)).await?;

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
    let Some(store) = store().await else {
        return Ok(());
    };
    let rule = store.create(write(10)).await?;

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
    let Some(store) = store().await else {
        return Ok(());
    };
    let first = store.create(write(10)).await?;
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
async fn disabling_withdraws_what_is_pending_and_the_kept_count_bounds_history() -> R {
    let Some(store) = store().await else {
        return Ok(());
    };
    let mut rule = store.create(write(10)).await?;

    for round in 0..5_i128 {
        store.record(recording(&rule, breach(11 + round))).await?;
        store.record(recording(&rule, clear(1))).await?;
    }
    let kept = store
        .notifications(rule.id, Page::parse(None, None)?)
        .await?;
    assert_eq!(kept.len(), 3, "the store keeps the newest three");
    assert_eq!(kept[0].value, Number::Int(15));

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
    let Some(store) = store().await else {
        return Ok(());
    };
    let needle = format!("listing {}", Uuid::now_v7().simple());
    let mut named = write(10);
    named.spec.name = AlertName::parse(&needle)?;
    let on_rule = store.create(named.clone()).await?;
    let off_rule = store.create(named).await?;
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
