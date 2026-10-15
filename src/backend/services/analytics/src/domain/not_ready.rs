//! Why an answer carries no rows although nothing failed.

use serde::Serialize;

/// The marker an empty answer carries when its emptiness is not a reading.
///
/// A relation the warehouse has not built yet is neither a failure nor a zero:
/// no connector has synced, or the build has not reached it. A reader that
/// sees this renders "no data yet"; one that renders `0` reports a measurement
/// nobody took.
///
/// INVARIANT: only absence sets it. A relation that exists and disagrees with
/// the deployed code (`UNKNOWN_IDENTIFIER`) still fails — that is the defect
/// the schema channel exists to catch, and an empty answer would hide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NotReadyReason {
    /// A relation the read names is absent: `UNKNOWN_TABLE` (60) for the table,
    /// `UNKNOWN_DATABASE` (81) for the whole database on a fresh install.
    SourceRelationMissing,
}
