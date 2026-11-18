//! Live `ClickHouse` tests for a composite metric: two stored metrics over
//! tables of their own, combined, run for real through the same path a reader
//! takes, and asserted on the rows that come back.
//!
//! Each test owns its tables, named after a fresh id, and drops them again.
//! The rows are synthetic and written here. Like the window tests, these skip
//! when `INTEGRATION_TESTS_CLICKHOUSE_URL` is unset.

use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::definition::{DefinitionId, DefinitionKind};
use crate::domain::metric_run::MetricRuns;
use crate::domain::query::metric_query::{MetricRunner, People, RunResult};
use crate::domain::query::time_window::WindowRequest;
use crate::store::datasets::memory::MemoryDatasets;
use crate::store::definitions::memory::MemoryDefinitions;
use crate::window_live_tests::{execute, live_client};

const COMPOSITE: &str = "incidents-per-deploy";

/// A warehouse holding the tables a case made, and a store holding the
/// metrics over them.
struct Stand {
    client: insight_clickhouse::Client,
    definitions: MemoryDefinitions,
    datasets: MemoryDatasets,
    suffix: String,
    /// Every relation this case made, dropped when it is done.
    made: Vec<String>,
}

impl Stand {
    fn new(client: insight_clickhouse::Client) -> Self {
        Self {
            client,
            definitions: MemoryDefinitions::new(),
            datasets: MemoryDatasets::at(chrono::Utc::now()),
            suffix: Uuid::now_v7().simple().to_string(),
            made: Vec::new(),
        }
    }

    /// Makes a table of this case's own under a name that starts `prefix`.
    async fn table(&mut self, prefix: &str, schema: &str, rows: &[&str]) -> String {
        let table = format!("{prefix}_{}", self.suffix);
        execute(&self.client, &format!("CREATE TABLE {table} {schema}")).await;
        self.made.push(format!("TABLE {table}"));
        execute(
            &self.client,
            &format!("INSERT INTO {table} VALUES {}", rows.join(", ")),
        )
        .await;

        table
    }

    async fn metric(&self, id: &str, body: &Value) {
        self.definitions
            .seed(DefinitionKind::Metric, &metric_id(id), body)
            .await
            .unwrap_or_else(|error| panic!("the store takes `{id}`: {error}"));
    }

    async fn run(&self, people: People, request: &WindowRequest) -> RunResult {
        let runner = MetricRunner::new(
            insight_clickhouse::Client::new(self.client.config().clone()),
            people,
        );
        let runs = MetricRuns::new(
            &self.definitions,
            &runner,
            &self.datasets,
            "insight_datasets",
        );

        runs.run(&metric_id(COMPOSITE), request)
            .await
            .unwrap_or_else(|error| panic!("the composite runs: {error}"))
    }

    async fn drop_all(&self) {
        for made in self.made.iter().rev() {
            execute(&self.client, &format!("DROP {made}")).await;
        }
    }
}

fn metric_id(id: &str) -> DefinitionId {
    DefinitionId::parse(id).unwrap_or_else(|error| panic!("`{id}` is an id: {error}"))
}

fn request(range: Option<&str>) -> WindowRequest {
    WindowRequest::parse(range, None).unwrap_or_else(|error| panic!("`{range:?}` parses: {error}"))
}

/// Every cell as text: a string as it reads, anything else as JSON writes it.
fn cells(result: &RunResult) -> Vec<Vec<String>> {
    result
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect()
}

fn rows(expected: &[&[&str]]) -> Vec<Vec<String>> {
    expected
        .iter()
        .map(|row| row.iter().map(|cell| (*cell).to_owned()).collect())
        .collect()
}

/// Incidents of severity 2 and up, and deploys to production, per team. Each
/// side holds a team the other has no row for, and rows its filter leaves out.
async fn incidents_and_deploys(stand: &mut Stand) {
    let incidents = stand
        .table(
            "incidents",
            "(team String, severity Int32, occurred_at DateTime) \
             ENGINE = MergeTree ORDER BY occurred_at",
            &[
                "('core', 3, '2026-09-01 08:00:00')",
                "('core', 2, '2026-09-01 12:00:00')",
                "('core', 1, '2026-09-01 13:00:00')",
                "('core', 4, '2026-09-02 09:00:00')",
                "('infra', 2, '2026-09-01 10:00:00')",
                "('web', 5, '2026-09-02 11:00:00')",
            ],
        )
        .await;
    let deploys = stand
        .table(
            "deploys",
            "(team String, env String, occurred_at DateTime) \
             ENGINE = MergeTree ORDER BY occurred_at",
            &[
                "('core', 'prod', '2026-09-01 09:00:00')",
                "('core', 'prod', '2026-09-02 09:00:00')",
                "('core', 'staging', '2026-09-02 10:00:00')",
                "('infra', 'prod', '2026-09-01 09:00:00')",
                "('infra', 'prod', '2026-09-01 10:00:00')",
                "('infra', 'prod', '2026-09-02 09:00:00')",
                "('infra', 'prod', '2026-09-02 10:00:00')",
                "('data', 'prod', '2026-09-01 15:00:00')",
                "('data', 'prod', '2026-09-01 16:00:00')",
            ],
        )
        .await;

    let per_team = |table: &str, filter: Value| {
        json!({
            "table": table,
            "time": { "column": "occurred_at" },
            "fields": [
                { "column": "team", "type": "string", "as_name": "team" },
                { "agg": "count", "type": "int", "as_name": "count" }
            ],
            "group_by": ["team"],
            "filters": [filter]
        })
    };
    stand
        .metric(
            "incident-counts",
            &per_team(
                &incidents,
                json!({ "column": "severity", "type": "int", "op": "gte", "value": 2 }),
            ),
        )
        .await;
    stand
        .metric(
            "deploy-counts",
            &per_team(
                &deploys,
                json!({ "column": "env", "type": "string", "op": "eq", "value": "prod" }),
            ),
        )
        .await;
    stand
        .metric(
            COMPOSITE,
            &json!({
                "combine": { "incidents": "incident-counts", "deploys": "deploy-counts" },
                "group_by": ["team"],
                "fields": [
                    { "as_name": "incidents", "field": "incidents.count" },
                    { "as_name": "deploys", "field": "deploys.count" },
                    { "as_name": "incidents_per_deploy", "divide": ["incidents", "deploys"] }
                ]
            }),
        )
        .await;
}

