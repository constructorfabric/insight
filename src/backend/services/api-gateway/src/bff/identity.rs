//! Map a validated ID token to the internal identity fields the BFF
//! stores on the session.
//!
//! ## Model
//!
//! - **One OIDC issuer per installation.** No cross-issuer collisions are
//!   possible by deployment design, so we treat the OIDC `sub` claim as
//!   the internal `user_id` directly. No hashing, no derivation.
//! - **Identity Service is out of scope for this milestone.** When it
//!   lands, it will accept queries shaped like `(scope="oidc", id=<sub>)`
//!   and return cross-references to other identity sources. At that
//!   point we'll add a resolver call here; the persisted `idp_iss` and
//!   `idp_sub` on every session record give us everything we need.
//! - `tenant_id` comes from config (single-tenant).
//! - `email` and `display_name` come from the validated ID token claims.

/// Identity payload stitched onto a session at login.
#[derive(Debug, Clone)]
pub struct ResolvedIdentity {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub tenant_id: String,
}

/// Build a `ResolvedIdentity` from validated OIDC claims plus a tenant
/// pulled from config. `user_id` is the OIDC `sub` verbatim.
#[must_use]
pub fn resolve(
    sub: &str,
    email: Option<&str>,
    display_name: Option<&str>,
    tenant_id: &str,
) -> ResolvedIdentity {
    ResolvedIdentity {
        user_id: sub.to_owned(),
        email: email.unwrap_or("").to_owned(),
        display_name: display_name.unwrap_or("").to_owned(),
        tenant_id: tenant_id.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_id_is_the_oidc_sub_verbatim() {
        let r = resolve("00uA1b2C3d", None, None, "t-1");
        assert_eq!(r.user_id, "00uA1b2C3d");
    }

    #[test]
    fn resolve_carries_through_tenant_and_claims() {
        let r = resolve(
            "sub-1",
            Some("alice@example.com"),
            Some("Alice"),
            "tenant-1",
        );
        assert_eq!(r.user_id, "sub-1");
        assert_eq!(r.email, "alice@example.com");
        assert_eq!(r.display_name, "Alice");
        assert_eq!(r.tenant_id, "tenant-1");
    }

    #[test]
    fn resolve_handles_missing_claims() {
        let r = resolve("sub", None, None, "tenant-1");
        assert_eq!(r.email, "");
        assert_eq!(r.display_name, "");
    }
}
