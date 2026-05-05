//! `GET /auth/callback` — finishes the OIDC handshake.
//!
//! 1. Look up `bff:login_state:{state}` (atomic GET+DEL).
//! 2. If absent → 400. State mismatch / replay / TTL expired all collapse here.
//! 3. Exchange `code` + `verifier` at the IdP token endpoint.
//! 4. Validate ID token signature + iss/aud/nonce/exp.
//! 5. Resolve internal identity (deterministic user_id from iss+sub,
//!    tenant from config, email/name from validated claims).
//! 6. Read incoming `__Host-sid` cookie — fixation guard revokes it.
//! 7. Create session in Redis (atomic MULTI/EXEC).
//! 8. Set `__Host-sid` cookie + 302 to `return_to`.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::bff::audit::{AuthEvent, AuthEventKind, hash_session_id};
use crate::bff::cookies::{build_set_session, read_session_cookie};
use crate::bff::errors::BffError;
use crate::bff::handlers::BffState;
use crate::bff::identity;
use crate::bff::session_store::{CreateSessionRequest, login_state};

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    /// Set by the IdP if the user denied consent or the IdP errored out.
    pub error: Option<String>,
    pub error_description: Option<String>,
}

pub async fn callback(
    State(state): State<Arc<BffState>>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, BffError> {
    let st = state;

    if let Some(err) = q.error.as_deref() {
        // Log the raw values for debugging, but only echo a normalized
        // OAuth error code back to the user — the IdP-supplied query is
        // attacker-influenceable.
        tracing::warn!(
            err = %err,
            err_desc = %q.error_description.as_deref().unwrap_or(""),
            "OIDC callback returned error",
        );
        let safe = sanitize_oauth_error_code(err);
        crate::bff::audit::emit(
            AuthEventKind::LoginFail,
            &AuthEvent {
                reason: Some(&safe),
                ..Default::default()
            },
        );
        return Err(BffError::Idp(format!("idp error: {safe}")));
    }

    let oauth_state = q
        .state
        .as_deref()
        .ok_or(BffError::BadRequest("missing state"))?;
    let code = q
        .code
        .as_deref()
        .ok_or(BffError::BadRequest("missing code"))?;

    // Atomic take of the login-state record.
    let ls = login_state::take(&st.redis, oauth_state)
        .await?
        .ok_or(BffError::BadRequest("state mismatch or expired"))?;

    let exch = st
        .oidc
        .exchange_code(code, &ls.pkce_verifier, &ls.nonce)
        .await?;

    // Resolve internal identity. user_id == OIDC sub (single issuer per
    // installation). Tenant comes from config (single-tenant). Identity
    // Service mapping lands in a separate scope.
    let resolved = identity::resolve(
        &exch.claims.sub,
        exch.claims.email.as_deref(),
        exch.claims.name.as_deref(),
        &st.cfg.default_tenant_id,
    );

    // Fixation guard input: any incoming __Host-sid is treated as
    // attacker-controlled. We never reuse it.
    let incoming_sid = read_session_cookie(headers.get(header::COOKIE));
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let ip = client_ip_from_headers(&headers);

    let now = unix_now();
    let outcome = st
        .store
        .create_session(CreateSessionRequest {
            user_id: &resolved.user_id,
            tenant_id: &resolved.tenant_id,
            idp_iss: &exch.claims.iss,
            idp_sub: &exch.claims.sub,
            idp_sid: exch.claims.sid.as_deref().unwrap_or(""),
            id_token: &exch.id_token,
            email: &resolved.email,
            display_name: &resolved.display_name,
            user_agent: &user_agent,
            ip: &ip,
            now,
            session_ttl_seconds: st.cfg.session.ttl_seconds,
            absolute_lifetime_seconds: st.cfg.session.absolute_lifetime_seconds,
            incoming_sid: incoming_sid.as_deref(),
        })
        .await?;

    crate::bff::audit::emit(
        AuthEventKind::LoginOk,
        &AuthEvent {
            user_id: Some(&resolved.user_id),
            tenant_id: Some(&resolved.tenant_id),
            session_id_hash: Some(&hash_session_id(&outcome.session_id)),
            idp_iss: Some(&exch.claims.iss),
            idp_sub: Some(&exch.claims.sub),
            ip: Some(&ip),
            user_agent: Some(&user_agent),
            ..Default::default()
        },
    );

    // 302 to return_to + Set-Cookie. Even though /auth/login already
    // sanitized return_to before storing it in Redis, we re-sanitize here
    // — single source of truth, and defense-in-depth against tampered
    // login-state records.
    let return_to = super::login::sanitize_return_to(Some(&ls.return_to));

    let max_age = i64::try_from(st.cfg.session.ttl_seconds).unwrap_or(i64::MAX);
    let set_cookie = build_set_session(&outcome.session_id, max_age);

    let mut resp = Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, return_to)
        .body(axum::body::Body::empty())
        .map_err(|e| BffError::Internal(anyhow::anyhow!("response builder: {e}")))?;
    resp.headers_mut().append(header::SET_COOKIE, set_cookie);
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );

    Ok(resp.into_response())
}

