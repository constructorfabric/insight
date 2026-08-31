use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{Days, Months};
use futures::future::join_all;
use serde_json::json;
use uuid::Uuid;

use super::benchmark_support::{RssSampler, TempSampler, current_rss_bytes};
use super::dto::{ReportExportFormat, ReportGranularity, ReportSubject};
use super::executor::{ReportExecutionContext, execute_report};
use super::executor_test_fixtures::{date, people_recipe, profile};
use super::export::{ReportWriterLimits, start_report_writer};
use super::planner::{ReportPlannerLimits, plan_report};
use super::query::{ReportMetricQuery, ReportQueryError, ReportQueryRunner, ReportQuerySubject};
use super::row::{ReportMetricValue, ReportMetricValues};
use super::telemetry::ReportTelemetry;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "run explicitly to measure report generation capacity"]
async fn synthetic_report_generation_benchmark() {
    let settings = BenchmarkSettings {
        people: setting("REPORT_BENCH_PEOPLE", 1_000),
        periods: setting("REPORT_BENCH_PERIODS", 5),
        metrics: setting("REPORT_BENCH_METRICS", 64),
        concurrency: setting("REPORT_BENCH_CONCURRENCY", 1),
        max_batch_cells: setting("REPORT_BENCH_BATCH_CELLS", 100_000),
        max_total_cells: setting_u64("REPORT_BENCH_MAX_TOTAL_CELLS", 6_000_000),
        max_output_bytes: setting("REPORT_BENCH_MAX_BYTES", usize::MAX),
        max_xlsx_spool_bytes: setting("REPORT_BENCH_MAX_XLSX_SPOOL_BYTES", 90 * 1024 * 1024),
    };
    let formats = formats();

    for format in formats {
        let temp_root = benchmark_root(format);
        let temp_root_path = temp_root.path();
        let rss_before = current_rss_bytes();
        let rss = RssSampler::start();
        let temp = TempSampler::start(temp_root_path.to_path_buf());
        let started = Instant::now();
        let results = join_all(
            (0..settings.concurrency).map(|run| run_case(temp_root_path, run, settings, format)),
        )
        .await;
        let elapsed = started.elapsed();
        let peak_rss = rss.finish();
        let peak_temp_bytes = temp.finish();
        let artifacts = results
            .into_iter()
            .map(|result| result.unwrap_or_else(|error| panic!("benchmark failed: {error}")))
            .collect::<Vec<_>>();
        let output_bytes = artifacts
            .iter()
            .map(|artifact| artifact.output_bytes)
            .sum::<u64>();
        let planning_ms = artifacts
            .iter()
            .map(|value| value.planning_ms)
            .sum::<u128>();
        let execution_ms = artifacts
            .iter()
            .map(|value| value.execution_ms)
            .sum::<u128>();
        let writer_finalize_ms = artifacts
            .iter()
            .map(|value| value.writer_finalize_ms)
            .sum::<u128>();
        let rows = artifacts.iter().map(|artifact| artifact.rows).sum::<u64>();
        let cells = artifacts.iter().map(|artifact| artifact.cells).sum::<u64>();

        println!(
            "{}",
            json!({
                "benchmark": "analytics_report_generation",
                "format": format_name(format),
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
                "cleanup_verified": true,
            })
        );
    }
}

#[derive(Debug)]
struct ArtifactMeasurement {
    rows: u64,
    cells: u64,
    output_bytes: u64,
    planning_ms: u128,
    execution_ms: u128,
    writer_finalize_ms: u128,
}

#[derive(Debug, Clone, Copy)]
struct BenchmarkSettings {
    people: usize,
    periods: usize,
    metrics: usize,
    concurrency: usize,
    max_batch_cells: usize,
    max_total_cells: u64,
    max_output_bytes: usize,
    max_xlsx_spool_bytes: usize,
}

