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

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct GearConfig {
    pub database_url: String,
    pub org_chart_source_type: String,
    pub roster_source_type: String,
    pub expand_subordinates: bool,
    pub max_depth: usize,
    pub clickhouse_url: String,
    pub clickhouse_database: String,
    pub clickhouse_user: String,
    pub clickhouse_password: String,
    pub clickhouse_cluster_mode: bool,
    pub clickhouse_cluster_name: String,
    pub tenant_default_id: String,
    pub bootstrap_admin_person_id: String,
    pub visibility_policy: VisibilityPolicy,
}

impl std::fmt::Debug for GearConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const REDACTED: &str = "<redacted>";
        f.debug_struct("GearConfig")
            .field("database_url", &REDACTED)
            .field("org_chart_source_type", &self.org_chart_source_type)
            .field("roster_source_type", &self.roster_source_type)
            .field("expand_subordinates", &self.expand_subordinates)
            .field("max_depth", &self.max_depth)
            .field("clickhouse_url", &self.clickhouse_url)
            .field("clickhouse_database", &self.clickhouse_database)
            .field("clickhouse_user", &self.clickhouse_user)
            .field("clickhouse_password", &REDACTED)
            .field("clickhouse_cluster_mode", &self.clickhouse_cluster_mode)
            .field("clickhouse_cluster_name", &self.clickhouse_cluster_name)
            .field("tenant_default_id", &self.tenant_default_id)
            .field("bootstrap_admin_person_id", &self.bootstrap_admin_person_id)
            .field("visibility_policy", &self.visibility_policy)
            .finish()
    }
}

impl GearConfig {
    #[must_use]
    pub fn topology(&self) -> insight_clickhouse::Topology {
        insight_clickhouse::Topology::new(
            self.clickhouse_cluster_mode,
            &self.clickhouse_cluster_name,
        )
    }
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
            clickhouse_cluster_mode: false,
            clickhouse_cluster_name: String::new(),
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

    #[test]
    fn an_install_that_says_nothing_is_a_standalone_single_node() -> anyhow::Result<()> {
        let config = config(serde_json::json!({}))?;

        assert_eq!(config.topology(), insight_clickhouse::Topology::Standalone);
        Ok(())
    }

    #[test]
    fn the_operators_pair_reaches_the_writer_as_one_topology() -> anyhow::Result<()> {
        let config = config(serde_json::json!({
            "clickhouse_cluster_mode": true,
            "clickhouse_cluster_name": "insight_cluster",
        }))?;

        assert_eq!(config.topology(), insight_clickhouse::Topology::Replicated);
        Ok(())
    }
}
