//! Gear configuration.
//!
//! Loaded via `GearCtx::config::<GearConfig>()` from the
//! `gears.identity-resolution.config` YAML section. Env overrides are
//! `APP__gears__identity_resolution__config__<field>`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityPolicy {
    #[default]
    OrgChart,
    Flat,
}

impl VisibilityPolicy {
    #[must_use]
    pub fn is_flat(self) -> bool {
        matches!(self, Self::Flat)
    }
}

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
    /// The one source trusted to say who exists: the persons-seed mints a person
    /// for its accounts even when they carry no address to match on, and asks an
    /// operator to confirm each one. Empty — the default — mints from an address
    /// only. Naming more than one source is not possible on purpose: two
    /// rosters give one addressless human two persons and nothing joins them.
    pub roster_source_type: String,
    /// Whether a profile response expands the recursive subordinates subtree.
    pub expand_subordinates: bool,
    /// Max org-tree recursion depth (cycle-safe; mirrors the .NET `MaxDepth`).
    pub max_depth: usize,
    /// `ClickHouse` HTTP URL for reading `identity_inputs` (persons-seed input).
    pub clickhouse_url: String,
    /// `ClickHouse` database (the `identity_inputs` table lives in `identity`).
    pub clickhouse_database: String,
    /// `ClickHouse` user (empty = no auth).
    pub clickhouse_user: String,
    /// `ClickHouse` password.
    pub clickhouse_password: String,
    /// Default tenant for the bootstrap-admin seed (mirrors the .NET
    /// `AppOptions.TenantDefaultId`). Empty = bootstrap skipped with a warning
    /// when a bootstrap person is configured.
    pub tenant_default_id: String,
    /// First-admin seed for the admin-gated CRUD endpoints (mirrors the .NET
    /// `AppOptions.BootstrapAdminPersonId`): on `migrate`, this person gets an
    /// active `admin` assignment in `tenant_default_id` unless one already
    /// exists. Empty = disabled.
    pub bootstrap_admin_person_id: String,
    /// Whose data a caller may see: the reporting line plus explicit grants
    /// (`org_chart`), or every person in the tenant (`flat`). Nothing is
    /// written to `visibility`, so the choice is reversible.
    pub visibility_policy: VisibilityPolicy,
}

impl Default for GearConfig {
    fn default() -> Self {
        Self {
            database_url: String::new(),
            org_chart_source_type: "bamboohr".to_owned(),
            roster_source_type: String::new(),
            expand_subordinates: true,
            max_depth: 16,
            clickhouse_url: String::new(),
            clickhouse_database: "identity".to_owned(),
            clickhouse_user: String::new(),
            clickhouse_password: String::new(),
            tenant_default_id: String::new(),
            bootstrap_admin_person_id: String::new(),
            visibility_policy: VisibilityPolicy::OrgChart,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: serde_json::Value) -> Result<GearConfig, serde_json::Error> {
        serde_json::from_value(json)
    }

    #[test]
    fn an_absent_policy_reads_as_org_chart() -> anyhow::Result<()> {
        let config = config(serde_json::json!({}))?;

        assert_eq!(config.visibility_policy, VisibilityPolicy::OrgChart);
        Ok(())
    }

    #[test]
    fn each_policy_is_named_as_the_wire_names_it() -> anyhow::Result<()> {
        for (value, expected) in [
            ("org_chart", VisibilityPolicy::OrgChart),
            ("flat", VisibilityPolicy::Flat),
        ] {
            let config = config(serde_json::json!({ "visibility_policy": value }))?;

            assert_eq!(config.visibility_policy, expected, "for: {value}");
        }
        Ok(())
    }

    #[test]
    fn a_value_that_is_not_a_policy_refuses_to_load() {
        // A policy decides who may see whom, so an unreadable one must stop the
        // service rather than resolve to whichever branch the type defaults to.
        for value in ["Flat", "FLAT", "flatt", "", "org-chart"] {
            assert!(
                config(serde_json::json!({ "visibility_policy": value })).is_err(),
                "should refuse: {value:?}"
            );
        }
    }

    #[test]
    fn only_the_flat_policy_reports_itself_flat() {
        assert!(VisibilityPolicy::Flat.is_flat());
        assert!(!VisibilityPolicy::OrgChart.is_flat());
    }
}
