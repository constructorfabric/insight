//! `GET /auth/login` — kicks off the OIDC handshake.
//!
//! Generates `state`, `nonce`, and a fresh PKCE pair; persists them in
//! `bff:login_state:{state}` with a 5-minute TTL; redirects the browser
//! to the IdP authorize endpoint.

use std::sync::Arc;

use axum::extract::Query;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::bff::audit::{AuthEvent, AuthEventKind};
use crate::bff::errors::BffError;
use crate::bff::handlers::{BffState, no_store};
use crate::bff::oidc_client::PkcePair;
use crate::bff::secrets::{new_nonce, new_state};
use crate::bff::session_store::login_state;

/// Cap on `return_to` length. Aligns with typical browser URL length caps;
/// not operator-tunable — purely defense in depth.
const RETURN_TO_MAX_LEN: usize = 1024;

#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    /// Path-only target inside the SPA to land on after callback.
    /// Validated for path-only-ness; absolute URLs are rejected.
    #[serde(default)]
    pub return_to: Option<String>,
}

pub async fn login(
    state: axum::extract::State<Arc<BffState>>,
    Query(q): Query<LoginQuery>,
) -> Result<Response, BffError> {
    let st = state.0;

    let return_to = sanitize_return_to(q.return_to.as_deref());
    let oauth_state = new_state();
    let oauth_nonce = new_nonce();
    let pkce = PkcePair::generate();

    let auth_url = st
        .oidc
        .authorize_url(&oauth_state, &oauth_nonce, &pkce.verifier);

    login_state::store(
        &st.redis,
        &oauth_state,
        &login_state::LoginState {
            pkce_verifier: pkce.verifier,
            nonce: oauth_nonce,
            return_to,
        },
    )
    .await?;

    // Bump the global per-pod login-state counter. Phase 3 will check it
    // against the cap; for now we just maintain it.
    let _ = login_state::touch(&st.redis).await;

    crate::bff::audit::emit(
        AuthEventKind::LoginStart,
        &AuthEvent {
            ..Default::default()
        },
    );

    let location = HeaderValue::from_str(&auth_url)
        .map_err(|e| BffError::Internal(anyhow::anyhow!("authorize url not ASCII: {e}")))?;
    Ok((
        StatusCode::FOUND,
        no_store(),
        [(header::LOCATION, location)],
    )
        .into_response())
}

/// Reject anything that could turn into an absolute URL when the browser
/// resolves a `Location:` header. Accepts only path-and-query starting
/// with `/`. Rejects:
///   * absolute URLs (`http://…`, `https://…`)
///   * protocol-relative (`//host/…`)
///   * backslash variants (`/\evil`, `\/evil`) — WHATWG URL parsing
///     normalizes `\` to `/` against special schemes
///   * percent-encoded slash/backslash (`/%2f%2fhost`, `/%5cevil`)
///   * any control character (CR/LF/TAB/etc) — defense in depth
///   * anything above the length cap
///
/// On rejection or absence we fall back to `/`.
pub(super) fn sanitize_return_to(raw: Option<&str>) -> String {
    let raw = raw.unwrap_or("/").trim();
    if raw.is_empty() || !raw.starts_with('/') {
        return "/".to_owned();
    }
    if raw.len() > RETURN_TO_MAX_LEN {
        return "/".to_owned();
    }
    // First two raw chars must be `/` followed by something other than
    // `/` or `\`. Catches `//evil`, `/\evil`, `/\\evil`.
    if let Some(c) = raw.chars().nth(1)
        && (c == '/' || c == '\\')
    {
        return "/".to_owned();
    }
    // Reject any control char or backslash anywhere in the value. A `\`
    // mid-path can still mislead URL parsers; if a real SPA path needs
    // a backslash, it can be percent-encoded *and* we reject `%5c` below.
    // `char::is_control` covers the full Unicode Cc category (C0
    // U+0000–U+001F, DEL U+007F, and C1 U+0080–U+009F).
    if raw.contains('\\') || raw.chars().any(char::is_control) {
        return "/".to_owned();
    }
    // Reject percent-encoded slashes/backslashes that some browsers
    // decode before resolving Location. Case-insensitive match.
    let lower = raw.to_ascii_lowercase();
    if lower.contains("%2f") || lower.contains("%5c") {
        return "/".to_owned();
    }
    raw.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_return_to_accepts_paths() {
        assert_eq!(sanitize_return_to(Some("/dashboard")), "/dashboard");
        assert_eq!(
            sanitize_return_to(Some("/connectors?id=1")),
            "/connectors?id=1"
        );
    }

    #[test]
    fn sanitize_return_to_rejects_absolute_urls() {
        assert_eq!(sanitize_return_to(Some("https://evil.example/")), "/");
        assert_eq!(sanitize_return_to(Some("http://evil/")), "/");
    }

    #[test]
    fn sanitize_return_to_rejects_protocol_relative() {
        assert_eq!(sanitize_return_to(Some("//evil/path")), "/");
    }

    #[test]
    fn sanitize_return_to_handles_empty_and_none() {
        assert_eq!(sanitize_return_to(None), "/");
        assert_eq!(sanitize_return_to(Some("")), "/");
        assert_eq!(sanitize_return_to(Some("   ")), "/");
    }

    #[test]
    fn sanitize_return_to_caps_length() {
        let long = format!("/{}", "a".repeat(RETURN_TO_MAX_LEN));
        assert_eq!(sanitize_return_to(Some(&long)), "/");
    }

    #[test]
    fn sanitize_return_to_rejects_backslash_variants() {
        // `/\evil.com` — browser may resolve as https://evil.com/
        assert_eq!(sanitize_return_to(Some("/\\evil.com")), "/");
        assert_eq!(sanitize_return_to(Some("/\\\\evil.com")), "/");
        // backslash mid-path is also rejected
        assert_eq!(sanitize_return_to(Some("/path\\evil")), "/");
    }

    #[test]
    fn sanitize_return_to_rejects_percent_encoded_slash() {
        assert_eq!(sanitize_return_to(Some("/%2f%2fevil.com")), "/");
        assert_eq!(sanitize_return_to(Some("/%2F%2Fevil.com")), "/");
        assert_eq!(sanitize_return_to(Some("/%5cevil.com")), "/");
        assert_eq!(sanitize_return_to(Some("/%5Cevil.com")), "/");
    }

    #[test]
    fn sanitize_return_to_rejects_control_chars() {
        assert_eq!(sanitize_return_to(Some("/foo\r\nLocation: x")), "/");
        assert_eq!(sanitize_return_to(Some("/foo\tbar")), "/");
        assert_eq!(sanitize_return_to(Some("/foo\u{0000}")), "/");
    }

    #[test]
    fn sanitize_return_to_rejects_del_and_c1_controls() {
        // DEL (U+007F) was missed by the prior `(c as u32) < 0x20` check.
        assert_eq!(sanitize_return_to(Some("/foo\u{007f}bar")), "/");
        // NEL (U+0085), a C1 control.
        assert_eq!(sanitize_return_to(Some("/foo\u{0085}bar")), "/");
        // APC (U+009F), C1.
        assert_eq!(sanitize_return_to(Some("/foo\u{009f}bar")), "/");
    }
}
