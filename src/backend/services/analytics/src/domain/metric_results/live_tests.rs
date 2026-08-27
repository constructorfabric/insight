//! Live ClickHouse integration tests for the metric-results query path:
//! compiled SQL executed for real, rows parsed exactly as the handler parses
//! them (`JSONEachRow` lines into the typed query rows).
//!
//! `#[ignore]`d and skip silently when `INTEGRATION_TESTS_CLICKHOUSE_URL` is
//! unset (same convention as the `probe_live_tests` in `api/metrics.rs`);
//! optional auth via `INTEGRATION_TESTS_CLICKHOUSE_USER` /
//! `INTEGRATION_TESTS_CLICKHOUSE_PASSWORD`.

use chrono::NaiveDate;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use super::batch::{RankedDimension, RankedGroup, ResolvedGroupLimit};
use super::builder::build_timeseries_view;
use super::compiler::{CompiledQuery, TimeseriesQueryRow, compile_timeseries_query};
use super::dto::MetricResultViewDto;
use super::dto::MetricViewErrorCode;
use super::failure::ViewFailure;
use super::validation::{ValidatedEntitySelection, ValidatedMetricResultsRequest};
use super::view::Bucket;
use crate::domain::metric_definitions::definition::{
    ComputationSpec, CustomObservationSql, MetricBase, MetricDefinition, MetricDirection,
    MetricFormat, MetricInput, MetricInputRole, ObservationSource,
};

const URL_VAR: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";

const TENANT: Uuid = Uuid::from_u128(0x2581);
const PERSON: Uuid = Uuid::from_u128(0xfeed);

// Empty counts as unset: the CI matrix passes '' to entries without a
// provisioned ClickHouse, and set-but-empty must skip exactly like absent.
fn client_or_skip() -> Option<insight_clickhouse::Client> {
    let url = std::env::var(URL_VAR).unwrap_or_default();
    if url.is_empty() {
        eprintln!("skipping: {URL_VAR} not set");
        return None;
    }
    let mut config = insight_clickhouse::Config::new(url, "default");
    if let (Ok(user), Ok(password)) = (
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER"),
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD"),
    ) && !user.is_empty()
    {
        config = config.with_auth(user, password);
    }
    Some(insight_clickhouse::Client::new(config))
}

/// A self-contained observation source projecting the full contract, with
/// `metric_date` declared `Nullable(Date)` — the shape a custom metric's SQL
/// is free to produce. Four rows, values 1..=4, one person.
fn nullable_date_sql() -> String {
    format!(
        "SELECT \
            '{TENANT}' AS tenant_id, \
            'custom_repro' AS source_key, \
            'person' AS entity_type, \
            '{PERSON}' AS entity_id, \
            CAST(toDate('2026-08-13') + number AS Nullable(Date)) AS metric_date, \
            'repro_value' AS measure_key, \
            now() AS observed_at, \
            toFloat64(number + 1) AS value, \
            CAST(NULL AS Nullable(String)) AS subject_key, \
            CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS dimensions \
        FROM numbers(4)"
    )
}

fn nullable_date_metric() -> MetricDefinition {
    MetricDefinition {
        transform: None,
        base: MetricBase {
            key: "custom.nullable_date_repro".to_owned(),
            label: "Nullable date repro".to_owned(),
            short_label: None,
            description: None,
            explanation: None,
            entity_type: "person".to_owned(),
            format: MetricFormat::Integer,
            unit: None,
            direction: MetricDirection::HigherIsBetter,
            peer_cohort_key: None,
            allowed_dimensions: Vec::new(),
        },
        spec: ComputationSpec::Sum {
            value: MetricInput {
                role: MetricInputRole::Value,
                observation: ObservationSource::Custom(CustomObservationSql::new(
                    nullable_date_sql(),
                )),
                source_key: "custom_repro".to_owned(),
                measure_key: "repro_value".to_owned(),
            },
        },
    }
}

fn request() -> ValidatedMetricResultsRequest {
    ValidatedMetricResultsRequest {
        tenant_id: TENANT,
        entity: ValidatedEntitySelection::Person { ids: vec![PERSON] },
        from: NaiveDate::from_ymd_opt(2026, 8, 13).unwrap_or_default(),
        to: NaiveDate::from_ymd_opt(2026, 8, 16).unwrap_or_default(),
        metrics: Vec::new(),
        enforce_tenant_scope: true,
    }
}

