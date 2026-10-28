//! Topology of the target `ClickHouse` — whether the tables a service creates
//! replicate.

/// Whether the tables this service creates must replicate.
///
/// Decides the engine family [`Ddl`](crate::Ddl) renders and the
/// read-after-write settings a swap needs. No statement a service emits
/// carries `ON CLUSTER`: inside a `Replicated` database ClickHouse rejects it
/// on table DDL, and the databases are created elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Topology {
    /// A single node: the plain `MergeTree` family.
    #[default]
    Standalone,
    /// Every engine carries the `Replicated` prefix, argument-less; the server
    /// fills the Keeper path from its defaults.
    Replicated,
}

impl Topology {
    /// Reads the operator's two settings as one state.
    ///
    /// `cluster_mode` is the switch. `cluster_name` is accepted because the
    /// chart injects the pair (`clickhouse_cluster_mode`,
    /// `clickhouse_cluster_name`) into every creator under one contract; the
    /// services emit no `ON CLUSTER`, so the name is unused here.
    #[must_use]
    pub fn new(cluster_mode: bool, _cluster_name: &str) -> Self {
        if cluster_mode {
            Self::Replicated
        } else {
            Self::Standalone
        }
    }

    /// Whether created engines carry the `Replicated` prefix.
    #[must_use]
    pub fn is_replicated(self) -> bool {
        matches!(self, Self::Replicated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_node_install_is_the_default() {
        assert_eq!(Topology::default(), Topology::Standalone);
        assert!(!Topology::default().is_replicated());
    }

    #[test]
    fn a_cluster_name_alone_does_not_make_an_install_clustered() {
        assert_eq!(
            Topology::new(false, "insight_cluster"),
            Topology::Standalone
        );
    }

    #[test]
    fn the_flag_replicates_with_or_without_a_cluster_name() {
        assert_eq!(Topology::new(true, ""), Topology::Replicated);
        assert_eq!(
            Topology::new(true, "  insight_cluster  "),
            Topology::Replicated
        );
        assert!(Topology::new(true, "").is_replicated());
    }
}
