//! Rules and computation: everything decidable without reaching another system.

pub(crate) mod assistant;
// The store is exercised by its own tests; the create and remove flows that
// call it arrive in the steps after this one. `expect` rather than `allow`, so
// the marker fails once they do.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired up by the dataset flows and API")
)]
pub(crate) mod datasets;
pub(crate) mod definition;
pub(crate) mod kinds;
pub(crate) mod metric_run;
pub(crate) mod query;
pub(crate) mod surfaces;