/// The handler's row path: bind the compiled params, fetch `JSONEachRow`,
/// parse each line into the typed row (`api/metric_results.rs::fetch_rows`).
async fn fetch_rows<T: DeserializeOwned>(
    ch: &insight_clickhouse::Client,
    query: &CompiledQuery,
) -> anyhow::Result<Vec<T>> {
    let mut ch_query = ch.query(&query.sql);
    for param in &query.params {
        ch_query = ch_query.bind(param.as_str());
    }

    let raw_bytes = ch_query.fetch_bytes("JSONEachRow")?.collect().await?;
    raw_bytes
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).map_err(Into::into))
        .collect()
}

#[tokio::test]
#[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn timeseries_over_nullable_metric_date_builds_every_bucket() -> anyhow::Result<()> {
    let Some(ch) = client_or_skip() else {
        return Ok(());
    };
    let def = nullable_date_metric();
    let req = request();

    for bucket in [Bucket::Day, Bucket::Week, Bucket::Month] {
        let query = compile_timeseries_query(&def, &req, bucket, &[], &[], None);
        let rows: Vec<TimeseriesQueryRow> = fetch_rows(&ch, &query)
            .await
            .map_err(|e| anyhow::anyhow!("bucket {bucket:?}: {e}"))?;

        let view = build_timeseries_view(&def, &req, bucket, &[], rows)?;
        let MetricResultViewDto::Timeseries { series, .. } = view else {
            anyhow::bail!("bucket {bucket:?}: expected a timeseries view");
        };
        let [ref one] = series[..] else {
            anyhow::bail!(
                "bucket {bucket:?}: expected one series, got {}",
                series.len()
            );
        };
        anyhow::ensure!(
            one.total == Some(10.0),
            "bucket {bucket:?}: total must sum all four observations, got {:?}",
            one.total
        );
        let observed: f64 = one.points.iter().filter_map(|point| point.value).sum();
        anyhow::ensure!(
            (observed - 10.0).abs() < f64::EPSILON,
            "bucket {bucket:?}: bucketed points must carry the observations, got {observed}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn capped_timeseries_over_nullable_metric_date_builds() -> anyhow::Result<()> {
    let Some(ch) = client_or_skip() else {
        return Ok(());
    };
    let mut def = nullable_date_metric();
    def.base.allowed_dimensions = vec!["tool".to_owned()];
    let req = request();
    let dimensions = vec!["tool".to_owned()];
    let group_limit = ResolvedGroupLimit {
        groups: vec![RankedGroup {
            rank: 1,
            dimensions: vec![RankedDimension {
                value: "__unknown__".to_owned(),
                label: None,
            }],
        }],
        include_remainder: true,
    };

    let query = compile_timeseries_query(
        &def,
        &req,
        Bucket::Day,
        &dimensions,
        &[],
        Some(&group_limit),
    );
    let rows: Vec<TimeseriesQueryRow> = fetch_rows(&ch, &query).await?;

    let view = build_timeseries_view(&def, &req, Bucket::Day, &dimensions, rows)?;
    let MetricResultViewDto::Timeseries { series, .. } = view else {
        anyhow::bail!("expected a timeseries view");
    };
    let total: f64 = series.iter().filter_map(|s| s.total).sum();
    anyhow::ensure!(
        (total - 10.0).abs() < f64::EPSILON,
        "capped series totals must sum all four observations, got {total}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn a_real_missing_relation_error_classifies_as_source_relation_missing() -> anyhow::Result<()>
{
    let Some(ch) = client_or_skip() else {
        return Ok(());
    };

    let error = match ch
        .query("SELECT * FROM relation_absent_for_this_test")
        .fetch_bytes("JSONEachRow")
    {
        Err(e) => e.to_string(),
        Ok(mut cursor) => match cursor.collect().await {
            Err(e) => e.to_string(),
            Ok(_) => anyhow::bail!("the query over a missing relation must fail"),
        },
    };

    let failure = ViewFailure::from_query_error(&error);
    anyhow::ensure!(
        failure.code == MetricViewErrorCode::SourceRelationMissing,
        "the live UNKNOWN_TABLE wording must classify as a missing relation, got {failure:?}"
    );
    Ok(())
}
