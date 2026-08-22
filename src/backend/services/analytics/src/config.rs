//! Gear configuration.
//!
//! Loaded via `GearCtx::config::<GearConfig>()` (toolkit serde path), which
//! deserializes the YAML under `gears.analytics.config`. The figment
//! loader was removed in the gears-rust migration — the toolkit host owns
//! config layering (defaults -> YAML -> env -> CLI). Env overrides are
//! `APP__gears__analytics__config__<field>` (the prefix changed from the
//! old `ANALYTICS__*`).

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityPolicy {
    #[default]
    OrgChart,
    Flat,
}

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

    /// Deployment-wide person visibility and peer-population policy.
    pub visibility_policy: VisibilityPolicy,

    /// Redis URL (e.g., `redis://localhost:6379`). Empty disables every
    /// Redis-backed path; multi-replica deploys configure it so a cache added
    /// here is coordinated across replicas rather than per-process.
    pub redis_url: String,

    /// Metric read configuration.
    pub metric_catalog: MetricCatalogConfig,

    /// Usage-monitoring configuration.
    pub usage: UsageConfig,
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
            visibility_policy: VisibilityPolicy::default(),
            redis_url: String::new(),
            metric_catalog: MetricCatalogConfig::default(),
            usage: UsageConfig::default(),
        }
    }
}

/// Per-environment knobs for the metric read path.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MetricCatalogConfig {
    /// Permit authenticated callers to read tenant-level metric definitions,
    /// results, evidence, and exports.
    ///
    /// Env: `APP__gears__analytics__config__metric_catalog__tenant_metrics_enabled`.
    pub tenant_metrics_enabled: bool,

    /// Enforce the per-tenant observation filter (#1967) on metric reads.
    /// Defaults to `false`: the ingested `tenant_id` in the bronze/silver/gold
    /// pipeline is not yet aligned to the JWT tenant, so an exact
    /// `tenant_id = <session>` match would silently empty every metric read.
    /// Flip to `true` per environment once the ingest tenant is aligned (#1829).
    ///
    /// Env: `APP__gears__analytics__config__metric_catalog__enforce_tenant_scope`.
    pub enforce_tenant_scope: bool,
}

/// Whether this instance records how the product is used.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UsageConfig {
    /// Off means the SPA never starts its telemetry SDK and the ingest
    /// endpoint drops what reaches it anyway.
    ///
    /// Env: `APP__gears__analytics__config__usage__enabled`.
    pub enabled: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn default_bind_addr() -> String {
    "0.0.0.0:8081".to_owned()
}

fn default_clickhouse_database() -> String {
    "insight".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_policy_defaults_to_org_chart() -> anyhow::Result<()> {
        let config: GearConfig = serde_json::from_value(serde_json::json!({}))?;

        assert_eq!(config.visibility_policy, VisibilityPolicy::OrgChart);
        Ok(())
    }

    #[test]
    fn visibility_policy_accepts_only_wire_values() -> anyhow::Result<()> {
        for (value, expected) in [
            ("org_chart", VisibilityPolicy::OrgChart),
            ("flat", VisibilityPolicy::Flat),
        ] {
            let config: GearConfig =
                serde_json::from_value(serde_json::json!({ "visibility_policy": value }))?;

            assert_eq!(config.visibility_policy, expected, "for: {value}");
        }

        for value in ["OrgChart", "org-chart", "Flat", "unknown", ""] {
            assert!(
                serde_json::from_value::<GearConfig>(
                    serde_json::json!({ "visibility_policy": value })
                )
                .is_err(),
                "should reject: {value}"
            );
        }
        Ok(())
    }
}
