//! Auth-event audit emitter (`cpt-insightspec-nfr-bff-audit`).
//!
//! v1 emits structured tracing events. A future iteration will wire a
//! Redpanda producer behind the same `emit` call so consumers don't need
//! to change.

use serde::Serialize;

/// Auth event kind. Stable strings; renaming a variant means breaking the
/// audit topic, which we want to know about explicitly.
///
/// Variants beyond `LoginStart`/`LoginOk`/`LoginFail` are reserved for
/// Phase 2/3 (refresh, logout, sessions, back-channel). They're declared
/// here so the audit envelope shape doesn't shift when those handlers
/// land.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum AuthEventKind {
    LoginStart,
    LoginOk,
    LoginFail,
    SessionRefresh,
    Logout,
    RevokeSingle,
    RevokeAll,
    RevokeAdmin,
    BackChannelLogout,
}

impl AuthEventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::LoginStart => "login_start",
            Self::LoginOk => "login_ok",
            Self::LoginFail => "login_fail",
            Self::SessionRefresh => "session_refresh",
            Self::Logout => "logout",
            Self::RevokeSingle => "revoke_single",
            Self::RevokeAll => "revoke_all",
            Self::RevokeAdmin => "revoke_admin",
            Self::BackChannelLogout => "back_channel_logout",
        }
    }
}

/// Optional context for an audit event. All fields are optional so a
/// caller can include only what it knows.
#[derive(Debug, Default, Clone, Serialize)]
pub struct AuthEvent<'a> {
    pub user_id: Option<&'a str>,
    pub tenant_id: Option<&'a str>,
    pub session_id_hash: Option<&'a str>,
    pub idp_iss: Option<&'a str>,
    pub idp_sub: Option<&'a str>,
    pub ip: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub correlation_id: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Emit a structured audit event.
///
/// Never log raw cookies, tokens, or session IDs — pass the SHA-256 hex
/// prefix via `session_id_hash` instead.
pub fn emit(kind: AuthEventKind, ev: &AuthEvent<'_>) {
    tracing::info!(
        target: "audit.auth",
        event = kind.as_str(),
        user_id = ev.user_id.unwrap_or(""),
        tenant_id = ev.tenant_id.unwrap_or(""),
        session_id_hash = ev.session_id_hash.unwrap_or(""),
        idp_iss = ev.idp_iss.unwrap_or(""),
        idp_sub = ev.idp_sub.unwrap_or(""),
        ip = ev.ip.unwrap_or(""),
        user_agent = ev.user_agent.unwrap_or(""),
        correlation_id = ev.correlation_id.unwrap_or(""),
        reason = ev.reason.unwrap_or(""),
        "auth event",
    );
}

/// Hash a session ID for audit logs. Truncated SHA-256 hex (16 chars,
/// 64 bits) — enough to correlate events for the same session, far short
/// of being a usable cookie.
#[must_use]
pub fn hash_session_id(sid: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(sid.as_bytes());
    let digest = h.finalize();
    hex::encode(&digest[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_session_id_is_stable_and_short() {
        let a = hash_session_id("opaque-abc");
        let b = hash_session_id("opaque-abc");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn hash_session_id_distinguishes_inputs() {
        assert_ne!(hash_session_id("a"), hash_session_id("b"));
    }

    #[test]
    fn event_kind_strings_are_snake_case() {
        assert_eq!(AuthEventKind::LoginOk.as_str(), "login_ok");
        assert_eq!(
            AuthEventKind::BackChannelLogout.as_str(),
            "back_channel_logout"
        );
    }
}
