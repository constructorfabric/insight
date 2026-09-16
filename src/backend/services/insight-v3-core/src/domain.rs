//! Rules and computation: everything decidable without reaching another system.

pub(crate) mod assistant;
// Both are exercised by their own tests; the API that calls them arrives in
// the steps after this one. `expect` rather than `allow`, so the marker fails
// once it does.
#[cfg_attr(not(test), expect(dead_code, reason = "wired up by the dataset API"))]
pub(crate) mod dataset_ingest;
#[cfg_attr(not(test), expect(dead_code, reason = "wired up by the dataset API"))]
pub(crate) mod dataset_lifecycle;
#[cfg_attr(not(test), expect(dead_code, reason = "wired up by the dataset API"))]
pub(crate) mod datasets;
pub(crate) mod definition;
pub(crate) mod kinds;
pub(crate) mod metric_run;
pub(crate) mod query;
pub(crate) mod surfaces;
