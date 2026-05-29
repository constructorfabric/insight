//! Application configuration.

use figment::Figment;
use figment::providers::{Env, Format, Yaml};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// HTTP bind address (e.g., `0.0.0.0:8081`).
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    /// `MariaDB` connection URL.
    /// Example: `mysql://insight:password@localhost:3306/analytics`
    pub database_url: String,

    /// `ClickHouse` HTTP URL (e.g., `http://localhost:8123`).
    pub clickhouse_url: String,

    /// `ClickHouse` database name (e.g., `insight`).
    #[serde(default = "default_clickhouse_database")]
    pub clickhouse_database: String,

    /// `ClickHouse` username. Optional — omit for no-auth deployments.
    #[serde(default)]
    pub clickhouse_user: Option<String>,

    /// `ClickHouse` password.
    #[serde(default)]
    pub clickhouse_password: Option<String>,

    /// Identity service base URL (e.g., `http://insight-identity:8082`).
    /// Optional — when empty, `person_ids` from `$filter` are used directly against
    /// `ClickHouse` without alias resolution (MVP mode).
    #[serde(default)]
    pub identity_url: String,

    /// Redis URL for caching (e.g., `redis://localhost:6379`). Backs
    /// `cpt-metric-cat-component-cache-layer`. Leave empty in single-replica
    /// dev installs — the cache layer degrades to a no-op stub. Multi-replica
    /// deploys MUST configure this; the cross-replica-invalidation NFR
    /// (`cpt-metric-cat-nfr-cross-replica-invalidation`) cannot be satisfied
    /// by purely in-process state.
    #[serde(default)]
    pub redis_url: String,

    /// Metric Catalog configuration (DESIGN §3.5).
    #[serde(default)]
    pub metric_catalog: MetricCatalogConfig,
}

/// Configuration consumed by `cpt-metric-cat-component-auth-trait` and the rest
/// of the catalog stack (DESIGN §3.5). Currently carries only the single-tenant
/// fallback per `cpt-metric-cat-constraint-tenant-default`; future catalog
/// knobs (cache TTL, etc.) land here too.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MetricCatalogConfig {
    /// Single-tenant fallback. When set, requests without a session-bound
    /// tenant resolve to this UUID; when unset (multi-tenant install), such
    /// requests are rejected with a canonical `invalid_argument` envelope
    /// carrying `field_violations[{field: "tenant_id", reason:
    /// "TENANT_UNRESOLVED"}]`. Mirrors `IDENTITY__identity__tenant_default_id`
    /// in the identity service so operators see the same single-tenant
    /// ergonomic across Insight services. The session-bound tenant ALWAYS
    /// wins over this default (security invariant — see
    /// `domain::auth::TenantAuthorization`).
    ///
    /// Env: `ANALYTICS__metric_catalog__tenant_default_id`.
    #[serde(default)]
    pub tenant_default_id: Option<Uuid>,
}

fn default_bind_addr() -> String {
    "0.0.0.0:8081".to_owned()
}

fn default_clickhouse_database() -> String {
    "insight".to_owned()
}

impl AppConfig {
    /// Load config: YAML file then environment variables (`ANALYTICS__*`).
    ///
    /// # Errors
    ///
    /// Returns error if config cannot be loaded or parsed.
    pub fn load(config_path: Option<&str>) -> anyhow::Result<Self> {
        let mut figment = Figment::new();

        if let Some(path) = config_path {
            figment = figment.merge(Yaml::file(path));
        }

        figment = figment.merge(Env::prefixed("ANALYTICS__").split("__"));

        let config: Self = figment.extract()?;
        Ok(config)
    }
}
