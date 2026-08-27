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
        }
    }
}

impl GearConfig {
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
}
