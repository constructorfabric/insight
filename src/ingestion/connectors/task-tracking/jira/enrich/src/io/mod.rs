//! ClickHouse I/O layer. Feature-gated behind `io` because the `clickhouse` crate
//! requires rustc 1.89+ (see top-level `Cargo.toml`).
//!
//! Contains:
//!   - `ch_client` — shared connection wrapper
//!   - `schema`    — startup validation of `staging.jira__task_field_history` DDL
//!   - `reader`    — Bronze/Staging queries (per-issue HWM, last-state, new events, snapshots)
//!   - `writer`    — batched INSERT into `staging.jira__task_field_history`

pub mod ch_client;
pub mod reader;
pub mod schema;
pub mod writer;

#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("ClickHouse error: {0}")]
    ClickHouse(#[from] clickhouse::error::Error),

    #[error("Schema mismatch: {0}")]
    SchemaMismatch(String),

    #[error("Cursor conflict: another run appears to be in progress")]
    CursorConflict,

    #[error("ClickHouse INSERT timed out after {0}s (writer={1}, batch={2}, rows={3})")]
    InsertTimeout(u64, usize, usize, usize),
}
