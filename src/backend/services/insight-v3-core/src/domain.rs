//! Rules and computation: everything decidable without reaching another system.

pub(crate) mod assistant;
// The ingest endpoint that addresses a dataset arrives in a later step;
// `expect` rather than `allow`, so the marker fails once it does.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired up by the ingest endpoint")
)]
pub(crate) mod dataset_ingest;
pub(crate) mod dataset_lifecycle;
pub(crate) mod datasets;
pub(crate) mod definition;
pub(crate) mod kinds;
pub(crate) mod metric_run;
pub(crate) mod query;
pub(crate) mod surfaces;
pub(crate) mod violation;
