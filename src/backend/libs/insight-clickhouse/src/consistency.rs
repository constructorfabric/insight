//! Read-after-write on a replicated warehouse.
//!
//! Behind a load balancer a read can reach a replica that has not merged the
//! parts a write just made. A caller whose next decision depends on its own
//! write — a count that verifies a batch, a watermark a swap is gated on —
//! opts into the pair below through
//! [`consistent_insert`](crate::Client::consistent_insert) and
//! [`consistent_query`](crate::Client::consistent_query).
//!
//! Both are empty on a standalone warehouse: there is one replica, and a read
//! there already sees every write.

use crate::Topology;

/// Confirms an insert on a quorum of replicas, so the read below is
/// guaranteed to reach one that holds it. `insert_quorum_parallel = 0` is what
/// gives that read its meaning — ClickHouse makes no sequential-consistency
/// promise while quorum inserts run in parallel.
const QUORUM_WRITE: [(&str, &str); 2] =
    [("insert_quorum", "auto"), ("insert_quorum_parallel", "0")];
/// Pins a read to the quorum-confirmed parts.
const CONSISTENT_READ: [(&str, &str); 1] = [("select_sequential_consistency", "1")];

pub(crate) fn write_settings(topology: &Topology) -> &'static [(&'static str, &'static str)] {
    if topology.is_replicated() {
        &QUORUM_WRITE
    } else {
        &[]
    }
}

pub(crate) fn read_settings(topology: &Topology) -> &'static [(&'static str, &'static str)] {
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
        assert!(write_settings(&Topology::Standalone).is_empty());
        assert!(read_settings(&Topology::Standalone).is_empty());
    }

    #[test]
    fn a_replicated_write_is_confirmed_before_the_read_that_decides_on_it() {
        let topology = Topology::new(true, "insight_cluster");

        assert_eq!(
            write_settings(&topology),
            [("insert_quorum", "auto"), ("insert_quorum_parallel", "0")]
        );
        assert_eq!(
            read_settings(&topology),
            [("select_sequential_consistency", "1")]
        );
    }

    #[test]
    fn a_parallel_quorum_would_void_the_read_it_is_paired_with() {
        let parallel = write_settings(&Topology::new(true, "insight_cluster"))
            .iter()
            .find(|(name, _)| *name == "insert_quorum_parallel");

        assert_eq!(parallel, Some(&("insert_quorum_parallel", "0")));
    }
}
