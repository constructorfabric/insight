//! Scheduled materialization of the semantic layer's measures. A read never
//! triggers a build; a tick builds each due measure's hot window into staging
//! and swaps it in one partition at a time. A build that fails leaves the
//! previous coverage and the previously served partitions exactly as they were.

mod error;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod live_tests;
mod lock;
mod plan;
mod policy;
pub mod read_gate;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod warehouse_live_tests;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::Utc;
use clickhouse::Row;
use sea_orm::DatabaseConnection;
use serde::Deserialize;

use crate::domain::compiler::cache_build::{
    CacheBuild, CacheRowKind, compile_cache_build, row_kind,
};
use crate::domain::compiler::sql::{CompiledMeasureQuery, QueryParam};
use crate::domain::definitions::definition::MeasureDefinition;
use crate::domain::definitions::seeds::product_definitions;
use crate::domain::field_catalog::model::FieldCatalog;
use crate::domain::field_catalog::product_catalog;

use lock::{LockOutcome, LockSession};
use plan::HotWindow;
use policy::CoverageWrite;

pub use error::CacheRefreshError;
pub use policy::seed_cache_policies;
pub use read_gate::{CacheDecision, ReadGate};

/// How often the loop looks for due measures. Each measure's own cadence comes
/// from its policy, so this only bounds how late a due measure starts.
const TICK: Duration = Duration::from_mins(1);

/// A build is background work over a whole dataset: it gets room to finish and
/// a thread budget that cannot starve the serving pool. No read-bytes ceiling —
/// scanning the dataset is the job, and a cap sized for a small install would
/// fail the build on a large one instead of bounding it.
const BUILD_TIMEOUT_SECS: u64 = 600;
const BUILD_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const BUILD_THREADS: u32 = 2;

#[derive(Debug, Row, Deserialize)]
struct CachePartition {
    version: u32,
    month: u32,
}

/// A shipped measure and the row shape its own metrics decide, resolved once so
/// no build has to guess one for the other.
struct CachedMeasure {
    definition: MeasureDefinition,
    kind: CacheRowKind,
}

/// Materializes measures on a schedule. One measure at a time per tick, so a
/// build never competes with the next one for the warehouse.
pub struct MeasureCacheRefresher {
    db: DatabaseConnection,
    locks: LockSession,
    ch: insight_clickhouse::Client,
    catalog: &'static FieldCatalog,
    measures: BTreeMap<String, CachedMeasure>,
    attempted: BTreeMap<String, Instant>,
}

impl MeasureCacheRefresher {
    /// # Errors
    ///
    /// Returns an error if the compiled-in product definitions do not load —
    /// an authoring failure rather than a runtime one — or if the pinned lock
    /// session cannot be opened.
    pub async fn new(
        db: DatabaseConnection,
        ch: insight_clickhouse::Client,
        database_url: &str,
    ) -> anyhow::Result<Self> {
        let definitions = product_definitions()?;
        let catalog = product_catalog().map_err(|error| anyhow::anyhow!("{error}"))?;
        let locks = LockSession::connect(database_url).await?;

        let measures = definitions
            .measures
            .iter()
            .map(|measure| {
                let kind = row_kind(measure, &definitions.metrics);
                (
                    measure.key.clone(),
                    CachedMeasure {
                        definition: measure.clone(),
                        kind,
                    },
                )
            })
            .collect();

        Ok(Self {
            db,
            locks,
            ch,
            catalog,
            measures,
            attempted: BTreeMap::new(),
        })
    }

    /// Periodic sweep. Never returns; run it on a spawned task.
    pub async fn run(mut self) {
        let mut ticks = tokio::time::interval(TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            self.refresh_due().await;
        }
    }

