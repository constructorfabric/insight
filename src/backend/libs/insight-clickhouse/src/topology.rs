//! Topology of the target `ClickHouse` — the shape of the DDL a creator emits.

/// Whether relations this service creates must replicate, and which cluster
/// carries the `ON CLUSTER` clause.
///
/// One value rather than a `bool` beside an `Option<String>`: `ON CLUSTER`
/// without `Replicated*` engines creates an unreplicated table on every node,
/// which is the failure this distinction exists to prevent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Topology {
    /// A single node: the plain `MergeTree` family, no `ON CLUSTER`.
    #[default]
    Standalone,
    /// `Replicated*` engines. `on_cluster` names the cluster for the
    /// `ON CLUSTER` clause; the chart refuses a clustered install that names
    /// none, so `None` is a topology this service is never configured with.
    Replicated { on_cluster: Option<String> },
}

impl Topology {
    /// Reads the operator's two settings as one state.
    ///
    /// `cluster_mode` is the switch. A cluster name given while it is off
    /// names no cluster — nothing this service creates is replicated, so
    /// there is no DDL for the clause to qualify.
    #[must_use]
    pub fn new(cluster_mode: bool, cluster_name: &str) -> Self {
        if !cluster_mode {
            return Self::Standalone;
        }

        let name = cluster_name.trim();
        Self::Replicated {
            on_cluster: (!name.is_empty()).then(|| name.to_owned()),
        }
    }

    /// Whether created engines carry the `Replicated` prefix.
    #[must_use]
    pub fn is_replicated(&self) -> bool {
        matches!(self, Self::Replicated { .. })
    }

    /// The cluster the `ON CLUSTER` clause names, when there is one.
    #[must_use]
    pub fn on_cluster(&self) -> Option<&str> {
        match self {
            Self::Standalone => None,
            Self::Replicated { on_cluster } => on_cluster.as_deref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_node_install_is_the_default() {
        assert_eq!(Topology::default(), Topology::Standalone);
        assert!(!Topology::default().is_replicated());
        assert!(Topology::default().on_cluster().is_none());
    }

    #[test]
    fn a_cluster_name_alone_does_not_make_an_install_clustered() {
        let topology = Topology::new(false, "insight_cluster");

        assert_eq!(topology, Topology::Standalone);
    }

    #[test]
    fn the_flag_alone_replicates_without_naming_a_cluster() {
        let topology = Topology::new(true, "");

        assert!(topology.is_replicated());
        assert!(topology.on_cluster().is_none());
    }

    #[test]
    fn a_named_cluster_replicates_and_qualifies_its_ddl() {
        let topology = Topology::new(true, "  insight_cluster  ");

        assert!(topology.is_replicated());
        assert_eq!(topology.on_cluster(), Some("insight_cluster"));
    }
}
