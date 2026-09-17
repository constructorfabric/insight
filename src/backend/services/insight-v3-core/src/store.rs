//! The systems this service reaches: the warehouse, and the identity service.

pub(crate) mod dataset_tables;
pub(crate) mod datasets;
pub(crate) mod definitions;
pub(crate) mod identity;

/// A search needle as a `LIKE` pattern reads it, so a name holding `%` or `_`
/// matches itself rather than everything.
fn like_escaped(needle: &str) -> String {
    needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