    async fn refresh_due(&mut self) {
        let policies = match policy::enabled_policies(&self.db).await {
            Ok(policies) => policies,
            Err(error) => {
                tracing::warn!(%error, "measure cache policies are unreadable; nothing refreshed");
                return;
            }
        };
        let versions = match policy::current_versions(&self.db).await {
            Ok(versions) => versions,
            Err(error) => {
                tracing::warn!(%error, "measure definition versions are unreadable; nothing refreshed");
                return;
            }
        };

        let today = Utc::now().date_naive();
        for policy in policies {
            let due = Duration::from_secs(u64::from(policy.refresh_interval_minutes) * 60);
            if self
                .attempted
                .get(&policy.measure_key)
                .is_some_and(|last| last.elapsed() < due)
            {
                continue;
            }
            let Some(&version) = versions.get(&policy.measure_key) else {
                continue;
            };
            match self.locks.acquire(&policy.measure_key).await {
                LockOutcome::Held => {}
                LockOutcome::HeldElsewhere => {
                    tracing::debug!(
                        measure = %policy.measure_key,
                        "another replica is building this measure; skipping the tick"
                    );
                    continue;
                }
                LockOutcome::Unknown => continue,
            }

            self.attempted
                .insert(policy.measure_key.clone(), Instant::now());
            let window = plan::hot_window(today, policy.hot_window_days);

            let outcome = self.refresh(&policy.measure_key, version, window).await;
            self.locks.release(&policy.measure_key).await;

            if let Err(error) = outcome {
                tracing::error!(
                    %error,
                    measure = %policy.measure_key,
                    definition_version = version,
                    "measure cache refresh failed; the previous coverage keeps serving"
                );
            }
        }
    }

    async fn refresh(
        &self,
        measure_key: &str,
        version: u32,
        window: HotWindow,
    ) -> Result<(), CacheRefreshError> {
        let Some(cached) = self.measures.get(measure_key) else {
            return Ok(());
        };
        let measure = &cached.definition;
        let dataset = self.catalog.dataset(&measure.dataset).ok_or_else(|| {
            CacheRefreshError::UncataloguedDataset {
                measure: measure.key.clone(),
                dataset: measure.dataset.clone(),
            }
        })?;

        let build = CacheBuild {
            measure,
            definition_version: version,
            kind: cached.kind,
            from: window.from,
            to: window.to,
        };
        let compiled = compile_cache_build(dataset, &build).map_err(|source| {
            CacheRefreshError::Uncompilable {
                measure: measure.key.clone(),
                source,
            }
        })?;
        let months = plan::months(window);

        self.clear_staging(measure_key, version, &months).await?;
        self.execute(measure_key, &compiled).await?;

        // INVARIANT: a shape change at one version cannot be healed by widening
        // coverage — a settled month would keep rows of the old shape under a
        // window claiming the new one. The drop follows the staged build, so a
        // build the warehouse refuses drops nothing.
        let write = if self.reshaped(measure_key, version, cached.kind).await? {
            self.drop_version(measure_key, version).await?;
            CoverageWrite::Replace
        } else {
            CoverageWrite::Widen
        };

        for month in &months {
            self.alter_partition(measure_key, &plan::swap_partition_sql(), version, *month)
                .await?;
        }
        self.clear_staging(measure_key, version, &months).await?;

        policy::record_coverage(
            &self.db,
            measure_key,
            version,
            cached.kind,
            window.from,
            window.to,
            write,
        )
        .await?;
        self.drop_superseded(measure_key, version).await;
        Ok(())
    }

    /// Whether what the coverage attests was built at a different row shape
    /// than the one this release folds the measure into.
    async fn reshaped(
        &self,
        measure_key: &str,
        version: u32,
        kind: CacheRowKind,
    ) -> Result<bool, CacheRefreshError> {
        let attested = policy::coverage_row_kind(&self.db, measure_key, version).await?;
        Ok(attested.is_some_and(|attested| attested != kind))
    }

