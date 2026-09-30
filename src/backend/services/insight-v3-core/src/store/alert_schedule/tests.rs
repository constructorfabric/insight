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
        let state = state(&warehouse().await, schedule.clone(), store.clone()).await;

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

        Ok(())
    }
}
