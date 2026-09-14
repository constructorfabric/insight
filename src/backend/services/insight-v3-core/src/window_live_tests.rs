//! Live `ClickHouse` tests for the windowed metric path: the SQL the compiler
//! emits, executed for real, asserted on the rows that come back.
//!
//! Each test owns a table of its own, named after a fresh id, and drops it
//! again — so the suite is parallel-safe and needs nothing seeded. The rows
//! are synthetic and written here.
//!
//! These skip silently when `INTEGRATION_TESTS_CLICKHOUSE_URL` is unset, the
//! convention the analytics live tests use; optional auth comes from
//! `INTEGRATION_TESTS_CLICKHOUSE_USER` / `_PASSWORD`. They are not `#[ignore]`d
//! because this crate's CI runs plain `cargo test`, which would never reach an
//! ignored test.

use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::anchor::Anchor;
use crate::catalog::TableEngine;
use crate::metric_query::{MetricQuery, MetricRunner, People, RunResult};
use crate::time_window::WindowRequest;

const URL_VAR: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";

struct Stand {
    runner: MetricRunner,
    client: insight_clickhouse::Client,
    table: String,
}

// Empty counts as unset: the CI matrix passes '' to entries without a
// provisioned ClickHouse, and set-but-empty must skip exactly like absent.
async fn stand_or_skip(schema: &str) -> Option<Stand> {
    let url = std::env::var(URL_VAR).unwrap_or_default();
    if url.is_empty() {
        eprintln!("skipping: {URL_VAR} not set");
        return None;
    }

    let database = std::env::var("INTEGRATION_TESTS_CLICKHOUSE_DATABASE").unwrap_or_default();
    let mut config = insight_clickhouse::Config::new(
        url,
        if database.is_empty() {
            "default".to_owned()
        } else {
            database
        },
    );
    let user = std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER").unwrap_or_default();
    let password = std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD").unwrap_or_default();
    // Half a credential would silently connect as nobody and read an empty
    // stand, which passes every assertion below for the wrong reason.
    assert_eq!(
        user.is_empty(),
        password.is_empty(),
        "set both INTEGRATION_TESTS_CLICKHOUSE_USER and _PASSWORD, or neither"
    );
    if !user.is_empty() {
        config = config.with_auth(user, password);
    }

    let client = insight_clickhouse::Client::new(config);
    let table = format!("window_{}", Uuid::now_v7().simple());
    execute(&client, &format!("CREATE TABLE {table} {schema}")).await;

    Some(Stand {
        runner: MetricRunner::new(
            insight_clickhouse::Client::new(client.config().clone()),
            People::new("identity"),
        ),
        client,
        table,
    })
}

impl Stand {
    async fn insert(&self, columns: &str, rows: &[&str]) {
        let values = rows.join(", ");
        execute(
            &self.client,
            &format!("INSERT INTO {} ({columns}) VALUES {values}", self.table),
        )
        .await;
    }

    fn metric(&self, body: &Value) -> MetricQuery {
        let mut body = body.clone();
        body["table"] = Value::String(self.table.clone());

        serde_json::from_value(body)
            .unwrap_or_else(|error| panic!("the fixture metric parses: {error}"))
    }

    /// The whole run: anchor, window, compiled SQL, rows.
    async fn answer(
        &self,
        metric: &MetricQuery,
        request: &WindowRequest,
        engine: TableEngine,
    ) -> (RunResult, Anchor) {
        let anchor = match metric
            .anchor_query(engine)
            .unwrap_or_else(|error| panic!("the anchor compiles: {error}"))
        {
            Some(query) => self
                .runner
                .anchor(&query)
                .await
                .unwrap_or_else(|error| panic!("the anchor reads: {error}")),
            None => Anchor::default(),
        };

        let window = request
            .resolve(Utc::now())
            .unwrap_or_else(|error| panic!("the window resolves: {error}"));
        let compiled = metric
            .compile_window(self.runner.people(), &window, engine)
            .unwrap_or_else(|error| panic!("the metric compiles: {error}"));
        let result = self
            .runner
            .run(&compiled)
            .await
            .unwrap_or_else(|error| panic!("the metric runs: {error}"));

        (result, anchor)
    }

    async fn drop_table(&self) {
        execute(
            &self.client,
            &format!("DROP TABLE IF EXISTS {}", self.table),
        )
        .await;
    }
}

async fn execute(client: &insight_clickhouse::Client, sql: &str) {
    client
        .query(sql)
        .execute()
        .await
        .unwrap_or_else(|error| panic!("`{sql}` runs: {error}"));
}

fn counted(metric: &Value) -> Value {
    let mut metric = metric.clone();
    metric["fields"] = json!([{ "agg": "count", "type": "int", "as_name": "total" }]);
    metric
}

