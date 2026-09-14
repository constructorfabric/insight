use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use chrono::{Days, Months};
use futures::future::join_all;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::benchmark_support::{RssSampler, TempSampler, current_rss_bytes};
use super::dto::{ReportExportFormat, ReportGranularity, ReportSubject};
use super::executor::{ReportExecutionContext, execute_report};
use super::executor_test_fixtures::{date, people_recipe, profile};
use super::export::{ReportWriterLimits, start_report_writer};
use super::planner::{ReportPlannerLimits, plan_report};
use super::query::ClickHouseReportQueryRunner;
use super::telemetry::ReportTelemetry;
use crate::domain::metric_definitions::definition::{
    AliasCollapse, ComputationSpec, CustomObservationSql, MetricFormat, MetricInput,
    MetricInputRole, ObservationSource, RatioDenominatorAggregation,
};
use crate::infra::query::fetch_json_rows;

const URL_VAR: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";
const TENANT_ID: Uuid = Uuid::from_u128(9);

#[derive(Debug, Clone, Copy)]
struct Settings {
    people: usize,
    periods: usize,
    metrics: usize,
    concurrency: usize,
    max_batch_cells: usize,
    max_total_cells: u64,
    max_output_bytes: usize,
    max_xlsx_spool_bytes: usize,
    format: ReportExportFormat,
}

#[derive(Debug)]
struct Measurement {
    rows: u64,
    cells: u64,
    output_bytes: u64,
    planning_ms: u128,
    execution_ms: u128,
    writer_finalize_ms: u128,
}

