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

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::bff::cookies::{SessionCookie, read_session_cookie};
use crate::bff::errors::BffError;
use crate::bff::handlers::{BffState, jittered_refresh_at, no_store, unix_now};
use crate::bff::session::{SessionView, TenantView, UserView};

const NO_SESSION_REASON: &str = "no session";

pub async fn me(
    State(state): State<Arc<BffState>>,
    headers: HeaderMap,
) -> Result<Response, BffError> {
    let st = state;

    let Some(sid) = read_session_cookie(headers.get(header::COOKIE)) else {
        return Ok(unauthorized_clear_cookie());
    };

    let Some(record) = st.store.get_session(&sid).await? else {
        return Ok(unauthorized_clear_cookie());
    };

    let now = unix_now();
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

    Ok((StatusCode::OK, no_store(), Json(view)).into_response())
}

fn unauthorized_clear_cookie() -> Response {
    // Compose: canonical `unauthenticated` envelope (status, problem+json
    // body, type URI, reason) from `BffError`; layered with `no_store()`
    // and `SessionCookie::clear()` so the browser drops the (possibly
    // stale) `__Host-sid` cookie before the SPA redirects to /auth/login.
    (
        no_store(),
        SessionCookie::clear(),
        BffError::Unauthorized(NO_SESSION_REASON),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn unauthorized_clear_cookie_carries_clear_set_cookie_and_canonical_envelope() {
        let resp = unauthorized_clear_cookie();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).expect("ct"),
            "application/problem+json",
        );
        let sc = resp
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("set-cookie");
        assert!(sc.contains("Max-Age=0"));
        assert!(sc.contains("__Host-sid="));
        assert!(sc.contains("SameSite=Strict"));

        let bytes = to_bytes(resp.into_body(), 4096).await.expect("body");
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(v["status"], 401);
        assert_eq!(
            v["type"], "gts://gts.cf.core.errors.err.v1~cf.core.err.unauthenticated.v1~",
            "must use the constructorfabric canonical envelope",
        );
        assert_eq!(v["context"]["reason"], NO_SESSION_REASON);
    }
}
