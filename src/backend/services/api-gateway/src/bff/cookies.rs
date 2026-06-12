//! Single source of truth for the BFF session cookie.
//!
//! Per `cpt-insightspec-nfr-bff-cookie-attrs` every session-cookie response
//! MUST carry `__Host-` prefix, `HttpOnly`, `Secure`, `SameSite=Strict`,
//! `Path=/`, no `Domain`. We build every `Set-Cookie` here via the typed
//! `cookie` crate (re-exported by `axum-extra`) so the attribute set is
//! constructed structurally rather than concatenated by hand.
//!
//! Handlers don't touch any of that. They compose a tuple response with
//! [`SessionCookie::set`] (after login or refresh) or
//! [`SessionCookie::clear`] (logout, 401 paths), and `IntoResponseParts`
//! does the encoding and `Set-Cookie` insertion.

use std::convert::Infallible;

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponseParts, ResponseParts};
use axum_extra::extract::cookie::{Cookie, SameSite};
use time::Duration;

/// Name of the BFF session cookie. The `__Host-` prefix tells the browser to:
///   * require Secure
///   * require Path=/
///   * forbid Domain — pinning the cookie to one host
pub const SESSION_COOKIE_NAME: &str = "__Host-sid";

/// Typed response-part for the BFF session cookie. Constructed by
/// handlers and composed into a tuple response; `IntoResponseParts`
/// renders the full attribute set and appends a single `Set-Cookie`
/// header. Callers never see `Max-Age` arithmetic or `HeaderName`s.
///
/// ```ignore
/// (StatusCode::OK, no_store(), SessionCookie::set(&sid, expires_at, now), Json(view))
///     .into_response()
/// ```
#[must_use]
pub struct SessionCookie<'a> {
    value: &'a str,
    /// Residual cookie lifetime, clamped to `[0, u32::MAX]` at the
    /// construction boundary. The session's `expires_at - now`
    /// subtraction is naturally `i64`, but the stored result is always
    /// non-negative and bounded above by the absolute session lifetime
    /// (≤ 24h per spec), so `u32` is the right shape — it eliminates the
    /// "is this negative?" question downstream.
    max_age_seconds: u32,
}

impl<'a> SessionCookie<'a> {
    /// Set the cookie to `sid` with `Max-Age = max(expires_at - now, 0)`.
    /// `expires_at` and `now` are both absolute epoch seconds; the helper
    /// turns them into the residual lifetime so callers can pass the
    /// session record's `expires_at` directly without restating the
    /// subtraction.
    pub fn set(sid: &'a str, expires_at: i64, now: i64) -> Self {
        let residual_signed = expires_at.saturating_sub(now).max(0);
        let max_age_seconds = u32::try_from(residual_signed).unwrap_or(u32::MAX);
        Self {
            value: sid,
            max_age_seconds,
        }
    }

    /// Clear the cookie. Empty value + `Max-Age=0`, every other attribute
    /// identical to the live cookie so a buggy browser never ends up with
    /// weaker attrs than the original.
    pub fn clear() -> SessionCookie<'static> {
        SessionCookie {
            value: "",
            max_age_seconds: 0,
        }
    }
}

impl IntoResponseParts for SessionCookie<'_> {
    type Error = Infallible;

    fn into_response_parts(self, mut res: ResponseParts) -> Result<ResponseParts, Self::Error> {
        let value = render(&build_cookie(self.value, self.max_age_seconds));
        res.headers_mut().append(header::SET_COOKIE, value);
        Ok(res)
    }
}

/// Read the BFF session cookie value from a single `Cookie` header.
///
/// Returns `None` when:
///   * no header is present;
///   * no `__Host-sid` cookie is found;
///   * **more than one** `__Host-sid` cookie is present (RFC 6265bis
///     leaves duplicate handling undefined — we reject to avoid an
///     attacker-set duplicate masking the real cookie).
#[must_use]
pub fn read_session_cookie(cookie_header: Option<&HeaderValue>) -> Option<String> {
    let raw = cookie_header?.to_str().ok()?;
    let mut found: Option<String> = None;
    for parsed in Cookie::split_parse(raw) {
        let Ok(c) = parsed else { continue };
        if c.name() != SESSION_COOKIE_NAME {
            continue;
        }
        if found.is_some() {
            // Duplicate `__Host-sid` in the same Cookie header — refuse
            // rather than guess which one is genuine.
            return None;
        }
        // RFC 6265 allows a cookie value to be wrapped in double quotes;
        // the `cookie` crate's parser preserves them verbatim. The
        // SIDs we mint never contain quotes, so stripping a matching
        // outer pair is a safe normalization.
        found = Some(unquote(c.value()).to_owned());
    }
    found
}

fn unquote(s: &str) -> &str {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Construct a `Cookie` carrying the full attribute set mandated by
/// `cpt-insightspec-nfr-bff-cookie-attrs`. Single source of truth for
/// what makes a session cookie valid.
fn build_cookie(value: &str, max_age_seconds: u32) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, value.to_owned()))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(Duration::seconds(i64::from(max_age_seconds)))
        .build()
}