#[derive(Debug, Deserialize)]
struct QueryLogSummary {
    #[serde(deserialize_with = "wire_u64")]
    queries: u64,
    #[serde(deserialize_with = "wire_u64")]
    read_rows: u64,
    #[serde(deserialize_with = "wire_u64")]
    written_rows: u64,
    #[serde(deserialize_with = "wire_u64")]
    peak_query_memory_bytes: u64,
    #[serde(deserialize_with = "wire_u64")]
    total_query_duration_ms: u64,
    #[serde(deserialize_with = "wire_u64")]
    max_query_duration_ms: u64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WireU64 {
    Number(u64),
    String(String),
}

fn wire_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match WireU64::deserialize(deserializer)? {
        WireU64::Number(value) => Ok(value),
        WireU64::String(value) => value.parse().map_err(serde::de::Error::custom),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires isolated ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn clickhouse_report_generation_benchmark() -> anyhow::Result<()> {
    let Some(client) = client_or_skip() else {
        return Ok(());
    };
    let settings = settings();
    let run_key = Uuid::new_v4().simple().to_string();
    let log_prefix = format!("report:timeseries:benchmark.{run_key}.");
    let temp_root = benchmark_root();
    let temp_root_path = temp_root.path();
    let generation_capacity = Arc::new(tokio::sync::Semaphore::new(settings.concurrency));
    let rss_before = current_rss_bytes();
    let rss = RssSampler::start();
    let temp = TempSampler::start(temp_root_path.to_path_buf());
    let started = Instant::now();

    let results = join_all((0..settings.concurrency).map(|run| {
        run_case(
            &client,
            temp_root_path,
            &run_key,
            run,
            settings,
            Arc::clone(&generation_capacity),
        )
    }))
    .await;
    let elapsed = started.elapsed();
    let peak_rss = rss.finish();
    let peak_temp_bytes = temp.finish();
    let measurements = results.into_iter().collect::<anyhow::Result<Vec<_>>>()?;

    client.query("SYSTEM FLUSH LOGS").execute().await?;
    let query_summary = query_summary(&client, &log_prefix).await?;
    let rows = measurements.iter().map(|value| value.rows).sum::<u64>();
    let cells = measurements.iter().map(|value| value.cells).sum::<u64>();
    let output_bytes = measurements
        .iter()
        .map(|value| value.output_bytes)
        .sum::<u64>();
    let planning_ms = measurements
        .iter()
        .map(|value| value.planning_ms)
        .sum::<u128>();
    let execution_ms = measurements
        .iter()
        .map(|value| value.execution_ms)
        .sum::<u128>();
    let writer_finalize_ms = measurements
        .iter()
        .map(|value| value.writer_finalize_ms)
        .sum::<u128>();

    println!(
        "{}",
        json!({
            "benchmark": "analytics_report_generation_clickhouse",
            "format": format_name(settings.format),
            "people_per_report": settings.people,
            "periods_per_report": settings.periods,
            "metrics_per_report": settings.metrics,
            "concurrency": settings.concurrency,
            "rows": rows,
            "cells": cells,
            "output_bytes": output_bytes,
            "peak_temp_bytes": peak_temp_bytes,
            "elapsed_ms": elapsed.as_millis(),
            "planning_ms": planning_ms,
            "execution_ms": execution_ms,
            "writer_finalize_ms": writer_finalize_ms,
            "rss_before_bytes": rss_before,
            "peak_rss_bytes": peak_rss,
            "peak_rss_growth_bytes": peak_rss.saturating_sub(rss_before),
            "clickhouse_queries": query_summary.queries,
            "clickhouse_read_rows": query_summary.read_rows,
            "clickhouse_written_rows": query_summary.written_rows,
            "clickhouse_peak_query_memory_bytes": query_summary.peak_query_memory_bytes,
            "clickhouse_total_query_duration_ms": query_summary.total_query_duration_ms,
            "clickhouse_max_query_duration_ms": query_summary.max_query_duration_ms,
            "cleanup_verified": true,
        })
    );
    Ok(())
}

async fn run_case(
    client: &insight_clickhouse::Client,
    temp_root: &Path,
    run_key: &str,
    run: usize,
    settings: Settings,
    generation_capacity: Arc<tokio::sync::Semaphore>,
) -> anyhow::Result<Measurement> {
    let profiles = (0..settings.people)
        .map(|index| profile((index + 1) as u128, &format!("Example Person {index}")))
        .collect::<Vec<_>>();
    let mut recipe = people_recipe(&profiles, settings.metrics);
    recipe.from = date("2025-01-01");
    recipe.to = recipe
        .from
        .checked_add_months(Months::new(u32::try_from(settings.periods)?))
        .and_then(|value| value.checked_sub_days(Days::new(1)))
        .ok_or_else(|| anyhow::anyhow!("benchmark period range overflowed"))?;
    recipe.granularity = ReportGranularity::Month;
    apply_synthetic_observations(&mut recipe, run_key, settings);
    let planning_started = Instant::now();
    let plan = plan_report(
        &recipe,
        &profiles,
        ReportPlannerLimits {
            max_batch_cells: settings.max_batch_cells,
            max_total_cells: settings.max_total_cells,
        },
    )?;
    let planning_ms = planning_started.elapsed().as_millis();
    let ids = profiles.iter().map(|value| value.person_id).collect();
    let telemetry = ReportTelemetry::new(&ReportSubject::People { ids }, settings.format);
    let generation_permit = generation_capacity.acquire_owned().await?;
    let temp_dir = temp_root.join(run.to_string());
    let (sink, writer) = start_report_writer(
        settings.format,
        &plan,
        temp_dir.clone(),
        ReportWriterLimits {
            max_generated_bytes: settings.max_output_bytes,
            max_xlsx_spool_bytes: settings.max_xlsx_spool_bytes,
            channel_batches: 1,
        },
        telemetry,
        generation_permit,
    )?;
    let runner = ClickHouseReportQueryRunner::new(client);

    let execution_started = Instant::now();
    execute_report(
        &recipe,
        &plan,
        &profiles,
        ReportExecutionContext {
            tenant_id: TENANT_ID,
            enforce_tenant_scope: true,
        },
        &runner,
        sink,
    )
    .await?;
    let execution_ms = execution_started.elapsed().as_millis();
    let writer_started = Instant::now();
    let artifact = writer.await??;
    let writer_finalize_ms = writer_started.elapsed().as_millis();
    let output_bytes = artifact.content_length();
    let (path, _) = artifact.disarm()?;
    std::fs::remove_file(path)?;
    anyhow::ensure!(
        std::fs::read_dir(&temp_dir)?.next().is_none(),
        "benchmark temporary directory was not empty"
    );
    std::fs::remove_dir(temp_dir)?;

    Ok(Measurement {
        rows: plan.size.total_rows,
        cells: plan.size.total_cells,
        output_bytes,
        planning_ms,
        execution_ms,
        writer_finalize_ms,
    })
}

fn apply_synthetic_observations(
    recipe: &mut super::validation::ValidatedReportRecipe,
    run_key: &str,
    settings: Settings,
) {
    let sql = synthetic_observation_sql(settings.people, settings.periods);
    for (index, metric) in recipe.metrics.iter_mut().enumerate() {
        metric.base.key = format!("benchmark.{run_key}.{index}");
        metric.base.format = MetricFormat::Decimal;
        let value = MetricInput {
            role: MetricInputRole::Value,
            observation: ObservationSource::Custom(CustomObservationSql::new(sql.clone())),
            source_key: "benchmark".to_owned(),
            measure_key: "value".to_owned(),
            alias_collapse: AliasCollapse::Sum,
        };
        metric.spec = match index % 6 {
            0 => ComputationSpec::Sum { value },
            1 => ComputationSpec::Ratio {
                numerator: value.clone(),
                denominator: value,
                scale: 100.0,
                denominator_aggregation: RatioDenominatorAggregation::Sum,
            },
            2 => ComputationSpec::Median { value },
            3 => ComputationSpec::Percentile { value, q: 0.75 },
            4 => ComputationSpec::Stddev { value },
            5 => ComputationSpec::DistinctCount { value },
            _ => unreachable!(),
        };
    }
}

fn synthetic_observation_sql(people: usize, periods: usize) -> String {
    let rows = people.saturating_mul(periods);
    format!(
        "SELECT \
         '{TENANT_ID}' AS tenant_id, \
         'benchmark' AS source_key, \
         'person' AS entity_type, \
         concat('00000000-0000-0000-0000-', leftPad(toString(intDiv(number, {periods}) + 1), 12, '0')) AS entity_id, \
         addMonths(toDate('2025-01-01'), modulo(number, {periods})) AS metric_date, \
         'value' AS measure_key, \
         now() AS observed_at, \
         toFloat64(modulo(number, 100) + 1) AS value, \
         CAST(toString(number) AS Nullable(String)) AS subject_key, \
         CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS dimensions \
         FROM numbers({rows})"
    )
}

async fn query_summary(
    client: &insight_clickhouse::Client,
    log_prefix: &str,
) -> anyhow::Result<QueryLogSummary> {
    let rows = fetch_json_rows::<QueryLogSummary>(
        client,
        "SELECT \
         count() AS queries, \
         sum(read_rows) AS read_rows, \
         sum(written_rows) AS written_rows, \
         max(memory_usage) AS peak_query_memory_bytes, \
         sum(query_duration_ms) AS total_query_duration_ms, \
         max(query_duration_ms) AS max_query_duration_ms \
         FROM system.query_log \
         WHERE type = 'QueryFinish' AND startsWith(log_comment, ?)",
        &[log_prefix.to_owned()],
        crate::infra::metrics::QueryKind::Report,
        "benchmark:report-query-summary",
    )
    .await?;

    rows.into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("ClickHouse query log summary was empty"))
}

fn client_or_skip() -> Option<insight_clickhouse::Client> {
    let url = std::env::var(URL_VAR).unwrap_or_default();
    if url.is_empty() {
        eprintln!("skipping: {URL_VAR} not set");
        return None;
    }
    let mut config = insight_clickhouse::Config::new(url, "default")
        .with_query_max_threads(4)
        .with_query_max_memory_bytes(1_610_612_736);
    if let (Ok(user), Ok(password)) = (
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER"),
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD"),
    ) && !user.is_empty()
    {
        config = config.with_auth(user, password);
    }
    Some(insight_clickhouse::Client::new(config))
}

fn settings() -> Settings {
    Settings {
        people: setting("REPORT_BENCH_PEOPLE", 1_000),
        periods: setting("REPORT_BENCH_PERIODS", 5),
        metrics: setting("REPORT_BENCH_METRICS", 64),
        concurrency: setting("REPORT_BENCH_CONCURRENCY", 2),
        max_batch_cells: setting("REPORT_BENCH_BATCH_CELLS", 100_000),
        max_total_cells: setting_u64("REPORT_BENCH_MAX_TOTAL_CELLS", 6_000_000),
        max_output_bytes: setting("REPORT_BENCH_MAX_BYTES", 25 * 1024 * 1024),
        max_xlsx_spool_bytes: setting("REPORT_BENCH_MAX_XLSX_SPOOL_BYTES", 90 * 1024 * 1024),
        format: match std::env::var("REPORT_BENCH_FORMAT").as_deref() {
            Ok("csv") => ReportExportFormat::Csv,
            Ok("xlsx") | Err(_) => ReportExportFormat::Xlsx,
            Ok(value) => panic!("unsupported REPORT_BENCH_FORMAT: {value}"),
        },
    }
}

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn setting_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

const fn format_name(format: ReportExportFormat) -> &'static str {
    match format {
        ReportExportFormat::Csv => "csv",
        ReportExportFormat::Xlsx => "xlsx",
    }
}

fn benchmark_root() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("analytics-report-clickhouse-")
        .tempdir()
        .unwrap_or_else(|error| panic!("benchmark root must create: {error}"))
}
