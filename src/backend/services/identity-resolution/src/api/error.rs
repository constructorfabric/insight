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

/// Shared admin-gate errors (401 no caller / 403 not admin), used by every
/// admin-gated endpoint via [`crate::api::gate`].
#[resource_error("gts.cf.insight.identity_resolution.access.v1~")]
pub struct AccessError;

#[resource_error("gts.cf.insight.identity_resolution.role.v1~")]
pub struct RoleError;

#[resource_error("gts.cf.insight.identity_resolution.person_role.v1~")]
pub struct PersonRoleError;

#[resource_error("gts.cf.insight.identity_resolution.visibility.v1~")]
pub struct VisibilityError;

#[resource_error("gts.cf.insight.identity_resolution.subchart.v1~")]
pub struct SubchartError;
