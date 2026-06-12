//! Session record domain types.
//!
//! See DESIGN §3.7 for the canonical Redis HASH layout. We keep the
//! field names and types tight against the spec so other modules (Router,
//! janitor) can decode without surprises.

use serde::{Deserialize, Serialize};

/// Full session record stored under `bff:session:{sid}` (HASH).
///
/// Field names mirror the DESIGN doc one-for-one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub user_id: String,
    pub tenant_id: String,
    pub idp_iss: String,
    pub idp_sub: String,
    /// OIDC `sid` claim — empty if the IdP did not supply one.
    pub idp_sid: String,
    /// Stored only for use as `id_token_hint` on RP-initiated logout.
    pub id_token: String,
    /// Email pulled from the OIDC `email` claim, surfaced via `/auth/me`.
    pub email: String,
    /// Display name from `name` / `preferred_username` if present.
    pub display_name: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub absolute_expires_at: i64,
    pub user_agent: String,
    pub ip: String,
    pub csrf_token: String,
}

impl SessionRecord {
    /// Encode the record as Redis HASH field/value pairs in stable order.
    /// The order matches the read path in `from_redis_pairs`.
    #[must_use]
    pub fn to_redis_pairs(&self) -> Vec<(&'static str, String)> {
        vec![
            ("user_id", self.user_id.clone()),
            ("tenant_id", self.tenant_id.clone()),
            ("idp_iss", self.idp_iss.clone()),
            ("idp_sub", self.idp_sub.clone()),
            ("idp_sid", self.idp_sid.clone()),
            ("id_token", self.id_token.clone()),
            ("email", self.email.clone()),
            ("display_name", self.display_name.clone()),
            ("created_at", self.created_at.to_string()),
            ("expires_at", self.expires_at.to_string()),
            ("absolute_expires_at", self.absolute_expires_at.to_string()),
            ("user_agent", self.user_agent.clone()),
            ("ip", self.ip.clone()),
            ("csrf_token", self.csrf_token.clone()),
        ]
    }
}

/// Compact summary for `GET /auth/sessions` listings (Phase 2). Defined
/// here so future code shares one struct across the controller and audit.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct SessionSummary {
    pub session_id: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub user_agent: String,
    pub ip: String,
    pub current: bool,
}

/// Result of a successful `/auth/me` or `/auth/refresh` call. The SPA
/// schedules its next refresh from `refresh_at`.
#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    pub user: UserView,
    pub tenant: TenantView,
    pub expires_at: i64,
    pub refresh_at: i64,
    pub csrf_token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserView {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TenantView {
    pub tenant_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionRecord {
        SessionRecord {
            user_id: "u-1".into(),
            tenant_id: "t-1".into(),
            idp_iss: "https://idp/".into(),
            idp_sub: "sub-1".into(),
            idp_sid: "sid-1".into(),
            id_token: "jwt".into(),
            email: "alice@example.com".into(),
            display_name: "Alice".into(),
            created_at: 1,
            expires_at: 121,
            absolute_expires_at: 28_801,
            user_agent: "ua".into(),
            ip: "1.2.3.4".into(),
            csrf_token: "csrf".into(),
        }
    }

    #[test]
    fn pairs_have_stable_field_order() {
        let pairs = sample().to_redis_pairs();
        let names: Vec<&str> = pairs.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            names,
            vec![
                "user_id",
                "tenant_id",
                "idp_iss",
                "idp_sub",
                "idp_sid",
                "id_token",
                "email",
                "display_name",
                "created_at",
                "expires_at",
                "absolute_expires_at",
                "user_agent",
                "ip",
                "csrf_token",
            ]
        );
    }
}
