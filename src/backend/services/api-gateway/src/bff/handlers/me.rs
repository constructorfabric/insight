//! `GET /auth/me` — bootstrap endpoint for the SPA.
//!
//! Returns `{user, tenant, expires_at, refresh_at, csrf_token}` so the SPA
//! can prime its refresh timer and CSRF cache without an extra round-trip.
//!
//! Returns 401 + clear cookie when:
//!   * no `__Host-sid` cookie is present
//!   * the session is missing in Redis (expired / revoked)
//!   * the session has expired but Redis has not yet evicted it (race)

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::bff::cookies::{build_clear_session, read_session_cookie};
use crate::bff::errors::BffError;
use crate::bff::handlers::{BffState, jittered_refresh_at};
use crate::bff::session::{SessionView, TenantView, UserView};

pub async fn me(
    State(state): State<Arc<BffState>>,
    headers: HeaderMap,
) -> Result<Response, BffError> {
    let st = state;

    let sid = read_session_cookie(headers.get(header::COOKIE))
        .ok_or(BffError::Unauthorized("no session cookie"))?;

    let Some(record) = st.store.get_session(&sid).await? else {
        return Ok(unauthorized_clear_cookie());
    };

    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
    .unwrap_or(0);
    if record.expires_at <= now {
        return Ok(unauthorized_clear_cookie());
    }
    // Hard cap on session lifetime — even if `expires_at` is fine, a
    // session past `absolute_expires_at` MUST NOT be served. Phase 2's
    // /auth/refresh enforces the same cap; mirror it here so /auth/me
    // doesn't silently outlive a refresh boundary.
    if record.absolute_expires_at <= now {
        return Ok(unauthorized_clear_cookie());
    }

    let refresh_at = jittered_refresh_at(
        record.expires_at,
        st.cfg.session.refresh_safety_margin_seconds,
        st.cfg.session.refresh_jitter_seconds,
    );

    let view = SessionView {
        user: UserView {
            user_id: record.user_id,
            email: record.email,
            display_name: record.display_name,
        },
        tenant: TenantView {
            tenant_id: record.tenant_id,
        },
        expires_at: record.expires_at,
        refresh_at,
        csrf_token: record.csrf_token,
    };

    let body = serde_json::to_vec(&view)
        .map_err(|e| BffError::Internal(anyhow::anyhow!("serialize: {e}")))?;
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(body))
        .map_err(|e| BffError::Internal(anyhow::anyhow!("response builder: {e}")))?;

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

        let bytes = to_bytes(resp.into_body(), 4096).await.expect("body");
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(v["status"], 401);
    }
}
