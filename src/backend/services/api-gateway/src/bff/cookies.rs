//! Single source of truth for the BFF session cookie.
//!
//! Per `cpt-insightspec-nfr-bff-cookie-attrs` every session-cookie response
//! MUST carry `__Host-` prefix, `HttpOnly`, `Secure`, `SameSite=Strict`,
//! `Path=/`, no `Domain`. We hard-code those attributes here so any other
//! call site has to go through these helpers.

use axum::http::HeaderValue;

/// Name of the BFF session cookie. The `__Host-` prefix tells the browser to:
///   * require Secure
///   * require Path=/
///   * forbid Domain — pinning the cookie to one host
pub const SESSION_COOKIE_NAME: &str = "__Host-sid";

/// Build a `Set-Cookie` header value that establishes a fresh session.
#[must_use]
pub fn build_set_session(value: &str, max_age_seconds: i64) -> HeaderValue {
    cookie_header(value, max_age_seconds)
}

/// Build a `Set-Cookie` header value that clears the session.
#[must_use]
pub fn build_clear_session() -> HeaderValue {
    // Empty value + Max-Age=0 evicts the cookie. We still set every other
    // attribute identically to the live cookie so a buggy browser never
    // ends up with a cookie that has weaker attrs than the original.
    cookie_header("", 0)
}

/// Read the BFF session cookie value from a single `Cookie` header.
///
/// Returns `None` when:
///   * no header is present;
///   * no `__Host-sid` cookie is found;
///   * **more than one** `__Host-sid` cookie is present (RFC 6265bis
///     leaves duplicate handling undefined — we reject to avoid an
///     attacker-set duplicate masking the real cookie).
///
/// Strips an optional pair of surrounding double-quotes so a quoted
/// cookie value `"abc"` matches the unquoted SID we generated.
#[must_use]
pub fn read_session_cookie(cookie_header: Option<&HeaderValue>) -> Option<String> {
    let raw = cookie_header?.to_str().ok()?;
    let mut found: Option<String> = None;
    for pair in raw.split(';') {
        let trimmed = pair.trim();
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        if name.trim() != SESSION_COOKIE_NAME {
            continue;
        }
        let v = unquote(value.trim()).to_owned();
        if found.is_some() {
            // Duplicate __Host-sid in the same Cookie header — refuse
            // rather than guess which one is genuine.
            return None;
        }
        found = Some(v);
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

fn cookie_header(value: &str, max_age_seconds: i64) -> HeaderValue {
    // RFC 6265 cookie attributes. Order is convention; all attributes are
    // mandatory to keep `cpt-insightspec-nfr-bff-cookie-attrs` honest.
    let raw = format!(
        "{SESSION_COOKIE_NAME}={value}; Max-Age={max_age_seconds}; Path=/; Secure; HttpOnly; SameSite=Strict",
    );
    // Inputs are: a fixed ASCII cookie name, an opaque base64url session
    // value (alphanumeric + `-_`) we generate ourselves in `secrets.rs`,
    // and a signed integer. None can introduce non-ASCII bytes; the
    // expect can never fire unless the program is rewritten to pass an
    // attacker-controlled value here.
    #[allow(clippy::expect_used)]
    HeaderValue::from_str(&raw).expect("session cookie header must be ASCII by construction")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_session_header_carries_all_required_attrs() {
        let v = build_set_session("opaque-value", 120);
        let s = v.to_str().expect("ascii");

        assert!(s.starts_with("__Host-sid=opaque-value"), "name and value");
        assert!(s.contains("Max-Age=120"));
        assert!(s.contains("Path=/"));
        assert!(s.contains("Secure"));
        assert!(s.contains("HttpOnly"));
        assert!(s.contains("SameSite=Strict"));
        assert!(!s.contains("Domain="), "no Domain attribute");
    }

    #[test]
    fn clear_header_uses_max_age_zero_with_full_attrs() {
        let v = build_clear_session();
        let s = v.to_str().expect("ascii");

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
        // Hard snapshot — if this changes, somebody touched cookie hardening.
        // Anyone updating this needs to re-run the security review.
        let v = build_set_session("AAAA", 120);
        let s = v.to_str().expect("ascii");
        assert_eq!(
            s,
            "__Host-sid=AAAA; Max-Age=120; Path=/; Secure; HttpOnly; SameSite=Strict"
        );
    }

    #[test]
    fn snapshot_clear_session() {
        let v = build_clear_session();
        let s = v.to_str().expect("ascii");
        assert_eq!(
            s,
            "__Host-sid=; Max-Age=0; Path=/; Secure; HttpOnly; SameSite=Strict"
        );
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
