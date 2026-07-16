//! Gear configuration.
//!
//! Loaded via `GearCtx::config::<GearConfig>()` from the
//! `gears.identity-resolution.config` YAML section. Env overrides are
//! `APP__gears__identity-resolution__config__<field>`.

use serde::Deserialize;

/// Configuration consumed by the identity-resolution gear. Deserialized from
/// `gears.identity-resolution.config`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GearConfig {
    /// `MariaDB` connection URL.
    /// Example: `mysql://insight:password@localhost:3306/identity`
    pub database_url: String,
    /// Source instance whose `org_chart` edges populate the supervisor/parent
    /// fields of a profile (matches the .NET `AppOptions.OrgChartSourceType`).
    pub org_chart_source_type: String,
    /// Whether a profile response expands the recursive subordinates subtree.
    pub expand_subordinates: bool,
    /// Max org-tree recursion depth (cycle-safe; mirrors the .NET `MaxDepth`).
    pub max_depth: usize,
}

impl Default for GearConfig {
    fn default() -> Self {
        Self {
            database_url: String::new(),
            org_chart_source_type: "bamboohr".to_owned(),
            expand_subordinates: true,
            max_depth: 16,
        }
    }
}
