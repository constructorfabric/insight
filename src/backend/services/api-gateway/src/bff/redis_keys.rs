//! Centralized Redis key builders for the BFF.
//!
//! Every key starts with `bff:` (DD-BFF-04). The Router owns `router:` and
//! the BFF only writes to `router:jwt_cache:*` to invalidate cached JWTs on
//! revoke.
//!
//! Key builders for Phase 2/3 surfaces (`bff:swap:*`, `bff:logout_jti:*`,
//! `bff:lock:janitor`) are declared here so the schema is one place; they
//! aren't called from Phase 1 handlers yet.

#![allow(dead_code)]

/// `bff:session:{sid}` — HASH, full session record.
#[must_use]
pub fn session(sid: &str) -> String {
    format!("bff:session:{sid}")
}

/// `bff:user_sessions:{user_id}` — ZSET, score = `expires_at`.
#[must_use]
pub fn user_sessions(user_id: &str) -> String {
    format!("bff:user_sessions:{user_id}")
}

/// `bff:sid_index:{iss}:{idp_sid}` — SET of local `session_id`s, used to
/// resolve back-channel `logout_token` (iss, sid) → local sessions.
#[must_use]
pub fn sid_index(iss: &str, idp_sid: &str) -> String {
    format!("bff:sid_index:{iss}:{idp_sid}")
}

/// `bff:login_state:{state}` — HASH, PKCE verifier + nonce + return_to.
/// One-shot, 5 min TTL.
#[must_use]
pub fn login_state(state: &str) -> String {
    format!("bff:login_state:{state}")
}

/// `bff:swap:{old_sid}` — STRING → new_sid; refresh grace window
/// (DD-BFF-10), default 250 ms TTL.
#[must_use]
pub fn swap(old_sid: &str) -> String {
    format!("bff:swap:{old_sid}")
}

/// `bff:logout_jti:{iss}:{jti}` — STRING `1`, replay guard for OIDC
/// back-channel logout.
#[must_use]
pub fn logout_jti(iss: &str, jti: &str) -> String {
    format!("bff:logout_jti:{iss}:{jti}")
}

/// `router:jwt_cache:{sid}` — owned by the Router; the BFF only deletes
/// these on revoke / rotate to invalidate cached JWTs.
#[must_use]
pub fn router_jwt_cache(sid: &str) -> String {
    format!("router:jwt_cache:{sid}")
}

/// `bff:lock:janitor` — leader-election lock for the expired-session
/// janitor (Phase 3).
#[must_use]
pub fn janitor_lock() -> &'static str {
    "bff:lock:janitor"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_use_bff_prefix() {
        assert!(session("abc").starts_with("bff:session:"));
        assert!(user_sessions("u1").starts_with("bff:user_sessions:"));
        assert!(sid_index("https://idp/", "s1").starts_with("bff:sid_index:"));
        assert!(login_state("st").starts_with("bff:login_state:"));
        assert!(swap("old").starts_with("bff:swap:"));
        assert!(logout_jti("iss", "j1").starts_with("bff:logout_jti:"));
    }

    #[test]
    fn router_key_uses_router_prefix() {
        assert!(router_jwt_cache("sid").starts_with("router:"));
    }
}
