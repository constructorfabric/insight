//! `POST /auth/refresh` — rolling cookie rotation per DD-BFF-10.
//!
//! Behaviour:
//!   * No cookie → 401 + clear cookie.
//!   * Cookie maps to live session → mint a new SID, rotate atomically,
//!     return `{expires_at, refresh_at}` with the new cookie.
//!   * Cookie maps to a just-rotated SID (grace window) → return the
//!     current SID and its expiry without rotating again.
//!   * Cookie missing in both `bff:session:*` and `bff:swap:*`, or past
//!     the absolute cap → 401 + clear cookie.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::bff::audit::{AuthEvent, AuthEventKind, hash_session_id};
use crate::bff::cookies::{build_clear_session, build_set_session, read_session_cookie};
use crate::bff::errors::BffError;
use crate::bff::handlers::{BffState, jittered_refresh_at};
use crate::bff::session_store::{RefreshSessionRequest, RefreshSessionResult};

/// JSON body returned on a successful `/auth/refresh`. The SPA schedules
/// `setTimeout(refresh, refresh_at - now)`; `expires_at` is informational.
#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub expires_at: i64,
    pub refresh_at: i64,
}

pub async fn refresh(
    State(state): State<Arc<BffState>>,
    headers: HeaderMap,
) -> Result<Response, BffError> {
    let st = state;

    let Some(old_sid) = read_session_cookie(headers.get(header::COOKIE)) else {
        return Ok(unauthorized_clear_cookie());
    };

    let now = unix_now();
    let result = st
        .store
        .refresh_session(RefreshSessionRequest {
            old_sid: &old_sid,
            now,
            session_ttl_seconds: st.cfg.session.ttl_seconds,
            refresh_grace_ms: st.cfg.session.refresh_grace_ms,
        })
        .await?;

    let outcome = match result {
        RefreshSessionResult::Ok(o) => o,
        RefreshSessionResult::Gone => return Ok(unauthorized_clear_cookie()),
    };

    // `Max-Age` is the residual lifetime, not the configured TTL — on the
    // grace path it's whatever the winner left on the rotated session,
    // which may already have ticked down by a few hundred ms. If the
    // residual hits zero (or below) we'd issue `Max-Age=0` alongside a
    // 200 — that tells the browser to evict the cookie while we report
    // success. Treat as Gone instead so the SPA's contract holds: a 200
    // means it has a usable cookie.
    let max_age = outcome.expires_at - now;
    if max_age <= 0 {
        return Ok(unauthorized_clear_cookie());
    }

    let refresh_at = jittered_refresh_at(
        outcome.expires_at,
        st.cfg.session.refresh_safety_margin_seconds,
        st.cfg.session.refresh_jitter_seconds,
    );

    let set_cookie = build_set_session(&outcome.new_sid, max_age);

    crate::bff::audit::emit(
        AuthEventKind::SessionRefresh,
        &AuthEvent {
            user_id: Some(&outcome.user_id),
            session_id_hash: Some(&hash_session_id(&outcome.new_sid)),
            reason: Some(if outcome.graced { "grace" } else { "rotated" }),
            ..Default::default()
        },
    );

    let body = serde_json::to_vec(&RefreshResponse {
        expires_at: outcome.expires_at,
        refresh_at,
    })
    .map_err(|e| BffError::Internal(anyhow::anyhow!("serialize: {e}")))?;

    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(body))
        .map_err(|e| BffError::Internal(anyhow::anyhow!("response builder: {e}")))?;
    resp.headers_mut().append(header::SET_COOKIE, set_cookie);

    Ok(resp.into_response())
}

fn unauthorized_clear_cookie() -> Response {
    let mut resp = (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({
            "type": "urn:insight:error:unauthorized",
            "title": "Unauthorized",
            "status": 401,
            "detail": "no session"
        })),
    )
        .into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/problem+json"),
    );
    resp.headers_mut()
        .append(header::SET_COOKIE, build_clear_session());
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    resp
}

fn unix_now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn unauthorized_clear_cookie_carries_clear_set_cookie_and_problem_ct() {
        let resp = unauthorized_clear_cookie();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).expect("ct"),
            "application/problem+json"
        );
        let sc = resp
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("set-cookie");
        assert!(sc.contains("Max-Age=0"));
        assert!(sc.contains("__Host-sid="));
        assert!(sc.contains("SameSite=Strict"));
        assert!(sc.contains("HttpOnly"));
        assert!(sc.contains("Secure"));

        let bytes = to_bytes(resp.into_body(), 4096).await.expect("body");
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(v["status"], 401);
        assert_eq!(v["type"], "urn:insight:error:unauthorized");
    }

    #[test]
    fn refresh_response_serializes_only_the_two_documented_fields() {
        // DD-BFF-07: the SPA contract pins these field names. Any drift
        // here is an API break; this snapshot prevents accidental ones.
        let body = serde_json::to_value(RefreshResponse {
            expires_at: 1_714_320_120,
            refresh_at: 1_714_320_060,
        })
        .expect("ser");
        let obj = body.as_object().expect("object");
        assert_eq!(obj.len(), 2, "exactly 2 fields, not a partial SessionView");
        assert_eq!(obj["expires_at"], 1_714_320_120);
        assert_eq!(obj["refresh_at"], 1_714_320_060);
    }
}
