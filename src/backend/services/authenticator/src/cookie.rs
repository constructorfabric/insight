//! The single owner of the session cookie (DESIGN §4.1). Attributes are
//! hard-coded; only `Max-Age` varies. Any other code path setting cookies
//! fails review. A snapshot test pins the exact `Set-Cookie` bytes.

use axum::http::HeaderMap;
use axum::http::header::COOKIE;

/// The session cookie name. `__Host-` forbids `Domain=` and requires `Secure`
/// + `Path=/`, pinning the cookie to one host.
pub const COOKIE_NAME: &str = "__Host-sid";

/// Build the `Set-Cookie` value for a fresh session token with `max_age` TTL.
#[must_use]
pub fn set_session_cookie(token: &str, max_age_seconds: u64) -> String {
    format!(
        "{COOKIE_NAME}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={max_age_seconds}"
    )
}

/// Build the `Set-Cookie` value that clears the session cookie (logout).
#[must_use]
pub fn clear_session_cookie() -> String {
    format!("{COOKIE_NAME}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0")
}

/// Extract the session token from the request `Cookie` header, if present.
#[must_use]
pub fn read_session_token(headers: &HeaderMap) -> Option<String> {
    let prefix = format!("{COOKIE_NAME}=");
    for header in headers.get_all(COOKIE) {
        let raw = header.to_str().ok()?;
        for part in raw.split(';') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix(&prefix)
                && !value.is_empty()
            {
                return Some(value.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn set_cookie_snapshot() {
        // The exact bytes the browser must receive. HttpOnly + Secure +
        // SameSite=Strict + Path=/ + __Host- prefix, no Domain.
        assert_eq!(
            set_session_cookie("tok-abc123", 600),
            "__Host-sid=tok-abc123; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=600"
        );
    }

    #[test]
    fn clear_cookie_snapshot() {
        assert_eq!(
            clear_session_cookie(),
            "__Host-sid=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"
        );
    }

    #[test]
    fn reads_token_among_other_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            "foo=bar; __Host-sid=the-token; baz=qux".parse().unwrap(),
        );
        assert_eq!(read_session_token(&headers).as_deref(), Some("the-token"));
    }

    #[test]
    fn absent_token_is_none() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, "foo=bar".parse().unwrap());
        assert_eq!(read_session_token(&headers), None);
    }
}