async fn run_case(
    temp_root: &std::path::Path,
    run: usize,
    settings: BenchmarkSettings,
    format: ReportExportFormat,
) -> Result<ArtifactMeasurement, String> {
    let profiles = (0..settings.people)
        .map(|index| {
            let id = u128::try_from(index + 1).map_err(|error| error.to_string())?;
            Ok(profile(id, &format!("Example Person {index}")))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut recipe = people_recipe(&profiles, settings.metrics);
    recipe.from = date("2025-01-01");
    recipe.to = recipe
        .from
        .checked_add_months(Months::new(
            u32::try_from(settings.periods).map_err(|error| error.to_string())?,
        ))
        .and_then(|date| date.checked_sub_days(Days::new(1)))
        .ok_or_else(|| "benchmark period range overflowed".to_owned())?;
    recipe.granularity = ReportGranularity::Month;
    let planning_started = Instant::now();
    let plan = plan_report(
        &recipe,
        &profiles,
        ReportPlannerLimits {
            max_batch_cells: settings.max_batch_cells,
            max_total_cells: settings.max_total_cells,
        },
    )
    .map_err(|error| error.to_string())?;
    let planning_ms = planning_started.elapsed().as_millis();
    let temp_dir = temp_root.join(run.to_string());
    let ids = profiles.iter().map(|profile| profile.person_id).collect();
    let telemetry = ReportTelemetry::new(&ReportSubject::People { ids }, format);
    let generation_permit = Arc::new(tokio::sync::Semaphore::new(1))
        .acquire_owned()
        .await
        .map_err(|error| error.to_string())?;
    let (sink, writer) = start_report_writer(
        format,
        &plan,
        temp_dir.clone(),
        ReportWriterLimits {
            max_generated_bytes: settings.max_output_bytes,
            max_xlsx_spool_bytes: settings.max_xlsx_spool_bytes,
            channel_batches: 1,
        },
        telemetry,
        generation_permit,
    )
    .map_err(|error| error.to_string())?;
    let runner = SyntheticRunner {
        periods: plan
            .periods
            .iter()
            .map(|period| period.bucket_start)
            .collect(),
    };

    let execution_started = Instant::now();
    execute_report(
        &recipe,
        &plan,
        &profiles,
        ReportExecutionContext {
            tenant_id: Uuid::from_u128(9),
            enforce_tenant_scope: true,
        },
        &runner,
        sink,
    )
    .await
    .map_err(|error| error.to_string())?;
    let execution_ms = execution_started.elapsed().as_millis();
    let writer_started = Instant::now();
    let artifact = writer
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let writer_finalize_ms = writer_started.elapsed().as_millis();
    let output_bytes = artifact.content_length();
    let (path, _) = artifact.disarm().map_err(|error| error.to_string())?;
    std::fs::remove_file(path).map_err(|error| error.to_string())?;
    let clean = std::fs::read_dir(&temp_dir)
        .map_err(|error| error.to_string())?
        .next()
        .is_none();
    if !clean {
        return Err("benchmark temporary directory was not empty".to_owned());
    }
    std::fs::remove_dir(temp_dir).map_err(|error| error.to_string())?;

    Ok(ArtifactMeasurement {
        rows: plan.size.total_rows,
        cells: plan.size.total_cells,
        output_bytes,
        planning_ms,
        execution_ms,
        writer_finalize_ms,
    })
}

struct SyntheticRunner {
    periods: Vec<chrono::NaiveDate>,
}

#[async_trait]
impl ReportQueryRunner for SyntheticRunner {
    async fn run(&self, query: ReportMetricQuery) -> Result<ReportMetricValues, ReportQueryError> {
        let ids = match query.subject {
            ReportQuerySubject::People(ids) => ids,
            ReportQuerySubject::Tenant(id) => vec![id],
        };
        let value = u32::try_from(query.metric_index).map_or(0.0, f64::from);
        let values = ids
            .into_iter()
            .flat_map(|entity_id| {
                self.periods
                    .iter()
                    .map(move |bucket_start| ReportMetricValue {
                        entity_id,
                        bucket_start: *bucket_start,
                        value: Some(value),
                    })
            })
            .collect();

        Ok(ReportMetricValues {
            metric_index: query.metric_index,
            values,
        })
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

fn formats() -> Vec<ReportExportFormat> {
    match std::env::var("REPORT_BENCH_FORMAT").as_deref() {
        Ok("csv") => vec![ReportExportFormat::Csv],
        Ok("xlsx") => vec![ReportExportFormat::Xlsx],
        Ok("both") | Err(_) => vec![ReportExportFormat::Csv, ReportExportFormat::Xlsx],
        Ok(value) => panic!("unsupported REPORT_BENCH_FORMAT: {value}"),
    }
}

const fn format_name(format: ReportExportFormat) -> &'static str {
    match format {
        ReportExportFormat::Csv => "csv",
        ReportExportFormat::Xlsx => "xlsx",
    }
}

fn benchmark_root(format: ReportExportFormat) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!(
            "analytics-report-benchmark-{}-",
            format_name(format)
        ))
        .tempdir()
        .unwrap_or_else(|error| panic!("benchmark root must create: {error}"))
}
