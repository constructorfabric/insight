//! Against a real Redis, because the claims are about what `BullMQ` holds:
//! a scheduler per rule, replaced not duplicated, gone when removed. Run
//! with `INTEGRATION_TESTS_REDIS_URL` set.

use std::error::Error;

use uuid::Uuid;

use super::*;
use crate::domain::alerts::evaluation::EvaluationJob;
use crate::domain::alerts::schedule::Scheduled;

type R = Result<(), Box<dyn Error>>;

const URL_VAR: &str = "INTEGRATION_TESTS_REDIS_URL";

async fn schedule() -> Option<RedisSchedule> {
    let url = std::env::var(URL_VAR).ok().filter(|url| !url.is_empty())?;

    Some(
        RedisSchedule::connect(&url)
            .await
            .unwrap_or_else(|error| panic!("{URL_VAR} must be reachable: {error}")),
    )
}

fn scheduled(rule_id: Uuid, revision: u32, every_secs: u32) -> Scheduled {
    Scheduled {
        job: EvaluationJob { rule_id, revision },
        every_secs,
    }
}

#[tokio::test]
#[ignore = "needs INTEGRATION_TESTS_REDIS_URL"]
async fn a_rule_is_scheduled_once_however_often_it_is_written() -> R {
    let Some(schedule) = schedule().await else {
        return Ok(());
    };
    let rule_id = Uuid::now_v7();

    schedule.upsert(scheduled(rule_id, 1, 60)).await?;
    schedule.upsert(scheduled(rule_id, 2, 120)).await?;
    schedule.upsert(scheduled(rule_id, 3, 120)).await?;

    let held = schedule.scheduled().await?;
    assert_eq!(held.iter().filter(|held| **held == rule_id).count(), 1);

    schedule.remove(rule_id).await?;
    schedule.remove(rule_id).await?;
    assert!(!schedule.scheduled().await?.contains(&rule_id));

    Ok(())
}

#[test]
fn a_scheduler_key_names_its_rule_and_nothing_else_is_a_rule() {
    let rule_id = Uuid::now_v7();

    assert_eq!(rule_of(&scheduler_id(rule_id)), Some(rule_id));
    assert_eq!(rule_of("rule:not-a-uuid"), None);
    assert_eq!(rule_of("other:thing"), None);
}

/// The whole loop against a real Redis: a rule is scheduled, the worker
/// takes its first check, runs the metric against a warehouse that answers
/// one row over the threshold, and the store ends up owing a notification.
mod worker {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;

    use super::super::{AlertWorker, Checks, RedisSchedule};
    use crate::api::{Alerts, AppState};
    use crate::chat::ChatClient;
    use crate::domain::alerts::evaluation::Evaluator;
    use crate::domain::alerts::rule::{
        AlertName, AlertStore, Condition, Operator, RuleSpec, Write,
    };
    use crate::domain::alerts::schedule::{AlertSchedule as _, Scheduled};
    use crate::domain::alerts::{Destinations, Limits, Number};
    use crate::domain::definition::{DefinitionKind, DefinitionName, Definitions as _, Page};
    use crate::store::alerts::memory::MemoryAlerts;
    use crate::store::definitions::memory::MemoryDefinitions;

    struct StateChecks(Arc<AppState>);

    impl Checks for StateChecks {
        fn evaluator(&self) -> Evaluator<'_> {
            self.0
                .alert_evaluator()
                .unwrap_or_else(|| panic!("the state has alerts"))
        }

