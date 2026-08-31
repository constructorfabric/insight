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

use super::batch::{
    BatchItem, PeriodWideRow, RankedDimension, RankedGroup, ResolvedGroupLimit, demux_period_rows,
};
use super::builder::{build_breakdown_view, build_period_view, build_timeseries_view};
use super::compiler::{
    BreakdownQueryRow, CompiledQuery, TimeseriesQueryRow, compile_breakdown_query,
    compile_period_batch_query, compile_timeseries_query,
};
use super::dto::MetricResultViewDto;
use super::dto::MetricViewErrorCode;
use super::failure::ViewFailure;
use super::validation::{DateWindow, ValidatedEntitySelection, ValidatedMetricResultsRequest};
use super::view::Bucket;
use crate::domain::external_links::ExternalSourceRegistry;
use crate::domain::metric_definitions::definition::ValueTransform;
use crate::domain::metric_definitions::definition::{
    AliasCollapse, ComputationSpec, CustomObservationSql, MetricBase, MetricDefinition,
    MetricDirection, MetricFormat, MetricInput, MetricInputRole, ObservationSource,
    RatioDenominatorAggregation,
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

/// Two dimension groups that do NOT overlap in time: `org/a` only in
/// 2026-08-15..16, `org/b` only in 2026-08-13..14. A standalone request over
/// either window returns exactly one of them — which is what the windowed
/// shape has to reproduce.
fn disjoint_groups_sql() -> String {
    let row = |repo: &str, day: &str, value: &str, measure: &str| {
        format!(
            "SELECT                 '{TENANT}' AS tenant_id,                 'custom_repro' AS source_key,                 'person' AS entity_type,                 '{PERSON}' AS entity_id,                 CAST(toDate('{day}') AS Nullable(Date)) AS metric_date,                 '{measure}' AS measure_key,                 now() AS observed_at,                 toFloat64({value}) AS value,                 CAST(NULL AS Nullable(String)) AS subject_key,                 CAST([('repository', '{repo}', NULL)] AS Array(Tuple(key String, value String, label Nullable(String)))) AS dimensions"
        )
    };
    [
        row("org/a", "2026-08-15", "10", "repro_value"),
        row("org/a", "2026-08-16", "0", "repro_denominator"),
        row("org/b", "2026-08-13", "20", "repro_value"),
        row("org/b", "2026-08-13", "4", "repro_denominator"),
    ]
    .join(" UNION ALL ")
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
                alias_collapse: AliasCollapse::Sum,
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
        compare_to: None,
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

/// The four observations of `nullable_date_sql` split across two windows:
/// 2026-08-13..14 carry 1 + 2, and 2026-08-15..16 carry 3 + 4. A windowed
/// request must read both out of ONE scan of the union.
#[tokio::test]
#[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn a_windowed_period_batch_answers_each_window_from_one_scan() -> anyhow::Result<()> {
    let Some(ch) = client_or_skip() else {
        return Ok(());
    };
    let mut req = request();
    req.from = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap_or_default();
    req.to = NaiveDate::from_ymd_opt(2026, 8, 16).unwrap_or_default();
    req.compare_to = Some(DateWindow {
        from: NaiveDate::from_ymd_opt(2026, 8, 13).unwrap_or_default(),
        to: NaiveDate::from_ymd_opt(2026, 8, 14).unwrap_or_default(),
    });

    // The second pass adds a transform, so the batch's projection stage has to
    // re-select every window's column and not just the primary one.
    for multiplier in [None, Some(10.0)] {
        let mut def = nullable_date_metric();
        def.transform = multiplier.map(|multiplier| ValueTransform {
            multiplier: Some(multiplier),
            offset: None,
            clamp_min: None,
            clamp_max: None,
        });
        let scale = multiplier.unwrap_or(1.0);

        let query = compile_period_batch_query(&[&def], &req, &[]);
        let rows: Vec<PeriodWideRow> = fetch_rows(&ch, &query)
            .await
            .map_err(|e| anyhow::anyhow!("multiplier {multiplier:?}: {e}"))?;
        let items = vec![BatchItem {
            metric_index: 0,
            view_index: 0,
            def: def.clone(),
        }];
        let per_item = demux_period_rows(&items, rows, req.compare_to.is_some())?;

        let MetricResultViewDto::Period { values } =
            build_period_view(&def, &req, per_item.into_iter().next().unwrap_or_default())
        else {
            anyhow::bail!("multiplier {multiplier:?}: expected a period view");
        };
        let [ref one] = values[..] else {
            anyhow::bail!(
                "multiplier {multiplier:?}: expected one entity, got {}",
                values.len()
            );
        };
        anyhow::ensure!(
            one.value == Some(7.0 * scale),
            "multiplier {multiplier:?}: the primary window must sum 3 + 4, got {:?}",
            one.value
        );
        anyhow::ensure!(
            one.compare_to == Some(3.0 * scale),
            "multiplier {multiplier:?}: the comparison window must sum 1 + 2, got {:?}",
            one.compare_to
        );
    }
    Ok(())
}

/// A ratio over the disjoint-group fixture: `org/a` lives only in the primary
/// window and its denominator is zero there, `org/b` only in the extra window.
/// This is the case a value column cannot express on its own.
fn disjoint_ratio_metric() -> MetricDefinition {
    let sql = disjoint_groups_sql();
    MetricDefinition {
        transform: None,
        base: MetricBase {
            key: "custom.disjoint_ratio".to_owned(),
            label: "Disjoint ratio".to_owned(),
            short_label: None,
            description: None,
            explanation: None,
            entity_type: "person".to_owned(),
            format: MetricFormat::Percent,
            unit: None,
            direction: MetricDirection::HigherIsBetter,
            peer_cohort_key: None,
            allowed_dimensions: vec!["repository".to_owned()],
        },
        spec: ComputationSpec::Ratio {
            numerator: MetricInput {
                role: MetricInputRole::Numerator,
                observation: ObservationSource::Custom(CustomObservationSql::new(sql.clone())),
                source_key: "custom_repro".to_owned(),
                measure_key: "repro_value".to_owned(),
                alias_collapse: AliasCollapse::Sum,
            },
            denominator: MetricInput {
                role: MetricInputRole::Denominator,
                observation: ObservationSource::Custom(CustomObservationSql::new(sql)),
                source_key: "custom_repro".to_owned(),
                measure_key: "repro_denominator".to_owned(),
                alias_collapse: AliasCollapse::Sum,
            },
            scale: 100.0,
            denominator_aggregation: RatioDenominatorAggregation::Sum,
        },
    }
}

/// The contract a projected window rests on: per group and per window, the
/// response has to say whether a standalone request over that window would
/// have returned the group at all — independently of its value.
#[tokio::test]
#[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn a_windowed_breakdown_reports_presence_apart_from_value() -> anyhow::Result<()> {
    let Some(ch) = client_or_skip() else {
        return Ok(());
    };
    let def = disjoint_ratio_metric();
    let mut req = request();
    req.from = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap_or_default();
    req.to = NaiveDate::from_ymd_opt(2026, 8, 16).unwrap_or_default();
    req.compare_to = Some(DateWindow {
        from: NaiveDate::from_ymd_opt(2026, 8, 13).unwrap_or_default(),
        to: NaiveDate::from_ymd_opt(2026, 8, 14).unwrap_or_default(),
    });
    let dimensions = vec!["repository".to_owned()];

    let query = compile_breakdown_query(&def, &req, &dimensions, &[]);
    let rows: Vec<BreakdownQueryRow> = fetch_rows(&ch, &query).await?;
    let view = build_breakdown_view(&req, &dimensions, rows, &ExternalSourceRegistry::default())?;
    let MetricResultViewDto::Breakdown { values, .. } = view else {
        anyhow::bail!("expected a breakdown view");
    };

    let group = |repo: &str| {
        values.iter().find(|value| {
            value
                .dimensions
                .iter()
                .any(|dimension| dimension.value == repo)
        })
    };
    let Some(a) = group("org/a") else {
        anyhow::bail!("org/a must be in the combined row set");
    };
    let Some(b) = group("org/b") else {
        anyhow::bail!("org/b must be in the combined row set");
    };

    // org/a IS in the primary window, and its ratio reads NULL there because
    // the denominator sums to zero. A reader dropping NULL values would lose a
    // row that a standalone request returns.
    anyhow::ensure!(
        a.present == Some(true) && a.value.is_none(),
        "org/a must be present in the primary window with a NULL value, got {:?}",
        (a.present, a.value)
    );
    let Some(ref a_window) = a.compare_to else {
        anyhow::bail!("expected a comparison window for org/a");
    };
    anyhow::ensure!(
        !a_window.present,
        "org/a has no rows in the comparison window, got present={}",
        a_window.present
    );

    // org/b is the mirror image: absent from the primary window, present in
    // the extra one with a real value.
    let Some(ref b_window) = b.compare_to else {
        anyhow::bail!("expected a comparison window for org/b");
    };
    anyhow::ensure!(
        b.present == Some(false),
        "org/b has no rows in the primary window, got {:?}",
        b.present
    );
    anyhow::ensure!(
        b_window.present && b_window.value == Some(500.0),
        "org/b must read 100 * 20 / 4 in the comparison window, got {:?}",
        (b_window.present, b_window.value)
    );
    Ok(())
}

