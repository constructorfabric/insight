//! Canonical error types for the identity-resolution HTTP surface.
//!
//! Binds a GTS resource namespace; the builders (`not_found`, `invalid_argument`,
//! `aborted`, …) come from `toolkit-canonical-errors` and serialize to an
//! RFC 9457 `application/problem+json` envelope.

use toolkit_canonical_errors::resource_error;

#[resource_error("gts.cf.insight.identity_resolution.profile.v1~")]
pub struct ProfileError;

#[resource_error("gts.cf.insight.identity_resolution.persons_seed.v1~")]
pub struct PersonsSeedError;