        fn deliveries(&self) -> &dyn crate::domain::alerts::delivery::Deliveries {
            self.0
                .alert_deliveries()
                .unwrap_or_else(|| panic!("the state has alerts"))
        }
    }

    /// A warehouse that answers one row: `total` above any threshold here.
    async fn warehouse() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("the upstream must bind: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("the upstream must have an address: {error}"));
        let upstream = Router::new().route(
            "/",
            post(|_sql: String| async {
                Json(json!({
                    "meta": [{"name": "total", "type": "UInt64"}],
                    "data": [{"total": "42"}]
                }))
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, upstream).await;
        });

        format!("http://{address}")
    }

    async fn state(
        warehouse_url: &str,
        schedule: Arc<RedisSchedule>,
        store: Arc<MemoryAlerts>,
        deliveries: Arc<dyn crate::domain::alerts::delivery::Deliveries>,
    ) -> Arc<AppState> {
        let definitions = Arc::new(MemoryDefinitions::new());
        let metric = DefinitionName::parse("prs-open").unwrap_or_else(|error| panic!("{error}"));
        definitions
            .put(
                DefinitionKind::Metric,
                &metric,
                &json!({"dataset": "commits", "fields": [{"agg": "count", "type": "int", "as_name": "total"}]}),
            )
            .await
            .unwrap_or_else(|error| panic!("the metric is stored: {error}"));
        let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            warehouse_url,
            "insight",
        ));

        Arc::new(
            AppState::new(
                crate::domain::query::metric_query::MetricRunner::new(
                    client,
                    crate::domain::query::metric_query::People::new("identity"),
                ),
                definitions,
                ChatClient::keyless(),
                crate::store::identity::IdentityClient::fixed(true),
                crate::api::Datasets::holding(
                    warehouse_url,
                    &[(
                        "commits",
                        json!({
                            "title": "Commits",
                            "fields": [{ "name": "lines", "path": "lines", "type": "int" }]
                        }),
                    )],
                ),
                crate::store::catalog::Catalog::fixed(Vec::new()),
            )
            .with_alerts(Alerts {
                store,
                schedule,
                deliveries,
                providers: std::collections::BTreeMap::new(),
                limits: Limits::default(),
                destinations: Destinations::new(std::collections::BTreeMap::from([(
                    "ops".to_owned(),
                    "discord".to_owned(),
                )])),
            }),
        )
    }

    #[tokio::test]
    #[ignore = "needs INTEGRATION_TESTS_REDIS_URL"]
    async fn a_scheduled_rule_is_checked_by_the_worker_and_owes_its_notification()
    -> Result<(), Box<dyn std::error::Error>> {
        let Some(url) = std::env::var(super::URL_VAR)
            .ok()
            .filter(|url| !url.is_empty())
        else {
            return Ok(());
        };
        let schedule = Arc::new(RedisSchedule::connect(&url).await?);
        let store = Arc::new(MemoryAlerts::new());
        let deliveries = Arc::new(crate::store::alert_schedule::memory::MemoryDeliveries::new());
        let state = state(
            &warehouse().await,
            schedule.clone(),
            store.clone(),
            deliveries.clone(),
        )
        .await;

        let rule = store
            .create(Write {
                spec: RuleSpec {
                    name: AlertName::parse("Worker probe")?,
                    metric: DefinitionName::parse("prs-open")?,
                    column: "total".to_owned(),
                    condition: Condition {
                        operator: Operator::Gt,
                        threshold: Number::Int(10),
                    },
                    range: None,
                    interval_secs: 3600,
                    destination: "ops".to_owned(),
                },
                enabled: true,
                actor: None,
            })
            .await?;
        schedule.upsert(Scheduled::of(&rule)).await?;

        let worker = AlertWorker::start(
            &url,
            Arc::new(StateChecks(state)),
            1,
            Duration::from_secs(30),
        )
        .await?;

        let mut owed = Vec::new();
        for _ in 0..100 {
            owed = store
                .notifications(rule.id, Page::parse(None, None)?)
                .await?;
            if !owed.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        worker.stop().await;
        schedule.remove(rule.id).await?;

        assert_eq!(owed.len(), 1, "one check, one notification: {owed:?}");
        assert_eq!(owed[0].value, Number::Int(42));
        let checked = store
            .get(rule.id)
            .await?
            .unwrap_or_else(|| panic!("the rule stays"));
        assert_eq!(checked.state.last_valid_breached, Some(true));
        assert_eq!(
            deliveries.queued(),
            vec![owed[0].id],
            "the owed notification is queued"
        );

        Ok(())
    }
}

/// Delivery end to end against a real Redis: an owed notification is
/// queued, the delivery worker sends it to a server playing the provider,
/// and the row ends up sent with the provider's receipt — or, after an
/// unconfirmed answer, retried and then sent.
mod delivery {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use axum::Router;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use chrono::Utc;
    use secrecy::SecretString;
    use serde_json::json;
    use uuid::Uuid;

    use super::super::{DeliveryWorker, RedisDeliveries, Sends};
    use crate::domain::alerts::Number;
    use crate::domain::alerts::delivery::{Deliverer, Deliveries as _, Provider};
    use crate::domain::alerts::rule::{
        AlertName, AlertStore as _, Condition, Notification, NotificationStatus, Operator,
        RuleSpec, Write,
    };
    use crate::domain::definition::DefinitionName;
    use crate::store::alerts::memory::MemoryAlerts;
    use crate::store::providers::Discord;

    struct Fixture {
        store: Arc<MemoryAlerts>,
        providers: std::collections::BTreeMap<String, Arc<dyn Provider>>,
    }

