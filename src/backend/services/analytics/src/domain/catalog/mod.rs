//! Metric-catalog read path (Refs #524).
//!
//! Three components per DESIGN §3.2:
//! - [`reader::CatalogReader`] (`cpt-metric-cat-component-catalog-reader`) —
//!   orchestrates cache lookup, resolver call on miss, and serialization.
//! - [`resolver::ThresholdResolver`] (`cpt-metric-cat-component-threshold-resolver`) —
//!   exactly one bulk SQL fetch per request, then an in-memory most-specific-wins
//!   walk over `{ product-default, tenant, role, team, team+role }`. Halts on
//!   the first locked broader-scope row and surfaces `bounded_by_lock = true`.
//! - [`response`] — wire shape for `POST /v1/catalog/get_metrics`. Crucially carries
//!   `id` (UUIDv7) and NOT `metric_key`: the latter is `<table>.<column>` form
//!   and stays backend-internal so consumers can't couple to ClickHouse
//!   source-schema names.

pub mod reader;
pub mod resolver;
pub mod response;

#[cfg(test)]
mod live_tests;

pub use reader::CatalogReader;
pub use resolver::ThresholdResolver;