/// A team only one side has a row for reads 0 for the count it lacks, and a
/// ratio over a missing denominator reads nothing rather than a number.
#[tokio::test]
async fn a_composite_answers_every_team_either_side_has_once() {
    let Some(client) = live_client() else {
        return;
    };
    let mut stand = Stand::new(client);
    incidents_and_deploys(&mut stand).await;

    let result = stand.run(People::new("identity"), &request(None)).await;

    assert_eq!(
        result.columns,
        ["team", "incidents", "deploys", "incidents_per_deploy"]
    );
    assert_eq!(
        cells(&result),
        rows(&[
            &["core", "3", "2", "1.5"],
            &["data", "0", "2", "0"],
            &["infra", "1", "4", "0.25"],
            &["web", "1", "0", "null"],
        ])
    );

    stand.drop_all().await;
}

#[tokio::test]
async fn a_windowed_composite_answers_each_team_once_per_bucket() {
    let Some(client) = live_client() else {
        return;
    };
    let mut stand = Stand::new(client);
    incidents_and_deploys(&mut stand).await;

    let result = stand
        .run(
            People::new("identity"),
            &request(Some("2026-09-01/2026-09-03")),
        )
        .await;

    assert_eq!(
        result.columns,
        [
            "bucket",
            "team",
            "incidents",
            "deploys",
            "incidents_per_deploy"
        ]
    );
    assert_eq!(
        cells(&result),
        rows(&[
            &["2026-09-01 00:00:00", "core", "2", "1", "2"],
            &["2026-09-01 00:00:00", "data", "0", "2", "0"],
            &["2026-09-01 00:00:00", "infra", "1", "2", "0.5"],
            &["2026-09-02 00:00:00", "core", "1", "1", "1"],
            &["2026-09-02 00:00:00", "infra", "0", "2", "0"],
            &["2026-09-02 00:00:00", "web", "1", "0", "null"],
        ])
    );
    assert_eq!(
        result.clock.map(|clock| clock.field),
        Some("occurred_at".to_owned())
    );

    stand.drop_all().await;
}

/// A child that resolves a person's name opens with the `WITH` clauses that
/// read identity; stacked as a subquery, it still names each person once.
#[tokio::test]
async fn a_child_resolving_people_by_name_combines_on_the_name() {
    let Some(client) = live_client() else {
        return;
    };
    let mut stand = Stand::new(client);
    let people = format!("people_{}", stand.suffix);
    execute(&stand.client, &format!("CREATE DATABASE {people}")).await;
    stand.made.push(format!("DATABASE {people}"));
    execute(
        &stand.client,
        &format!(
            "CREATE TABLE {people}.identity_persons \
             (person_id UUID, value_type String, value_effective String, created_at DateTime) \
             ENGINE = MergeTree ORDER BY person_id"
        ),
    )
    .await;
    execute(
        &stand.client,
        &format!(
            "INSERT INTO {people}.identity_persons VALUES \
             ('00000000-0000-0000-0000-000000000001', 'email', 'ada@example.com', '2026-01-01 00:00:00'), \
             ('00000000-0000-0000-0000-000000000001', 'display_name', 'Ada Example', '2026-01-01 00:00:00')"
        ),
    )
    .await;

    let incidents = stand
        .table(
            "incidents",
            "(owner_email String) ENGINE = MergeTree ORDER BY owner_email",
            &[
                "('ada@example.com')",
                "('ada@example.com')",
                "('nobody@example.com')",
            ],
        )
        .await;
    let deploys = stand
        .table(
            "deploys",
            "(owner String) ENGINE = MergeTree ORDER BY owner",
            &["('Ada Example')"],
        )
        .await;
    stand
        .metric(
            "incident-counts",
            &json!({
                "table": incidents,
                "fields": [
                    {
                        "column": "owner_email", "type": "string", "person": "email",
                        "as_name": "owner"
                    },
                    { "agg": "count", "type": "int", "as_name": "count" }
                ],
                "group_by": ["owner"]
            }),
        )
        .await;
    stand
        .metric(
            "deploy-counts",
            &json!({
                "table": deploys,
                "fields": [
                    { "column": "owner", "type": "string", "as_name": "owner" },
                    { "agg": "count", "type": "int", "as_name": "count" }
                ],
                "group_by": ["owner"]
            }),
        )
        .await;
    stand
        .metric(
            COMPOSITE,
            &json!({
                "combine": { "incidents": "incident-counts", "deploys": "deploy-counts" },
                "group_by": ["owner"],
                "fields": [
                    { "as_name": "incidents", "field": "incidents.count" },
                    { "as_name": "deploys", "field": "deploys.count" }
                ]
            }),
        )
        .await;

    let result = stand.run(People::new(people), &request(None)).await;

    assert_eq!(
        cells(&result),
        rows(&[
            &["Ada Example", "2", "1"],
            &["nobody@example.com", "1", "0"],
        ])
    );

    stand.drop_all().await;
}
