//! Gear configuration.
//!
//! Loaded via `GearCtx::config::<GearConfig>()` from the
//! `gears.previews.config` YAML section. Env overrides are
//! `APP__gears__previews__config__<field>`.

use serde::Deserialize;

use crate::domain::objects::RouteTarget;

/// Configuration consumed by the previews gear. Deserialized from
/// `gears.previews.config`. The image repository is NOT here on purpose — it
/// is hardcoded ([`crate::domain::objects::IMAGE_REPOSITORY`]) so no
/// deployment can widen the image surface.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GearConfig {
    /// The ONE namespace this service creates and deletes experiment objects
    /// in; it never touches another.
    pub namespace: String,
    /// The shared Gateway experiments' routes attach to.
    pub gateway_name: String,
    pub gateway_namespace: String,
    /// Target listener on the Gateway (e.g. `https`). Empty = all listeners.
    pub gateway_section_name: String,
    /// The single preview host every experiment serves under. Empty renders
    /// host-relative URLs in responses; route creation still requires it.
    pub route_host: String,
    /// Prefix experiments live under (`/exp/<name>`, prefix-stripped).
    pub base_path: String,
    /// Cap on live experiments; a create beyond it is refused.
    pub max_experiments: usize,
    /// TTL applied when a create names none.
    pub default_ttl_days: u32,
    /// Longest TTL a create may ask for.
    pub max_ttl_days: u32,
    /// How often the TTL sweep looks for expired experiments.
    pub sweep_interval_secs: u64,
    /// OCI registry for tag listing; empty disables `GET /v1/images`.
    pub registry_url: String,
    /// Bearer credential, needed only for a private repository (for GHCR,
    /// the base64-encoded read token); empty lists anonymously.
    pub registry_token: String,
}

impl Default for GearConfig {
    fn default() -> Self {
        Self {
            namespace: "insight-previews".to_owned(),
            gateway_name: "insight".to_owned(),
            gateway_namespace: "insight-infra".to_owned(),
            gateway_section_name: String::new(),
            route_host: String::new(),
            base_path: "/exp".to_owned(),
            max_experiments: 10,
            default_ttl_days: 7,
            max_ttl_days: 30,
            sweep_interval_secs: 300,
            registry_url: "https://ghcr.io".to_owned(),
            registry_token: String::new(),
        }
    }
}

/// Keeps `TtlDays::expires_at` far inside chrono's representable range.
const TTL_DAYS_CEILING: u32 = 3650;

impl GearConfig {
    /// Refuse a config the request path could only mis-serve: a default TTL
    /// outside `1..=max_ttl_days` would fail every ttl-less create, and an
    /// unbounded maximum would let expiry arithmetic leave chrono's range.
    ///
    /// # Errors
    ///
    /// Returns an error naming the violated bound.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.default_ttl_days >= 1 && self.default_ttl_days <= self.max_ttl_days,
            "default_ttl_days ({}) must be within 1..=max_ttl_days ({})",
            self.default_ttl_days,
            self.max_ttl_days
        );
        anyhow::ensure!(
            self.max_ttl_days <= TTL_DAYS_CEILING,
            "max_ttl_days ({}) must be at most {TTL_DAYS_CEILING}",
            self.max_ttl_days
        );
        anyhow::ensure!(
            self.max_experiments >= 1,
            "max_experiments must be at least 1"
        );
        anyhow::ensure!(
            self.registry_token.is_empty() || !self.registry_url.is_empty(),
            "registry_token is set but registry_url is empty"
        );
        Ok(())
    }

    #[must_use]
    pub fn route_target(&self) -> RouteTarget {
        RouteTarget {
            gateway_name: self.gateway_name.clone(),
            gateway_namespace: self.gateway_namespace.clone(),
            gateway_section_name: self.gateway_section_name.clone(),
            host: self.route_host.clone(),
            base_path: self.base_path.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_section_reads_as_the_documented_defaults() -> anyhow::Result<()> {
        let config: GearConfig = serde_json::from_value(serde_json::json!({}))?;

        assert_eq!(config.namespace, "insight-previews");
        assert_eq!(config.base_path, "/exp");
        assert_eq!(config.gateway_name, "insight");
        assert_eq!(config.gateway_namespace, "insight-infra");
        assert!(config.max_experiments > 0);
        assert!(config.default_ttl_days <= config.max_ttl_days);
        Ok(())
    }

    #[test]
    fn the_default_config_validates() {
        assert!(GearConfig::default().validate().is_ok());
    }

    #[test]
    fn a_config_the_request_path_could_only_mis_serve_refuses_to_boot() {
        type BreakIt = fn(&mut GearConfig);

        let cases: [(&str, BreakIt); 5] = [
            ("a registry token without a url", |c| {
                c.registry_token = "token".to_owned();
                c.registry_url = String::new();
            }),
            ("zero default ttl", |c| c.default_ttl_days = 0),
            ("default above max", |c| {
                c.default_ttl_days = c.max_ttl_days + 1;
            }),
            ("unbounded max ttl", |c| c.max_ttl_days = u32::MAX),
            ("zero experiment cap", |c| c.max_experiments = 0),
        ];
        for (label, break_it) in cases {
            let mut config = GearConfig::default();
            break_it(&mut config);

            assert!(config.validate().is_err(), "should refuse: {label}");
        }
    }
}
