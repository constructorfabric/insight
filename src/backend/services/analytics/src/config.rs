//! Gear configuration.
//!
//! Loaded via `GearCtx::config::<GearConfig>()` (toolkit serde path), which
//! deserializes the YAML under `gears.analytics.config`. The figment
//! loader was removed in the gears-rust migration — the toolkit host owns
//! config layering (defaults -> YAML -> env -> CLI). Env overrides are
//! `APP__gears__analytics__config__<field>` (the prefix changed from the
//! old `ANALYTICS__*`).

use serde::Deserialize;
use uuid::Uuid;

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

    /// Identity service base URL (e.g., `http://insight-identity:8082`).
    /// Optional — when empty, `person_ids` from `$filter` are used directly against
    /// `ClickHouse` without alias resolution (MVP mode).
    pub identity_url: String,

    /// Redis URL for caching (e.g., `redis://localhost:6379`). Backs
    /// `cpt-metric-cat-component-cache-layer`. Leave empty in single-replica
    /// dev installs — the cache layer degrades to a no-op stub. Multi-replica
    /// deploys MUST configure this; the cross-replica-invalidation NFR
    /// (`cpt-metric-cat-nfr-cross-replica-invalidation`) cannot be satisfied
    /// by purely in-process state.
    pub redis_url: String,

    /// Metric Catalog configuration (DESIGN §3.5).
    pub metric_catalog: MetricCatalogConfig,
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
        }
    }
}

/// Configuration consumed by `cpt-metric-cat-component-auth-trait` and the rest
/// of the catalog stack (DESIGN §3.5). Currently carries only the single-tenant
/// fallback per `cpt-metric-cat-constraint-tenant-default`; future catalog
/// knobs (cache TTL, etc.) land here too.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MetricCatalogConfig {
    /// Single-tenant fallback. When set, requests without a session-bound
    /// tenant resolve to this UUID. Under the auth-disabled host the gateway
    /// always injects a tenant (`DEFAULT_TENANT_ID`), so this is primarily a
    /// catalog-resolution hint. The session-bound tenant ALWAYS wins over
    /// this default (security invariant — see `domain::auth::TenantAuthorization`).
    ///
    /// Env: `APP__gears__analytics__config__metric_catalog__tenant_default_id`.
    pub tenant_default_id: Option<Uuid>,
}

fn default_bind_addr() -> String {
    "0.0.0.0:8081".to_owned()
}

fn default_clickhouse_database() -> String {
    "insight".to_owned()
}
