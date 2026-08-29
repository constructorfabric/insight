//! Gear configuration.
//!
//! Loaded via `GearCtx::config::<GearConfig>()` (toolkit serde path), which
//! deserializes the YAML under `gears.analytics.config`. The figment
//! loader was removed in the gears-rust migration — the toolkit host owns
//! config layering (defaults -> YAML -> env -> CLI). Env overrides are
//! `APP__gears__analytics__config__<field>` (the prefix changed from the
//! old `ANALYTICS__*`).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalSourceProvider {
    Github,
    Gitlab,
    BitbucketCloud,
    Jira,
    Youtrack,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExternalSourceConfig {
    pub id: String,
    pub provider: ExternalSourceProvider,
    pub web_base_url: String,
}

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

    /// AI-assist configuration.
    pub ai_assist: AiAssistConfig,

    /// Synchronous report generation configuration.
    pub reports: ReportsConfig,

    pub external_sources: Vec<ExternalSourceConfig>,
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
            ai_assist: AiAssistConfig::default(),
            reports: ReportsConfig::default(),
            external_sources: Vec::new(),
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ReportsConfig {
    pub temp_dir: PathBuf,
    pub max_batch_cells: usize,
    pub max_total_cells: u64,
    pub max_generated_bytes: usize,
    pub max_xlsx_spool_bytes: usize,
    pub request_timeout_secs: u64,
    pub capacity_wait_secs: u64,
    pub max_concurrent_generations: usize,
    pub max_concurrent_artifacts: usize,
    pub writer_channel_batches: usize,
}

impl Default for ReportsConfig {
    fn default() -> Self {
        Self {
            temp_dir: PathBuf::from("/app/data/reports"),
            max_batch_cells: 100_000,
            max_total_cells: 6_000_000,
            max_generated_bytes: 25 * 1024 * 1024,
            max_xlsx_spool_bytes: 90 * 1024 * 1024,
            request_timeout_secs: 120,
            capacity_wait_secs: 2,
            max_concurrent_generations: 2,
            max_concurrent_artifacts: 2,
            writer_channel_batches: 1,
        }
    }
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

/// Whether this instance can explain a metric with an LLM, and how.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AiAssistConfig {
    /// Off means the SPA renders nothing for the feature and every `/v1/ai/*`
    /// route answers "not found".
    ///
    /// Env: `APP__gears__analytics__config__ai_assist__enabled`.
    pub enabled: bool,

    /// Base64 of the 32 bytes that seal stored Anthropic tokens.
    ///
    /// Env: `APP__gears__analytics__config__ai_assist__token_encryption_key`.
    pub token_encryption_key: String,

    /// One Anthropic key for the whole stand. Set means nobody stores their
    /// own — every explanation is paid for by this key.
    ///
    /// Env: `APP__gears__analytics__config__ai_assist__api_key`.
    pub api_key: String,

    /// Restrict asking for an explanation to admins. On by default: the
    /// common setup is a stand key, and then every call spends the
    /// deployment's own money.
    ///
    /// Env: `APP__gears__analytics__config__ai_assist__admin_only`.
    pub admin_only: bool,

    /// Anthropic model the explain route asks for.
    pub model: String,

    /// Anthropic API base URL.
    pub api_base: String,

    /// Upper bound on one answer, in tokens.
    pub max_output_tokens: u32,

    /// How long one explain call may take before it is abandoned.
    pub request_timeout_secs: u64,

    /// How many explain calls may be in flight in this process at once.
    pub max_concurrent: usize,
}

impl AiAssistConfig {
    /// Whether the stand pays for explanations itself.
    #[must_use]
    pub fn has_stand_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }
}

/// Key length AES-256-GCM takes.
pub const KEY_BYTES: usize = 32;

impl Default for AiAssistConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token_encryption_key: String::new(),
            api_key: String::new(),
            admin_only: true,
            model: default_ai_model(),
            api_base: default_anthropic_api_base(),
            max_output_tokens: 512,
            request_timeout_secs: 30,
            max_concurrent: 8,
        }
    }
}

