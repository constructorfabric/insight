//! The semantic layer's definition format and its one write path: the typed
//! shapes a measure and a metric are authored in, the closed grammars they
//! compose from (filter trees, scalar expressions), and the store that versions
//! and audits every write.
//!
//! The grammars own structural validity — a value that parses is well-formed in
//! isolation. Binding to a dataset's field catalog (does the field exist, does
//! its type admit the operator) is the seed pipeline's second validation stage.

pub mod definition;
pub mod expr;
pub mod filter;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod live_tests;
pub mod seeds;
pub mod store;
pub mod validate;