fn render(c: &Cookie<'_>) -> HeaderValue {
    // Inputs are: a fixed ASCII cookie name, an opaque base64url session
    // value (alphanumeric + `-_`) we generate ourselves in `secrets.rs`,
    // and a signed integer Max-Age. None can introduce non-ASCII bytes,
    // and the cookie crate's encoder rejects control characters at parse
    // time, so this expect can only fire if the program is rewritten to
    // pass an attacker-controlled value here.
    #[allow(clippy::expect_used)]
    HeaderValue::from_str(&c.to_string())
        .expect("session cookie header must be ASCII by construction")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip a `SessionCookie` through the axum tuple-response
    /// machinery and return the rendered `Set-Cookie` header string —
    /// exactly what a handler's `(StatusCode, …, SessionCookie::…)`
    /// would emit on the wire.
    fn emit(c: SessionCookie<'_>) -> String {
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        // `(StatusCode, IntoResponseParts, IntoResponse)` is axum's
        // canonical "status + headers + body" shape; the `()` body keeps
        // the test tight without bringing `Body::empty()` into scope.
        let resp = (StatusCode::OK, c, ()).into_response();
        resp.headers()
            .get(header::SET_COOKIE)
            .expect("set-cookie present")
            .to_str()
            .expect("ascii")
            .to_owned()
    }

    #[test]
    fn set_carries_all_required_attrs() {
        let s = emit(SessionCookie::set("opaque-value", 220, 100));
        assert!(
            s.starts_with("__Host-sid=opaque-value"),
            "name + value first"
        );
        assert!(s.contains("Max-Age=120"));
        assert!(s.contains("Path=/"));
        assert!(s.contains("Secure"));
        assert!(s.contains("HttpOnly"));
        assert!(s.contains("SameSite=Strict"));
        assert!(!s.contains("Domain="), "no Domain attribute");
    }

    #[test]
    fn clear_uses_max_age_zero_with_full_attrs() {
        let s = emit(SessionCookie::clear());
        assert!(s.starts_with("__Host-sid="));
        assert!(s.contains("Max-Age=0"));
        assert!(s.contains("Path=/"));
        assert!(s.contains("Secure"));
        assert!(s.contains("HttpOnly"));
        assert!(s.contains("SameSite=Strict"));
        assert!(!s.contains("Domain="));
    }

    #[test]
    fn snapshot_set_session_120s() {
        // Hard snapshot — if this changes, somebody touched cookie hardening
        // (or bumped the `cookie` crate to a version that emits attributes
        // in a different order). Either path needs a fresh security review
        // before the snapshot is updated.
        let s = emit(SessionCookie::set("AAAA", 220, 100));
        assert_eq!(
            s,
            "__Host-sid=AAAA; HttpOnly; SameSite=Strict; Secure; Path=/; Max-Age=120",
        );
    }

    #[test]
    fn snapshot_clear_session() {
        let s = emit(SessionCookie::clear());
        assert_eq!(
            s,
            "__Host-sid=; HttpOnly; SameSite=Strict; Secure; Path=/; Max-Age=0",
        );
    }

    #[test]
    fn set_clamps_negative_residual_to_zero() {
        // `now` past `expires_at` — without the clamp, `Max-Age=-N` would
        // be rendered (cookie crate accepts negative). We floor to 0 so the
        // browser evicts immediately rather than holding a confused cookie.
        let s = emit(SessionCookie::set("AAAA", 100, 500));
        assert!(s.contains("Max-Age=0"));
    }

    #[test]
    fn read_session_cookie_finds_value() {
        let h = HeaderValue::from_static("foo=bar; __Host-sid=abc; baz=qux");
        assert_eq!(read_session_cookie(Some(&h)), Some("abc".to_owned()));
    }

    #[test]
    fn read_session_cookie_returns_none_when_absent() {
        let h = HeaderValue::from_static("foo=bar; baz=qux");
        assert_eq!(read_session_cookie(Some(&h)), None);
        assert_eq!(read_session_cookie(None), None);
    }

    #[test]
    fn read_session_cookie_handles_only_cookie() {
        let h = HeaderValue::from_static("__Host-sid=alone");
        assert_eq!(read_session_cookie(Some(&h)), Some("alone".to_owned()));
    }

    #[test]
    fn read_session_cookie_strips_quoted_value() {
        let h = HeaderValue::from_static("__Host-sid=\"quoted-val\"");
        assert_eq!(read_session_cookie(Some(&h)), Some("quoted-val".to_owned()));
    }

    #[test]
    fn read_session_cookie_rejects_duplicate_session_cookies() {
        // Two `__Host-sid` cookies in the same header — refuse to guess.
        let h = HeaderValue::from_static("__Host-sid=first; __Host-sid=second");
        assert_eq!(read_session_cookie(Some(&h)), None);
    }

    #[test]
    fn read_session_cookie_does_not_match_substring_of_other_cookie() {
        // A cookie whose value mentions `__Host-sid=...` must not be
        // treated as the session cookie.
        let h = HeaderValue::from_static("foo=__Host-sid=evil; bar=baz");
        assert_eq!(read_session_cookie(Some(&h)), None);
    }
}