    /// Every partition one version occupies, dropped before its rebuild so the
    /// months this run does not touch cannot keep the shape it is replacing.
    async fn drop_version(&self, measure_key: &str, version: u32) -> Result<(), CacheRefreshError> {
        let occupied = self
            .bounded(&plan::version_partitions_sql())
            .bind(measure_key)
            .bind(version)
            .fetch_all::<CachePartition>()
            .await
            .map_err(|error| {
                tracing::error!(%error, measure = %measure_key, version, "the partitions of a reshaped measure are unreadable");
                CacheRefreshError::BuildRefused {
                    measure: measure_key.to_owned(),
                }
            })?;

        let sql = plan::drop_cache_partition_sql();
        for partition in occupied {
            self.alter_partition(measure_key, &sql, partition.version, partition.month)
                .await?;
        }
        Ok(())
    }

    async fn clear_staging(
        &self,
        measure_key: &str,
        version: u32,
        months: &[u32],
    ) -> Result<(), CacheRefreshError> {
        let sql = plan::clear_staging_partition_sql();
        for month in months {
            self.alter_partition(measure_key, &sql, version, *month)
                .await?;
        }
        Ok(())
    }

    /// Reclamation, not correctness: a superseded version's rows are dropped
    /// after the current version has landed, and a drop that does not happen is
    /// disk this run failed to release, never an answer this run got wrong.
    async fn drop_superseded(&self, measure_key: &str, version: u32) {
        let stale = self
            .bounded(&plan::superseded_partitions_sql())
            .bind(measure_key)
            .bind(version)
            .fetch_all::<CachePartition>()
            .await;
        let stale = match stale {
            Ok(stale) => stale,
            Err(error) => {
                tracing::warn!(%error, measure = %measure_key, "superseded cache partitions are unreadable");
                return;
            }
        };

        let sql = plan::drop_cache_partition_sql();
        for partition in stale {
            if let Err(error) = self
                .alter_partition(measure_key, &sql, partition.version, partition.month)
                .await
            {
                tracing::warn!(%error, measure = %measure_key, "a superseded cache partition was not dropped");
            }
        }
    }

    async fn alter_partition(
        &self,
        measure_key: &str,
        sql: &str,
        version: u32,
        month: u32,
    ) -> Result<(), CacheRefreshError> {
        self.bounded(sql)
            .bind(measure_key)
            .bind(version)
            .bind(month)
            .execute()
            .await
            .map_err(|error| {
                tracing::error!(%error, measure = %measure_key, version, month, sql, "cache partition statement failed");
                CacheRefreshError::BuildRefused {
                    measure: measure_key.to_owned(),
                }
            })
    }

    async fn execute(
        &self,
        measure_key: &str,
        compiled: &CompiledMeasureQuery,
    ) -> Result<(), CacheRefreshError> {
        let mut request = self.bounded(&compiled.sql);
        for param in &compiled.params {
            request = match param {
                QueryParam::Text(value) => request.bind(value.as_str()),
                QueryParam::Int(value) => request.bind(*value),
                QueryParam::UInt(value) => request.bind(*value),
                QueryParam::Float(value) => request.bind(*value),
                QueryParam::Bool(value) => request.bind(*value),
            };
        }

        request.execute().await.map_err(|error| {
            tracing::error!(%error, measure = %measure_key, sql = %compiled.sql, "measure cache build failed");
            CacheRefreshError::BuildRefused {
                measure: measure_key.to_owned(),
            }
        })
    }

