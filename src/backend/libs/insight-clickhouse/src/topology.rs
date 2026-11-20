#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Topology {
    #[default]
    Standalone,
    Replicated,
}

impl Topology {
    #[must_use]
    pub fn new(cluster_mode: bool, _cluster_name: &str) -> Self {
        if cluster_mode {
            Self::Replicated
        } else {
            Self::Standalone
        }
    }

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