    impl Sends for Fixture {
        fn deliverer(&self) -> Deliverer<'_> {
            Deliverer::new(self.store.as_ref(), &self.providers)
        }
    }

    #[derive(Clone)]
    struct Flaky {
        hits: Arc<AtomicUsize>,
        fail_first: usize,
    }

    async fn webhook(State(flaky): State<Flaky>) -> impl IntoResponse {
        let hit = flaky.hits.fetch_add(1, Ordering::SeqCst);
        if hit < flaky.fail_first {
            return (StatusCode::BAD_GATEWAY, axum::Json(json!({})));
        }

        (
            StatusCode::OK,
            axum::Json(json!({"id": format!("msg-{hit}")})),
        )
    }

    async fn discord_playing(fail_first: usize) -> (Arc<dyn Provider>, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route("/hook", post(webhook))
            .with_state(Flaky {
                hits: hits.clone(),
                fail_first,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("the provider must bind: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("the provider must have an address: {error}"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_else(|error| panic!("a client builds: {error}"));

        (
            Arc::new(Discord::new(
                http,
                SecretString::from(format!("http://{address}/hook")),
            )),
            hits,
        )
    }

    async fn owed(store: &MemoryAlerts) -> Notification {
        let rule = store
            .create(Write {
                spec: RuleSpec {
                    name: AlertName::parse("Delivery probe")
                        .unwrap_or_else(|error| panic!("{error}")),
                    metric: DefinitionName::parse("prs-open")
                        .unwrap_or_else(|error| panic!("{error}")),
                    column: "total".to_owned(),
                    condition: Condition {
                        operator: Operator::Gt,
                        threshold: Number::Int(10),
                    },
                    range: None,
                    interval_secs: 3600,
                    destination: "ops".to_owned(),
                },
                enabled: true,
                actor: None,
            })
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let recorded = store
            .record(crate::domain::alerts::rule::Recording {
                rule_id: rule.id,
                revision: rule.revision,
                outcome: crate::domain::alerts::Outcome::Valid {
                    value: Number::Int(42),
                    breached: true,
                },
                evaluated_at: Utc::now(),
            })
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let crate::domain::alerts::rule::Recorded::Accepted(accepted) = recorded else {
            panic!("the breach is recorded");
        };

        accepted
            .notification
            .unwrap_or_else(|| panic!("the breach owes a notification"))
    }

    async fn wait_until_settled(store: &MemoryAlerts, id: Uuid) -> Notification {
        for _ in 0..150 {
            let current = store
                .notification(id)
                .await
                .unwrap_or_else(|error| panic!("{error}"))
                .unwrap_or_else(|| panic!("the notification stays"));
            if current.status != NotificationStatus::Pending {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        panic!("the notification was never settled");
    }

    /// A finished job leaves the queue whichever way it ended, so that the
    /// next enqueue of the same id is a new job.
    async fn wait_until_forgotten(deliveries: &RedisDeliveries, id: Uuid) {
        for _ in 0..50 {
            let held = deliveries
                .queue
                .get_job(&id.simple().to_string())
                .await
                .unwrap_or_else(|error| panic!("{error}"));
            if held.is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        panic!("the finished delivery job was kept");
    }

    /// One delivery queue per Redis, so the cases take turns: a worker
    /// started by one would otherwise take the jobs of another.
    static QUEUE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn run(fail_first: usize, attempts: u32) -> Option<(Notification, usize)> {
        let url = std::env::var(super::URL_VAR)
            .ok()
            .filter(|url| !url.is_empty())?;
        let _turn = QUEUE.lock().await;
        let (provider, hits) = discord_playing(fail_first).await;
        let store = Arc::new(MemoryAlerts::new());
        let notification = owed(&store).await;
        let deliveries = RedisDeliveries::connect(&url, attempts, Duration::from_millis(200))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        deliveries
            .enqueue(notification.id)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        deliveries
            .enqueue(notification.id)
            .await
            .unwrap_or_else(|error| panic!("a second enqueue is the same job: {error}"));

        let fixture = Arc::new(Fixture {
            store: store.clone(),
            providers: std::collections::BTreeMap::from([("ops".to_owned(), provider)]),
        });
        let worker = DeliveryWorker::start(&url, fixture, 1, Duration::from_secs(2), attempts)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let settled = wait_until_settled(&store, notification.id).await;
        wait_until_forgotten(&deliveries, notification.id).await;
        worker.stop().await;

        Some((settled, hits.load(Ordering::SeqCst)))
    }

    #[tokio::test]
    #[ignore = "needs INTEGRATION_TESTS_REDIS_URL"]
    async fn an_owed_notification_is_sent_once_and_keeps_the_receipt() {
        let Some((settled, hits)) = run(0, 3).await else {
            return;
        };

        assert_eq!(settled.status, NotificationStatus::Sent);
        assert_eq!(settled.provider_receipt.as_deref(), Some("msg-0"));
        assert_eq!(settled.attempts, 1);
        assert_eq!(hits, 1, "enqueued twice, sent once");
    }

    #[tokio::test]
    #[ignore = "needs INTEGRATION_TESTS_REDIS_URL"]
    async fn an_unconfirmed_send_is_retried_and_then_sent() {
        let Some((settled, hits)) = run(1, 3).await else {
            return;
        };

        assert_eq!(settled.status, NotificationStatus::Sent, "{settled:?}");
        assert_eq!(settled.attempts, 2);
        assert_eq!(settled.provider_receipt.as_deref(), Some("msg-1"));
        assert_eq!(hits, 2);
    }

    #[tokio::test]
    #[ignore = "needs INTEGRATION_TESTS_REDIS_URL"]
    async fn a_send_that_never_confirms_fails_after_the_last_attempt() {
        let Some((settled, hits)) = run(usize::MAX, 2).await else {
            return;
        };

        assert_eq!(settled.status, NotificationStatus::Failed, "{settled:?}");
        assert_eq!(settled.attempts, 2);
        assert!(
            settled
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("502")),
            "{settled:?}"
        );
        assert_eq!(hits, 2);
    }
}