impl AiAssistConfig {
    /// The sealing key as raw bytes, or why it cannot be used.
    pub fn encryption_key(&self) -> anyhow::Result<[u8; KEY_BYTES]> {
        let raw = BASE64
            .decode(self.token_encryption_key.trim())
            .map_err(|e| anyhow::anyhow!("token_encryption_key is not valid base64: {e}"))?;

        let len = raw.len();
        <[u8; KEY_BYTES]>::try_from(raw.as_slice()).map_err(|_| {
            anyhow::anyhow!("token_encryption_key decodes to {len} bytes, expected {KEY_BYTES}")
        })
    }
}

fn default_ai_model() -> String {
    "claude-sonnet-5".to_owned()
}

fn default_anthropic_api_base() -> String {
    "https://api.anthropic.com".to_owned()
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
    fn ai_assist_defaults_to_disabled() -> anyhow::Result<()> {
        let config: GearConfig = serde_json::from_value(serde_json::json!({}))?;

        assert!(!config.ai_assist.enabled);
        assert_eq!(config.ai_assist.model, "claude-sonnet-5");
        Ok(())
    }

    #[test]
    fn a_stand_without_its_own_key_leaves_it_to_the_caller() {
        assert!(!AiAssistConfig::default().has_stand_key());
    }

    #[test]
    fn whitespace_is_not_a_stand_key() {
        let config = AiAssistConfig {
            api_key: "   ".to_owned(),
            ..AiAssistConfig::default()
        };

        assert!(!config.has_stand_key());
    }

    #[test]
    fn a_configured_stand_key_is_recognised() {
        let config = AiAssistConfig {
            api_key: "sk-ant-x".to_owned(),
            ..AiAssistConfig::default()
        };

        assert!(config.has_stand_key());
    }

    #[test]
    fn explaining_is_admin_only_unless_a_stand_opens_it() -> anyhow::Result<()> {
        let default: GearConfig = serde_json::from_value(serde_json::json!({}))?;
        assert!(default.ai_assist.admin_only);

        let opened: GearConfig = serde_json::from_value(serde_json::json!({
            "ai_assist": { "admin_only": false }
        }))?;
        assert!(!opened.ai_assist.admin_only);
        Ok(())
    }

    #[test]
    fn encryption_key_accepts_32_bytes() -> anyhow::Result<()> {
        let config = AiAssistConfig {
            token_encryption_key: BASE64.encode([7_u8; KEY_BYTES]),
            ..AiAssistConfig::default()
        };

        assert_eq!(config.encryption_key()?, [7_u8; KEY_BYTES]);
        Ok(())
    }

    #[test]
    fn encryption_key_rejects_a_key_of_the_wrong_length() {
        let config = AiAssistConfig {
            token_encryption_key: BASE64.encode([7_u8; 16]),
            ..AiAssistConfig::default()
        };

        assert!(config.encryption_key().is_err());
    }

    #[test]
    fn encryption_key_rejects_non_base64() {
        let config = AiAssistConfig {
            token_encryption_key: "not base64 !!".to_owned(),
            ..AiAssistConfig::default()
        };

        assert!(config.encryption_key().is_err());
    }

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

    #[test]
    fn external_sources_default_to_empty() -> anyhow::Result<()> {
        let config: GearConfig = serde_json::from_value(serde_json::json!({}))?;

        assert!(config.external_sources.is_empty());
        Ok(())
    }

    #[test]
    fn external_sources_deserialize_provider_and_url() -> anyhow::Result<()> {
        let config: GearConfig = serde_json::from_value(serde_json::json!({
            "external_sources": [{
                "id": "source-a",
                "provider": "gitlab",
                "web_base_url": "https://code.example.test/platform"
            }]
        }))?;

        assert_eq!(config.external_sources.len(), 1);
        assert_eq!(config.external_sources[0].id, "source-a");
        assert_eq!(
            config.external_sources[0].provider,
            ExternalSourceProvider::Gitlab
        );
        Ok(())
    }
}
