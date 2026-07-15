//! Resource-scoped canonical error types (toolkit ADR 0005). Domain errors
//! convert into `CanonicalError` (RFC 9457 `Problem` on the wire):
//! `Unauthenticated` -> 401 (the `auth_request` deny), everything else -> 5xx
//! (the gateway fails closed). No custom error enums cross the API boundary.

use toolkit_canonical_errors::resource_error;

/// Failures resolving / creating the request's internal person.
#[resource_error("gts.cf.insight.authenticator.person.v1~")]
pub struct PersonError;

/// OIDC handshake failures (state/nonce/exchange/id_token validation).
#[resource_error("gts.cf.insight.authenticator.oidc.v1~")]
pub struct OidcError;

/// Service-token issuance failures (`POST /internal/token`): unknown/invalid
/// client assertion, replay, or a refused tenant scope.
#[resource_error("gts.cf.insight.authenticator.service_token.v1~")]
pub struct ServiceTokenError;
