//! How long a statement a `Replicated` database replays waits for the other
//! replicas.
//!
//! The server's default is to wait for every replica, active or not, for
//! three minutes, and then to answer with a timeout for a statement that has
//! already landed on the live ones. A service calling from a request handler
//! cannot wait that long, and a replica that is down will not confirm anything
//! however long the wait: it replays its queue entry when it returns. So in
//! cluster mode every statement [`execute_ddl`](crate::Client::execute_ddl)
//! runs waits for the active replicas only, for at most [`DDL_TASK_TIMEOUT`],
//! which is also what the deploy's own DDL does (`scripts/lib/ch-exec.sh`).
//!
//! A caller that bounds the call on its side keeps its bound above
//! [`DDL_TASK_TIMEOUT`]: that way the warehouse, not the caller, is the one
//! to report a statement that did not finish, and a statement that did finish
//! is never mistaken for one that did not.
//!
//! Empty on a standalone warehouse: there is nothing to wait for.

use std::time::Duration;

use crate::Topology;
use crate::missing_relation::reported_code;

/// The longest a coordinated statement waits for the active replicas before
/// the warehouse reports it unfinished.
pub const DDL_TASK_TIMEOUT: Duration = Duration::from_secs(30);

/// `TIMEOUT_EXCEEDED`.
const TIMEOUT_EXCEEDED: u32 = 159;
const OUTPUT_MODE: &str = "distributed_ddl_output_mode";
const ACTIVE_REPLICAS_ONLY: &str = "throw_only_active";
const TASK_TIMEOUT: &str = "distributed_ddl_task_timeout";

/// A statement the warehouse stopped waiting on before the replicas finished
/// it: reported as a timeout, but still queued, so whether it lands is
/// unknown to the caller. A drop sent to undo it could land first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the warehouse stopped waiting on a statement before it finished")]
pub struct Unfinished;

impl Unfinished {
    /// Reads a `ClickHouse` error.
    #[must_use]
    pub fn from_error(error: &clickhouse::error::Error) -> Option<Self> {
        Self::from_message(&error.to_string())
    }

    /// Reads a server message a caller holds as text rather than as an error.
    #[must_use]
    pub fn from_message(message: &str) -> Option<Self> {
        (reported_code(message)? == TIMEOUT_EXCEEDED).then_some(Self)
    }
}

pub(crate) fn settings(topology: Topology) -> Vec<(&'static str, String)> {
    if topology.is_replicated() {
        vec![
            (OUTPUT_MODE, ACTIVE_REPLICAS_ONLY.to_owned()),
            (TASK_TIMEOUT, DDL_TASK_TIMEOUT.as_secs().to_string()),
        ]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_node_waits_for_nothing() {
        assert!(settings(Topology::Standalone).is_empty());
    }

    #[test]
    fn a_statement_the_warehouse_stopped_waiting_on_is_unfinished() {
        for message in [
            "bad response: Code: 159. DB::Exception: Watching task /clickhouse/task_queue/ddl/query-0000000001 \
             is executing longer than distributed_ddl_task_timeout (=30) seconds. (TIMEOUT_EXCEEDED)",
            "Code: 159",
        ] {
            assert_eq!(
                Unfinished::from_message(message),
                Some(Unfinished),
                "should read as unfinished: {message:?}"
            );
        }
    }

    #[test]
    fn a_statement_the_warehouse_refused_is_not_unfinished() {
        for message in [
            "Code: 57. DB::Exception: Table identity.x already exists. (TABLE_ALREADY_EXISTS)",
            "Code: 279. DB::Exception: All connection tries failed: Code: 159. (TIMEOUT_EXCEEDED)",
        ] {
            assert_eq!(
                Unfinished::from_message(message),
                None,
                "a refusal is final: {message}"
            );
        }
    }

    #[test]
    fn a_replicated_statement_waits_for_the_active_replicas_within_the_bound() {
        let settings = settings(Topology::new(true, "insight_cluster"));

        assert_eq!(
            settings,
            [
                (
                    "distributed_ddl_output_mode",
                    "throw_only_active".to_owned()
                ),
                (
                    "distributed_ddl_task_timeout",
                    DDL_TASK_TIMEOUT.as_secs().to_string()
                ),
            ]
        );
    }
}
