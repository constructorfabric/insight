//! Connector health — what the ledger records about every connector's syncs.
//!
//! The write side is the reconcile loop's sweep
//! (`src/ingestion/reconcile-connectors/python/sweep`), which copies the data
//! mover's own account of each sync into `ingestion_history.sync_events`. This
//! module only reads it, and reads nothing else: no mover call, no cluster API,
//! no bronze access. That is what lets the page answer during exactly the
//! incident an operator opens it for.
//!
//! Spec: `docs/components/backend/analytics/specs/connector-health`.

mod model;
mod name;
mod read;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;

pub(crate) use model::{ConnectorHealthResponse, SyncHistoryResponse};
pub(crate) use name::ConnectorName;
pub(crate) use read::{HISTORY_WINDOW, read_health, read_syncs};