fn request(range: &str, bucketed: Option<bool>) -> WindowRequest {
    WindowRequest::parse(Some(range), bucketed)
        .unwrap_or_else(|error| panic!("`{range}` parses: {error}"))
}

fn pairs(result: &RunResult) -> Vec<(String, String)> {
    result
        .rows
        .iter()
        .map(|row| {
            let cell = |index: usize| match row.get(index) {
                Some(Value::String(text)) => text.clone(),
                Some(value) => value.to_string(),
                None => String::new(),
            };

            (cell(0), cell(1))
        })
        .collect()
}

/// A `VALUES` row holding one instant, in the second precision a `DateTime`
/// column keeps.
fn stamp(instant: chrono::DateTime<Utc>) -> String {
    format!("('{}')", instant.format("%Y-%m-%d %H:%M:%S"))
}

fn totals(result: &RunResult) -> Vec<String> {
    result
        .rows
        .iter()
        .map(|row| match row.first() {
            Some(Value::String(text)) => text.clone(),
            Some(value) => value.to_string(),
            None => String::new(),
        })
        .collect()
}

#[tokio::test]
async fn a_day_bucket_holds_the_rows_of_its_own_day() {
    let Some(stand) =
        stand_or_skip("(occurred_at DateTime) ENGINE = MergeTree ORDER BY occurred_at").await
    else {
        return;
    };
    stand
        .insert(
            "occurred_at",
            &[
                "('2026-09-01 04:00:00')",
                "('2026-09-01 22:00:00')",
                "('2026-09-02 09:00:00')",
            ],
        )
        .await;

    let metric = stand.metric(&counted(&json!({ "time": { "column": "occurred_at" } })));
    let (result, _) = stand
        .answer(
            &metric,
            &request("2026-09-01/2026-09-03", None),
            TableEngine::MergeTree,
        )
        .await;

    assert_eq!(
        pairs(&result),
        vec![
            ("2026-09-01 00:00:00".to_owned(), "2".to_owned()),
            ("2026-09-02 00:00:00".to_owned(), "1".to_owned()),
        ]
    );

    stand.drop_table().await;
}

#[tokio::test]
async fn a_row_on_the_boundary_belongs_to_the_window_that_starts_there() {
    let Some(stand) =
        stand_or_skip("(occurred_at DateTime) ENGINE = MergeTree ORDER BY occurred_at").await
    else {
        return;
    };
    stand
        .insert(
            "occurred_at",
            &["('2026-09-01 00:00:00')", "('2026-09-02 00:00:00')"],
        )
        .await;

    let metric = stand.metric(&counted(&json!({ "time": { "column": "occurred_at" } })));
    let (first, _) = stand
        .answer(
            &metric,
            &request("2026-09-01/2026-09-02", Some(false)),
            TableEngine::MergeTree,
        )
        .await;
    let (second, _) = stand
        .answer(
            &metric,
            &request("2026-09-02/2026-09-03", Some(false)),
            TableEngine::MergeTree,
        )
        .await;

    assert_eq!(totals(&first), vec!["1".to_owned()]);
    assert_eq!(totals(&second), vec!["1".to_owned()]);

    stand.drop_table().await;
}

#[tokio::test]
async fn rows_carrying_no_clock_are_left_out_of_the_window_and_counted() {
    let Some(stand) =
        stand_or_skip("(occurred_at Nullable(DateTime)) ENGINE = MergeTree ORDER BY tuple()").await
    else {
        return;
    };
    stand
        .insert(
            "occurred_at",
            &[
                "('2026-09-01 10:00:00')",
                "('2026-09-02 10:00:00')",
                "(NULL)",
                "(NULL)",
            ],
        )
        .await;

    let metric = stand.metric(&counted(&json!({ "time": { "column": "occurred_at" } })));
    let (result, anchor) = stand
        .answer(
            &metric,
            &request("inf", Some(false)),
            TableEngine::MergeTree,
        )
        .await;

    assert_eq!(totals(&result), vec!["2".to_owned()]);
    assert_eq!(anchor.undated(), 2);

    stand.drop_table().await;
}

#[tokio::test]
async fn an_empty_source_answers_no_rows_rather_than_a_window_at_the_epoch() {
    let Some(stand) =
        stand_or_skip("(occurred_at DateTime) ENGINE = MergeTree ORDER BY occurred_at").await
    else {
        return;
    };

    let metric = stand.metric(&counted(&json!({ "time": { "column": "occurred_at" } })));
    let (result, _) = stand
        .answer(&metric, &request("P7D", None), TableEngine::MergeTree)
        .await;

    assert!(result.rows.is_empty(), "{:?}", result.rows);

    stand.drop_table().await;
}