/// Sanitize an OAuth error code echoed from the IdP. RFC 6749 §4.1.2.1
/// defines the error code as `*VSCHAR (%x20-7E)`, but in practice all
/// real codes are short identifiers (`access_denied`, `invalid_request`).
/// We keep only `[a-zA-Z0-9_-]` and cap at 64 chars; anything else
/// collapses to `unknown`.
fn sanitize_oauth_error_code(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "unknown".to_owned();
    }
    let cleaned: String = trimmed
        .chars()
        .take(64)
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if cleaned.is_empty() {
        "unknown".to_owned()
    } else {
        cleaned
    }
}

/// Best-effort source IP. Honors `X-Forwarded-For` (first hop) when the
/// gateway is behind a trusted proxy — the ingress in our deployment.
fn client_ip_from_headers(headers: &HeaderMap) -> String {
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first) = v.split(',').next()
    {
        let trimmed = first.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

fn unix_now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ip_prefers_xff_first_hop() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            "203.0.113.5, 10.0.0.1".parse().expect("hv"),
        );
        assert_eq!(client_ip_from_headers(&h), "203.0.113.5");
    }

    #[test]
    fn client_ip_falls_back_to_x_real_ip() {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", "203.0.113.7".parse().expect("hv"));
        assert_eq!(client_ip_from_headers(&h), "203.0.113.7");
    }

    #[test]
    fn client_ip_empty_when_no_headers() {
        let h = HeaderMap::new();
        assert_eq!(client_ip_from_headers(&h), "");
    }

    #[test]
    fn sanitize_oauth_error_code_keeps_legit_identifiers() {
        assert_eq!(sanitize_oauth_error_code("access_denied"), "access_denied");
        assert_eq!(
            sanitize_oauth_error_code("invalid_request"),
            "invalid_request"
        );
    }

    #[test]
    fn sanitize_oauth_error_code_strips_suspicious_chars() {
        assert_eq!(
            sanitize_oauth_error_code("invalid_request<script>alert(1)</script>"),
            "invalid_requestscriptalert1script"
        );
    }

    #[test]
    fn sanitize_oauth_error_code_caps_length() {
        let long = "a".repeat(200);
        let s = sanitize_oauth_error_code(&long);
        assert_eq!(s.len(), 64);
    }

    #[test]
    fn sanitize_oauth_error_code_handles_empty_and_blank() {
        assert_eq!(sanitize_oauth_error_code(""), "unknown");
        assert_eq!(sanitize_oauth_error_code("   "), "unknown");
        assert_eq!(sanitize_oauth_error_code("@@@"), "unknown");
    }
}
