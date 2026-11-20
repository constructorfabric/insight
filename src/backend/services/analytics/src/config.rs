use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use secrecy::SecretString;
use serde::Deserialize;

const REDACTED_SECRET: &str = "<redacted>";

pub(crate) fn redacted_yaml(config: &toolkit::bootstrap::AppConfig) -> anyhow::Result<String> {
    let mut redacted = config.clone();
    for (section, key) in [("mcp", "clickhouse_password"), ("sql_api", "token")] {
        if let Some(secret) = redacted
            .gears
            .get_mut("analytics")
            .and_then(|gear| gear.get_mut("config"))
            .and_then(|config| config.get_mut(section))
            .and_then(|section| section.get_mut(key))
        {
            *secret = serde_json::Value::String(REDACTED_SECRET.to_owned());
        }
    }
    redacted.to_yaml()
}

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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub enabled: bool,
    pub bind_addr: String,
    pub public_url: String,
    pub allow_insecure_private_network: bool,
    pub max_concurrent_queries: usize,
    pub clickhouse_user: String,
    pub clickhouse_password: SecretString,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SqlApiConfig {
    pub enabled: bool,
    pub token: SecretString,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_addr: "0.0.0.0:8086".to_owned(),
            public_url: String::new(),
            allow_insecure_private_network: false,
            max_concurrent_queries: 16,
            clickhouse_user: "insight_mcp".to_owned(),
            clickhouse_password: SecretString::from(String::new()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GearConfig {
    pub bind_addr: String,

    pub database_url: String,

    pub clickhouse_url: String,

    pub clickhouse_database: String,

    pub clickhouse_user: Option<String>,

    pub clickhouse_password: Option<String>,

    pub identity_url: String,

    pub visibility_policy: VisibilityPolicy,

    pub redis_url: String,

    pub metric_catalog: MetricCatalogConfig,

    pub usage: UsageConfig,

    pub ai_assist: AiAssistConfig,

    pub reports: ReportsConfig,

    pub mcp: McpConfig,
    pub sql_api: SqlApiConfig,

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
            mcp: McpConfig::default(),
            sql_api: SqlApiConfig::default(),
            external_sources: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MetricCatalogConfig {
    pub tenant_metrics_enabled: bool,

    // SAFETY: tenancy shortcut (#1829, #1967): while false, metric reads are not tenant-scoped.
    // Keep false until ingested `tenant_id` matches the JWT tenant; enforcing earlier empties every read.
    pub enforce_tenant_scope: bool,

    pub periodic_validation_enabled: bool,
}

impl Default for MetricCatalogConfig {
    fn default() -> Self {
        Self {
            tenant_metrics_enabled: false,
            enforce_tenant_scope: false,
            periodic_validation_enabled: true,
        }
    }
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UsageConfig {
    pub enabled: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AiAssistConfig {
    pub enabled: bool,

    pub token_encryption_key: String,

    pub api_key: String,

    pub admin_only: bool,

    pub model: String,

    pub api_base: String,

    pub max_output_tokens: u32,

    pub request_timeout_secs: u64,

    pub max_concurrent: usize,
}

impl AiAssistConfig {
    #[must_use]
    pub fn has_stand_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }
}

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
    fn printed_config_redacts_mcp_clickhouse_password() -> anyhow::Result<()> {
        let password = "mcp-password-that-must-not-print";
        let token = "synthetic-sql-api-token-that-must-not-print";
        let mut config = toolkit::bootstrap::AppConfig::default();
        config.gears.insert(
            "analytics".to_owned(),
            serde_json::json!({
                "config": {
                    "mcp": { "clickhouse_password": password },
                    "sql_api": { "token": token }
                }
            }),
        );

        let printed = redacted_yaml(&config)?;

        assert!(printed.contains(REDACTED_SECRET));
        assert!(!printed.contains(password));
        assert!(!printed.contains(token));
        Ok(())
    }

    #[test]
    fn ai_assist_defaults_to_disabled() -> anyhow::Result<()> {
        let config: GearConfig = serde_json::from_value(serde_json::json!({}))?;

        assert!(!config.ai_assist.enabled);
        assert_eq!(config.ai_assist.model, "claude-sonnet-5");
        Ok(())
    }

    #[test]
    fn metric_validation_repeats_unless_disabled() -> anyhow::Result<()> {
        let omitted: GearConfig = serde_json::from_value(serde_json::json!({
            "metric_catalog": {
                "tenant_metrics_enabled": false,
                "enforce_tenant_scope": false
            }
        }))?;
        assert!(omitted.metric_catalog.periodic_validation_enabled);

        let one_shot: GearConfig = serde_json::from_value(serde_json::json!({
            "metric_catalog": { "periodic_validation_enabled": false }
        }))?;
        assert!(!one_shot.metric_catalog.periodic_validation_enabled);
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
