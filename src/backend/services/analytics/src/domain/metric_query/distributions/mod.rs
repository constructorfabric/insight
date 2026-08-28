//! The shape of a subject's own per-event values: how many fall in each bin of
//! that subject's range, and where the positions a question names sit in it.
//!
//! INVARIANT: only a computation over per-row values has a distribution.

mod assemble;
mod dto;
mod plan;
mod service;
mod validation;

pub use dto::{DistributionsRequest, DistributionsResponse};
pub use service::answer;
pub(super) use validation::distributable;
pub use validation::validate_request;
