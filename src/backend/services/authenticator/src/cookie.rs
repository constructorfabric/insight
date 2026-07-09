//! The single owner of the session cookie (DESIGN §4.1). Attributes are
//! hard-coded — only `Max-Age` varies. Built on axum-extra's `CookieJar`, so
//! reads (extractor) and writes (Set-Cookie response part) go through the
//! standard `cookie` crate instead of hand-rolled header strings.

use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use time::Duration;

/// The session cookie name. `__Host-` forbids `Domain=` and requires `Secure`
/// + `Path=/`, pinning the cookie to one host.
pub const COOKIE_NAME: &str = "__Host-sid";

/// The hardened session cookie carrying `token`, valid for `max_age_seconds`.
#[must_use]
pub fn session_cookie(token: &str, max_age_seconds: u64) -> Cookie<'static> {
    build(
        token.to_owned(),
        Duration::seconds(i64::try_from(max_age_seconds).unwrap_or(0)),
    )
}

/// A cookie that clears the session (logout): empty value, `Max-Age=0`.
#[must_use]
pub fn clear_cookie() -> Cookie<'static> {
    build(String::new(), Duration::ZERO)
}

/// The session token from the request jar, if present and non-empty.
#[must_use]
pub fn read(jar: &CookieJar) -> Option<String> {
    jar.get(COOKIE_NAME)
        .map(|c| c.value().to_owned())
        .filter(|v| !v.is_empty())
}

fn build(value: String, max_age: Duration) -> Cookie<'static> {
    // HttpOnly keeps it out of JS; SameSite=Strict blocks CSRF; Secure + Path=/
    // + the __Host- prefix (no Domain) pin it to the single gateway host.
    Cookie::build((COOKIE_NAME, value))
        .path("/")
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Strict)
        .max_age(max_age)
        .build()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_has_hardened_attributes() {
        let rendered = session_cookie("tok-abc123", 600).to_string();
        assert!(rendered.starts_with("__Host-sid=tok-abc123"), "{rendered}");
        for attr in [
            "Secure",
            "HttpOnly",
            "SameSite=Strict",
            "Path=/",
            "Max-Age=600",
        ] {
            assert!(rendered.contains(attr), "missing {attr} in: {rendered}");
        }
    }

    #[test]
    fn clear_cookie_expires_immediately() {
        let rendered = clear_cookie().to_string();
        assert!(rendered.starts_with("__Host-sid="), "{rendered}");
        assert!(rendered.contains("Max-Age=0"), "{rendered}");
    }

    #[test]
    fn read_extracts_token() {
        let jar = CookieJar::new().add(Cookie::new(COOKIE_NAME, "the-token"));
        assert_eq!(read(&jar).as_deref(), Some("the-token"));
    }

    #[test]
    fn read_absent_is_none() {
        assert_eq!(read(&CookieJar::new()), None);
    }
}
