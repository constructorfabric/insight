//! Canonical error type for the previews HTTP surface.
//!
//! Binds a GTS resource namespace; the builders (`not_found`,
//! `invalid_argument`, `resource_exhausted`, …) come from
//! `toolkit-canonical-errors` and serialize to an RFC 9457
//! `application/problem+json` envelope.

use toolkit_canonical_errors::resource_error;

#[resource_error("gts.cf.insight.previews.experiment.v1~")]
pub struct ExperimentError;
