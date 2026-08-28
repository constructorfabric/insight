//! Where a person stands against the people they are compared with: their own
//! value for the window, beside the spread of the population it sits in.
//!
//! INVARIANT: the population is disclosed as statistics only, never by member.

mod assemble;
mod dto;
mod plan;
mod pool;
mod service;
mod validation;

pub use dto::{ComparisonsRequest, ComparisonsResponse, Population};
pub use service::answer;
pub(super) use validation::population;
pub use validation::validate_request;
