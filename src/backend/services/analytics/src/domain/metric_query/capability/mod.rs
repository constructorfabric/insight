//! What the definitions can be asked: every metric, with the questions each
//! kind of read admits for it. The whole set is compiled into the binary and
//! answered in one document, so it takes no paging and no narrowing.
//!
//! INVARIANT: capability is a projection of the definitions, identical on every
//! tenant and on an empty one.

mod describe;
mod dto;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

pub use describe::describe;
pub use dto::MetricCatalogResponse;
