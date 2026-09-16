//! The systems this service reaches: the warehouse, and the identity service.

pub(crate) mod catalog;
// The store is exercised by its own tests; the create and remove flows that
// call it arrive in the steps after this one. `expect` rather than `allow`, so
// the marker fails once they do.
#[expect(dead_code, reason = "wired up by the dataset flows and API")]
pub(crate) mod datasets;
pub(crate) mod definitions;
pub(crate) mod identity;
pub(crate) mod raw_data;
pub(crate) mod tables;
