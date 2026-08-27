//! The rows behind a value: the dataset rows one input of a metric's
//! computation folded, reported one per row and one page at a time.
//!
//! INVARIANT: a page scopes its scan exactly as the value scoped its own, so
//! what a page shows cannot drift from what the value counted.

mod columns;
mod cursor;
mod dto;
mod plan;
mod service;
mod validation;

pub use dto::{RowsRequest, RowsResponse};
pub use service::answer;
pub use validation::validate_request;
