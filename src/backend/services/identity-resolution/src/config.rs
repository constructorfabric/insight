//! Gear configuration.
//!
//! Loaded via `GearCtx::config::<GearConfig>()` from the
//! `gears.identity-resolution.config` YAML section. Env overrides are
//! `APP__gears__identity-resolution__config__<field>`.

use serde::Deserialize;

/// Configuration consumed by the identity-resolution gear. Deserialized from
/// `gears.identity-resolution.config`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GearConfig {
    /// `MariaDB` connection URL.
    /// Example: `mysql://insight:password@localhost:3306/identity`
    pub database_url: String,
}
