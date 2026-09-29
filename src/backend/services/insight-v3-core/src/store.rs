//! The systems this service reaches: the warehouse, and the identity service.

pub(crate) mod alert_schedule;
pub(crate) mod alerts;
pub(crate) mod catalog;
pub(crate) mod dataset_tables;
pub(crate) mod datasets;
pub(crate) mod definitions;
pub(crate) mod identity;
pub(crate) mod providers;

/// A search needle as a `LIKE` pattern reads it, so a name holding `%` or `_`
/// matches itself rather than everything.
fn like_escaped(needle: &str) -> String {
    needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