#[tokio::test]
async fn a_replacing_table_counts_each_key_once() {
    let Some(stand) = stand_or_skip(
        "(id UInt64, occurred_at DateTime, version UInt64) \
         ENGINE = ReplacingMergeTree(version) ORDER BY id",
    )
    .await
    else {
        return;
    };
    // A part is deduplicated as it is written, so two versions of one key
    // must arrive in separate inserts and stay in separate parts.
    execute(
        &stand.client,
        &format!("SYSTEM STOP MERGES {}", stand.table),
    )
    .await;
    stand
        .insert(
            "id, occurred_at, version",
            &[
                "(1, '2026-09-01 10:00:00', 1)",
                "(2, '2026-09-01 11:00:00', 1)",
            ],
        )
        .await;
    stand
        .insert(
            "id, occurred_at, version",
            &["(1, '2026-09-01 10:00:00', 2)"],
        )
        .await;

    let metric = stand.metric(&counted(&json!({ "time": { "column": "occurred_at" } })));
    let (deduplicated, _) = stand
        .answer(
            &metric,
            &request("inf", Some(false)),
            TableEngine::ReplacingMergeTree,
        )
        .await;
    let (raw, _) = stand
        .answer(
            &metric,
            &request("inf", Some(false)),
            TableEngine::MergeTree,
        )
        .await;

    assert_eq!(totals(&deduplicated), vec!["2".to_owned()]);
    assert_eq!(totals(&raw), vec!["3".to_owned()]);

    stand.drop_table().await;
}

#[tokio::test]
async fn a_millisecond_short_of_the_end_is_still_inside_the_window() {
    let Some(stand) =
        stand_or_skip("(occurred_at DateTime64(3)) ENGINE = MergeTree ORDER BY occurred_at").await
    else {
        return;
    };
    stand
        .insert(
            "occurred_at",
            &["('2026-09-01 23:59:59.999')", "('2026-09-02 00:00:00.000')"],
        )
        .await;

    let metric = stand.metric(&counted(&json!({ "time": { "column": "occurred_at" } })));
    let (result, _) = stand
        .answer(
            &metric,
            &request("2026-09-01/2026-09-02", Some(false)),
            TableEngine::MergeTree,
        )
        .await;

    assert_eq!(totals(&result), vec!["1".to_owned()]);

    stand.drop_table().await;
}

#[tokio::test]
async fn a_clock_inside_an_ingested_payload_windows_the_same_way() {
    let Some(stand) = stand_or_skip("(raw_data String) ENGINE = MergeTree ORDER BY tuple()").await
    else {
        return;
    };
    stand
        .insert(
            "raw_data",
            &[
                "('{\"committed_at\": \"2026-09-01 08:00:00\"}')",
                "('{\"committed_at\": \"2026-09-02 08:00:00\"}')",
                "('{\"other\": 1}')",
            ],
        )
        .await;

    let metric = stand.metric(&counted(&json!({ "time": { "json": "committed_at" } })));
    let (result, anchor) = stand
        .answer(
            &metric,
            &request("2026-09-01/2026-09-02", Some(false)),
            TableEngine::MergeTree,
        )
        .await;

    assert_eq!(totals(&result), vec!["1".to_owned()]);
    assert_eq!(anchor.undated(), 1);

    stand.drop_table().await;
}

#[tokio::test]
async fn a_rolling_window_holds_what_happened_since_and_drops_what_did_not() {
    let Some(stand) =
        stand_or_skip("(occurred_at DateTime) ENGINE = MergeTree ORDER BY occurred_at").await
    else {
        return;
    };
    let now = Utc::now();
    let inside = stamp(now - chrono::TimeDelta::hours(1));
    let older = stamp(now - chrono::TimeDelta::days(20));
    stand
        .insert("occurred_at", &[inside.as_str(), older.as_str()])
        .await;

    let metric = stand.metric(&counted(&json!({ "time": { "column": "occurred_at" } })));
    let (result, _) = stand
        .answer(
            &metric,
            &request("P7D", Some(false)),
            TableEngine::MergeTree,
        )
        .await;

    // The window ends at the clock, so a row written an hour ago is inside it
    // and one from three weeks back is not.
    assert_eq!(totals(&result), vec!["1".to_owned()]);

    stand.drop_table().await;
}

#[tokio::test]
async fn a_window_bound_keeps_the_millisecond_it_was_cut_on() {
    let Some(stand) =
        stand_or_skip("(occurred_at DateTime64(3)) ENGINE = MergeTree ORDER BY occurred_at").await
    else {
        return;
    };
    stand
        .insert(
            "occurred_at",
            &[
                "('2026-09-01 00:00:00.000')",
                "('2026-09-01 00:00:00.500')",
                "('2026-09-08 00:00:00.000')",
            ],
        )
        .await;

    let metric = stand.metric(&counted(&json!({ "time": { "column": "occurred_at" } })));
    let (result, _) = stand
        .answer(
            &metric,
            &request("2026-09-01/2026-09-08", Some(false)),
            TableEngine::MergeTree,
        )
        .await;

    assert_eq!(totals(&result), vec!["2".to_owned()]);

    stand.drop_table().await;
}
