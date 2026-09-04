//! The query engine: one query contract over declared datasets.
//!
//! INVARIANT: identity is a dataset's declared policy, and no dataset in this
//! build declares one.

pub mod answer;
pub mod compile;
pub mod contract;
pub mod datasets;
#[cfg(test)]
pub mod fixtures;
pub mod plan;
pub mod validation;
pub mod violation;