    fn bounded(&self, sql: &str) -> clickhouse::query::Query {
        self.ch
            .query(sql)
            .with_setting("max_execution_time", BUILD_TIMEOUT_SECS.to_string())
            .with_setting("max_memory_usage", BUILD_MEMORY_BYTES.to_string())
            .with_setting("max_threads", BUILD_THREADS.to_string())
            .with_setting("log_comment", "semantic-measure-cache-refresh")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::domain::definitions::definition::Computation;

    /// Points at a closed port: a statement that reached the network would fail.
    fn offline_clickhouse() -> insight_clickhouse::Client {
        insight_clickhouse::Client::new(insight_clickhouse::Config {
            url: "http://127.0.0.1:1".to_owned(),
            database: "insight".to_owned(),
            user: None,
            password: None,
            query_timeout: None,
            query_max_threads: None,
            query_max_memory_bytes: None,
        })
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    fn offline_shipped_refresher() -> MeasureCacheRefresher {
        let definitions = product_definitions().expect("definitions are valid");

        offline_refresher(
            definitions
                .measures
                .iter()
                .map(|measure| {
                    let kind = row_kind(measure, &definitions.metrics);
                    (
                        measure.key.clone(),
                        CachedMeasure {
                            definition: measure.clone(),
                            kind,
                        },
                    )
                })
                .collect(),
        )
    }

    fn offline_refresher(measures: BTreeMap<String, CachedMeasure>) -> MeasureCacheRefresher {
        MeasureCacheRefresher {
            db: DatabaseConnection::default(),
            locks: LockSession::disconnected(),
            ch: offline_clickhouse(),
            catalog: product_catalog().expect("catalog loads"),
            measures,
            attempted: BTreeMap::new(),
        }
    }

    #[test]
    fn every_shipped_measure_is_given_the_row_shape_its_metrics_can_be_answered_from() {
        let definitions = product_definitions().expect("definitions are valid");

        for metric in &definitions.metrics {
            let read = match &metric.computation {
                Computation::Percentile { measure, .. } | Computation::Stddev { measure } => {
                    measure
                }
                Computation::Direct { .. }
                | Computation::Ratio { .. }
                | Computation::Derived { .. } => continue,
            };
            let measure = definitions
                .measures
                .iter()
                .find(|candidate| &candidate.key == read)
                .expect("a shipped metric reads a shipped measure");

            assert_eq!(
                row_kind(measure, &definitions.metrics),
                CacheRowKind::Event,
                "metric `{}` takes a distribution over `{}`",
                metric.key,
                measure.key
            );
        }
    }

    #[test]
    fn every_shipped_average_keeps_its_rows_because_an_average_cannot_re_fold() {
        use crate::domain::definitions::definition::Aggregation;

        let definitions = product_definitions().expect("definitions are valid");

        for measure in &definitions.measures {
            if measure.aggregation != Aggregation::Avg {
                continue;
            }
            assert_eq!(
                row_kind(measure, &definitions.metrics),
                CacheRowKind::Event,
                "measure `{}`",
                measure.key
            );
        }
    }

    #[test]
    fn every_shipped_measure_compiles_a_build_over_its_hot_window() {
        let definitions = product_definitions().expect("definitions are valid");
        let catalog = product_catalog().expect("catalog loads");
        let window = plan::hot_window(date(2026, 3, 10), 35);

        for measure in &definitions.measures {
            let dataset = catalog
                .dataset(&measure.dataset)
                .expect("a shipped measure reads a catalogued dataset");
            let build = CacheBuild {
                measure,
                definition_version: 1,
                kind: row_kind(measure, &definitions.metrics),
                from: window.from,
                to: window.to,
            };

            let compiled = compile_cache_build(dataset, &build)
                .unwrap_or_else(|error| panic!("measure `{}`: {error}", measure.key));

            assert_eq!(
                compiled.sql.matches('?').count(),
                compiled.params.len(),
                "measure `{}`",
                measure.key
            );
        }
    }

    #[tokio::test]
    async fn a_measure_this_release_does_not_ship_is_skipped_rather_than_failed() {
        let refresher = offline_refresher(BTreeMap::new());

        let outcome = refresher
            .refresh(
                "retired_measure",
                1,
                plan::hot_window(date(2026, 3, 10), 35),
            )
            .await;

        assert!(outcome.is_ok());
    }

    /// The offline store cannot be reached at all, so a `BuildRefused` proves
    /// the run stopped at the warehouse before the reshape read, the partition
    /// drop and the coverage write.
    #[tokio::test]
    async fn a_build_the_warehouse_refuses_drops_nothing_and_writes_no_coverage() {
        let refresher = offline_shipped_refresher();

        let outcome = refresher
            .refresh("commits", 1, plan::hot_window(date(2026, 3, 10), 35))
            .await;

        assert!(matches!(
            outcome.expect_err("a closed port cannot take a build"),
            CacheRefreshError::BuildRefused { measure } if measure == "commits"
        ));
    }
}
