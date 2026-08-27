//! Answering questions about metrics from the semantic definitions: parse the
//! question into typed values, decide which read answers it, run it, and state
//! the answer in explicit fields. One directory per kind of question, over the
//! definitions, refusals, reads and provenance they all share.

mod catalog;
pub(crate) mod dto;
mod error;
mod execute;
mod provenance;
mod question;

pub mod comparisons;
pub mod distributions;
pub mod values;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod fixtures;

pub use catalog::product_metric_catalog;