/// The breakdown's window columns, through the transform projection stage that
/// re-selects every one of them by name.
#[tokio::test]
#[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn a_windowed_breakdown_transforms_every_window_column() -> anyhow::Result<()> {
    let Some(ch) = client_or_skip() else {
        return Ok(());
    };
    let mut def = nullable_date_metric();
    def.base.allowed_dimensions = vec!["tool".to_owned()];
    def.transform = Some(ValueTransform {
        multiplier: Some(10.0),
        offset: None,
        clamp_min: None,
        clamp_max: None,
    });
    let mut req = request();
    req.from = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap_or_default();
    req.to = NaiveDate::from_ymd_opt(2026, 8, 16).unwrap_or_default();
    req.compare_to = Some(DateWindow {
        from: NaiveDate::from_ymd_opt(2026, 8, 13).unwrap_or_default(),
        to: NaiveDate::from_ymd_opt(2026, 8, 14).unwrap_or_default(),
    });
    let dimensions = vec!["tool".to_owned()];

    let query = compile_breakdown_query(&def, &req, &dimensions, &[]);
    let rows: Vec<BreakdownQueryRow> = fetch_rows(&ch, &query).await?;

    let view = build_breakdown_view(&req, &dimensions, rows, &ExternalSourceRegistry::default())?;
    let MetricResultViewDto::Breakdown { values, .. } = view else {
        anyhow::bail!("expected a breakdown view");
    };
    let [ref one] = values[..] else {
        anyhow::bail!("expected one dimension group, got {}", values.len());
    };
    anyhow::ensure!(
        one.value == Some(70.0),
        "the primary window must be the transformed 3 + 4, got {:?}",
        one.value
    );
    let Some(ref window) = one.compare_to else {
        anyhow::bail!("expected a comparison window");
    };
    anyhow::ensure!(
        window.value == Some(30.0) && window.present,
        "the comparison window must be present and transformed, got {:?}",
        (window.value, window.present)
    );
    anyhow::ensure!(
        one.present == Some(true),
        "the group is observed in the primary window too, got {:?}",
        one.present
    );
    Ok(())
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
        let query = compile_timeseries_query(&def, &req, bucket.into(), &[], &[], None);
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
        Bucket::Day.into(),
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
