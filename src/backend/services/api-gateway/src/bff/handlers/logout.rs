//! `POST /auth/logout` — local revocation + RP-initiated OIDC end-session.
//!
//! Idempotent. A request with no cookie, an expired cookie, or a cookie
//! that maps to no session all return `200 {end_session_url: null}` with
//! `Set-Cookie __Host-sid; Max-Age=0`. The SPA can always treat the call
//! as terminal: the local session is gone, navigate to `end_session_url`
//! when non-null, otherwise to its own login or root page.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::bff::audit::{AuthEvent, AuthEventKind, hash_session_id};
use crate::bff::cookies::{build_clear_session, read_session_cookie};
use crate::bff::errors::BffError;
use crate::bff::handlers::BffState;

/// Body returned on every `/auth/logout`. `end_session_url` is `null` when
/// the IdP did not advertise an `end_session_endpoint` in its discovery
/// doc, or when we have no `id_token_hint` to supply (session already
/// gone). The SPA must handle both shapes.
#[derive(Debug, Serialize)]
pub struct LogoutResponse {
    pub end_session_url: Option<String>,
}

pub async fn logout(
    State(state): State<Arc<BffState>>,
    headers: HeaderMap,
) -> Result<Response, BffError> {
    let st = state;

    let cookie_sid = read_session_cookie(headers.get(header::COOKIE));

    // Load the session record before revocation so we can mint the
    // RP-initiated logout URL with `id_token_hint`. A missing record is
    // fine — we just emit a no-token-hint logout.
    let record = match cookie_sid.as_deref() {
        Some(sid) => st.store.get_session(sid).await?,
        None => None,
    };

    let end_session_url = match record.as_ref() {
        Some(r) if !r.id_token.is_empty() => {
            st.oidc.end_session_url(&r.id_token, &st.cfg.public_origin)
        }
        _ => None,
    };

    if let Some(sid) = cookie_sid.as_deref() {
        // Idempotent for missing sessions; the inner pipeline drops every
        // index pointer + invalidates the Router JWT cache.
        st.store.revoke_session(sid).await?;
    }

    crate::bff::audit::emit(
        AuthEventKind::Logout,
        &AuthEvent {
            user_id: record.as_ref().map(|r| r.user_id.as_str()),
            tenant_id: record.as_ref().map(|r| r.tenant_id.as_str()),
            session_id_hash: cookie_sid.as_deref().map(hash_session_id).as_deref(),
            idp_iss: record.as_ref().map(|r| r.idp_iss.as_str()),
            idp_sub: record.as_ref().map(|r| r.idp_sub.as_str()),
            ..Default::default()
        },
    );

    let body = serde_json::to_vec(&LogoutResponse { end_session_url })
        .map_err(|e| BffError::Internal(anyhow::anyhow!("serialize: {e}")))?;

    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(body))
        .map_err(|e| BffError::Internal(anyhow::anyhow!("response builder: {e}")))?;
    resp.headers_mut()
        .append(header::SET_COOKIE, build_clear_session());

    Ok(resp.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logout_response_serializes_null_when_no_end_session_url() {
        let body = serde_json::to_value(LogoutResponse {
            end_session_url: None,
        })
        .expect("ser");
        assert!(body.get("end_session_url").expect("field").is_null());
    }

    #[test]
    fn logout_response_serializes_string_when_url_present() {
        let body = serde_json::to_value(LogoutResponse {
            end_session_url: Some("https://idp/logout?id_token_hint=...".into()),
        })
        .expect("ser");
        assert_eq!(
            body["end_session_url"],
            "https://idp/logout?id_token_hint=..."
        );
    }
}
