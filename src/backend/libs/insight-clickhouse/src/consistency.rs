//! Read-after-write on a replicated warehouse.
//!
//! The pair is not a stronger default. A sequential quorum insert is refused
//! while the previous one into the same table is unconfirmed
//! (`QUORUM_FOR_PREVIOUS_WRITE_NOT_SATISFIED`), so two writers of one table
//! fail each other; a sequentially consistent read on a lagging replica
//! throws (`REPLICA_IS_NOT_IN_QUORUM`) instead of answering. A call site
//! takes the pair only when a read of its own decides on a write of its own.

use crate::Topology;

/// Confirms an insert on a quorum of replicas, so the read below is
/// guaranteed to reach one that holds it. `insert_quorum_parallel = 0` is what
/// gives that read its meaning — ClickHouse makes no sequential-consistency
/// promise while quorum inserts run in parallel.
const QUORUM_WRITE: [(&str, &str); 2] =
    [("insert_quorum", "auto"), ("insert_quorum_parallel", "0")];
const CONSISTENT_READ: [(&str, &str); 1] = [("select_sequential_consistency", "1")];

pub(crate) fn write_settings(topology: Topology) -> &'static [(&'static str, &'static str)] {
    if topology.is_replicated() {
        &QUORUM_WRITE
    } else {
        &[]
    }
}

pub(crate) fn read_settings(topology: Topology) -> &'static [(&'static str, &'static str)] {
    if topology.is_replicated() {
        &CONSISTENT_READ
    } else {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_node_waits_for_nothing() {
        assert!(write_settings(Topology::Standalone).is_empty());
        assert!(read_settings(Topology::Standalone).is_empty());
    }

    #[test]
    fn a_replicated_write_is_confirmed_before_the_read_that_decides_on_it() {
        let topology = Topology::new(true, "insight_cluster");

        assert_eq!(
            write_settings(topology),
            [("insert_quorum", "auto"), ("insert_quorum_parallel", "0")]
        );
        assert_eq!(
            read_settings(topology),
            [("select_sequential_consistency", "1")]
        );
    }

    #[test]
    fn a_parallel_quorum_would_void_the_read_it_is_paired_with() {
        let parallel = write_settings(Topology::new(true, "insight_cluster"))
            .iter()
            .find(|(name, _)| *name == "insert_quorum_parallel");

        assert_eq!(parallel, Some(&("insert_quorum_parallel", "0")));
    }
}
