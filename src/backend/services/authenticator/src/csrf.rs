//! CSRF defense for state-changing `/auth/*` methods (PRD 5.11, DESIGN §4.2).
//!
//! `SameSite=Strict` on the session cookie is the primary defense; this is the
//! second line: `X-CSRF-Token` compared in constant time against the token
//! bound to the session at login, with an `Origin`-allowlist fallback
//! (`csrf_origins`; empty = fail closed, token required). Both failing yields
//! 403. The per-session token is issued at login, fetched via `GET /auth/csrf`,
//! and echoed by `/auth/me`.
//!
//! The back-channel logout endpoint is exempt: it is IdP server-to-server and
//! its credential is the signed `logout_token`, not a browser session.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use axum_extra::extract::cookie::CookieJar;
use sha2::{Digest as _, Sha256};

use crate::api::AppState;
use crate::api::error::SessionError;
use crate::cookie;

/// Paths under `/auth/` that skip CSRF checks: not browser-session-driven.
const EXEMPT_PATHS: &[&str] = &["/auth/oidc/back-channel-logout"];

/// The verdict of the pure check (unit-tested separately from the middleware).
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Pass,
    Forbidden(&'static str),
}

/// Pure CSRF decision for one state-changing request with a live session:
/// header token (constant-time) first, `Origin` allowlist as fallback,
/// fail closed when neither verifies.
fn verdict(
    header_token: Option<&str>,
    session_token: &str,
    origin: Option<&str>,
    allowlist: &[String],
) -> Verdict {
    if let Some(presented) = header_token {
        // Constant-time equality via fixed-size digests — the comparison cost
        // is independent of where the strings first differ.
        let a = Sha256::digest(presented.as_bytes());
        let b = Sha256::digest(session_token.as_bytes());
        if a == b && !session_token.is_empty() {
            return Verdict::Pass;
        }
        return Verdict::Forbidden("csrf_token_mismatch");
    }
    if let Some(origin) = origin
        && allowlist.iter().any(|allowed| allowed == origin)
    {
        return Verdict::Pass;
    }
    Verdict::Forbidden("csrf_token_required")
}

/// Axum middleware over the authenticator's route table. Only state-changing
/// `/auth/*` requests that present a resolvable session are checked — without
/// a session there is nothing to forge (the handler answers 401), and the
/// gateway-facing / well-known surfaces are not browser-state-changing.
pub async fn middleware(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method();
    let path = request.uri().path();
    let state_changing = matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if !state_changing || !path.starts_with("/auth/") || EXEMPT_PATHS.contains(&path) {
        return next.run(request).await;
    }

    let Some(token) = cookie::read(&jar) else {
        return next.run(request).await; // no session → handler 401s
    };
    let record = match state.sessions.resolve_by_token(&token).await {
        Ok(Some((_, record))) => record,
        Ok(None) => return next.run(request).await, // dead session → handler 401s
        Err(e) => {
            // Store down: fail closed like every auth path (503, not a bypass).
            tracing::warn!(error = %e, "csrf: session store unavailable");
            return toolkit_canonical_errors::CanonicalError::service_unavailable()
                .with_detail("session store unavailable")
                .create()
                .into_response();
        }
    };

    let header_token = request
        .headers()
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok());
    let origin = request
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok());

    match verdict(
        header_token,
        &record.csrf_token,
        origin,
        &state.cfg.csrf_origins,
    ) {
        Verdict::Pass => next.run(request).await,
        Verdict::Forbidden(reason) => {
            tracing::warn!(
                target: "audit",
                event = "csrf_rejected",
                person_id = %record.person_id,
                %path,
                reason,
                "state-changing /auth/* request failed CSRF verification"
            );
            SessionError::permission_denied()
                .with_reason(reason)
                .create()
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION_TOKEN: &str = "csrf-abc-123";

    #[test]
    fn matching_header_token_passes() {
        assert_eq!(
            verdict(Some(SESSION_TOKEN), SESSION_TOKEN, None, &[]),
            Verdict::Pass
        );
    }

    #[test]
    fn mismatched_header_token_is_forbidden_even_with_allowed_origin() {
        // A presented-but-wrong token is an attack signal; the Origin fallback
        // must not rescue it.
        let allow = vec!["https://app.example".to_owned()];
        assert_eq!(
            verdict(
                Some("wrong"),
                SESSION_TOKEN,
                Some("https://app.example"),
                &allow
            ),
            Verdict::Forbidden("csrf_token_mismatch")
        );
    }

    #[test]
    fn origin_allowlist_is_the_fallback() {
        let allow = vec!["https://app.example".to_owned()];
        assert_eq!(
            verdict(None, SESSION_TOKEN, Some("https://app.example"), &allow),
            Verdict::Pass
        );
        assert_eq!(
            verdict(None, SESSION_TOKEN, Some("https://evil.example"), &allow),
            Verdict::Forbidden("csrf_token_required")
        );
    }

    #[test]
    fn empty_allowlist_fails_closed() {
        // Default config: no origins → the header token is mandatory.
        assert_eq!(
            verdict(None, SESSION_TOKEN, Some("https://app.example"), &[]),
            Verdict::Forbidden("csrf_token_required")
        );
        assert_eq!(
            verdict(None, SESSION_TOKEN, None, &[]),
            Verdict::Forbidden("csrf_token_required")
        );
    }

    #[test]
    fn empty_session_token_never_matches() {
        // A session with no CSRF token (defensive) must not pass an empty header.
        assert_eq!(
            verdict(Some(""), "", None, &[]),
            Verdict::Forbidden("csrf_token_mismatch")
        );
    }
}
