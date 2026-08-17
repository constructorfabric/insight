//! Gear configuration.
//!
//! Loaded via `GearCtx::config::<GearConfig>()` (toolkit serde path), which
//! deserializes the YAML under `gears.analytics.config`. The figment
//! loader was removed in the gears-rust migration — the toolkit host owns
//! config layering (defaults -> YAML -> env -> CLI). Env overrides are
//! `APP__gears__analytics__config__<field>` (the prefix changed from the
//! old `ANALYTICS__*`).

use serde::Deserialize;

const DEFAULT_CACHE_TTL_SECS: u64 = 3600;

/// Configuration consumed by the analytics gear. Deserialized from
/// `gears.analytics.config`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GearConfig {
    /// HTTP bind address. Retained for compatibility/diagnostics — the
    /// `api-gateway` system gear owns the actual listener bind, so this is
    /// no longer consumed by the gear at runtime.
    pub bind_addr: String,

    /// `MariaDB` connection URL.
    /// Example: `mysql://insight:password@localhost:3306/analytics`
    pub database_url: String,

    /// `ClickHouse` HTTP URL (e.g., `http://localhost:8123`).
    pub clickhouse_url: String,

    /// `ClickHouse` database name (e.g., `insight`).
    pub clickhouse_database: String,

    /// `ClickHouse` username. Optional — omit for no-auth deployments.
    pub clickhouse_user: Option<String>,

    /// `ClickHouse` password.
    pub clickhouse_password: Option<String>,

    /// Identity service base URL (e.g., `http://insight-identity-resolution:8082`).
    /// Optional — when empty, `person_ids` from `$filter` are used directly against
    /// `ClickHouse` without alias resolution (MVP mode).
    pub identity_url: String,

    /// Redis URL (e.g., `redis://localhost:6379`). Empty disables every
    /// Redis-backed path; multi-replica deploys configure it so a cache added
    /// here is coordinated across replicas rather than per-process.
    ///
    /// This backs the metric-result view cache, whose key count grows with the
    /// variety of requests served and shrinks only as entries expire. Point it
    /// at an instance with an eviction policy, or at one dedicated to this
    /// service — an instance shared with session state and left on `noeviction`
    /// will start refusing writes once the cache fills it.
    pub redis_url: String,

    /// Metric read configuration.
    pub metric_catalog: MetricCatalogConfig,

    /// Metric-result view cache configuration.
    pub(crate) metric_results_cache: MetricResultsCacheConfig,
}

impl Default for GearConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_bind_addr(),
            database_url: String::new(),
            clickhouse_url: String::new(),
            clickhouse_database: default_clickhouse_database(),
            clickhouse_user: None,
            clickhouse_password: None,
            identity_url: String::new(),
            redis_url: String::new(),
            metric_catalog: MetricCatalogConfig::default(),
            metric_results_cache: MetricResultsCacheConfig::default(),
        }
    }
}

/// Per-environment knobs for the metric-result view cache. The cache is keyed
/// by the warehouse relation UUID, so a rebuild invalidates exactly; the TTL
/// only bounds how long superseded keys occupy Redis.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct MetricResultsCacheConfig {
    /// Lifetime of a cached view fragment.
    ///
    /// Env: `APP__gears__analytics__config__metric_results_cache__ttl_secs`.
    pub(crate) ttl_secs: u64,
}

impl Default for MetricResultsCacheConfig {
    fn default() -> Self {
        Self {
            ttl_secs: DEFAULT_CACHE_TTL_SECS,
        }
    }
}

/// Per-environment knobs for the metric read path.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MetricCatalogConfig {
    /// Enforce the per-tenant observation filter (#1967) on metric reads.
    /// Defaults to `false`: the ingested `tenant_id` in the bronze/silver/gold
    /// pipeline is not yet aligned to the JWT tenant, so an exact
    /// `tenant_id = <session>` match would silently empty every metric read.
    /// Flip to `true` per environment once the ingest tenant is aligned (#1829).
    ///
    /// Env: `APP__gears__analytics__config__metric_catalog__enforce_tenant_scope`.
    pub enforce_tenant_scope: bool,
}

fn default_bind_addr() -> String {
    "0.0.0.0:8081".to_owned()
}

fn default_clickhouse_database() -> String {
    "insight".to_owned()
}
