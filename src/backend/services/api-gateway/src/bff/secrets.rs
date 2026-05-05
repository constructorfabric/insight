//! CSPRNG helpers for opaque session IDs and CSRF tokens.

#![allow(dead_code)]
// `ct_eq` lands in Phase 3 with the CSRF middleware; defined here so the
// crypto primitives live next to the random tokens they protect.
//!
//! All values are URL-safe base64 (no padding) of 32 random bytes — 256 bits
//! of entropy, well above the 128-bit minimum required by
//! `cpt-insightspec-fr-bff-session-cookie`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use rand::rngs::OsRng;

const TOKEN_BYTES: usize = 32;

/// Generate a fresh session ID (CSPRNG, 256 bits entropy).
///
/// Returned string is URL-safe base64 with no padding.
#[must_use]
pub fn new_session_id() -> String {
    new_token(TOKEN_BYTES)
}

/// Generate a fresh CSRF token (CSPRNG, 256 bits entropy).
#[must_use]
pub fn new_csrf_token() -> String {
    new_token(TOKEN_BYTES)
}

/// Generate a fresh OIDC `state` parameter (CSPRNG, 256 bits entropy).
#[must_use]
pub fn new_state() -> String {
    new_token(TOKEN_BYTES)
}

/// Generate a fresh OIDC `nonce` parameter (CSPRNG, 256 bits entropy).
#[must_use]
pub fn new_nonce() -> String {
    new_token(TOKEN_BYTES)
}

fn new_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Constant-time comparison of two CSRF tokens. Both inputs are treated as
/// opaque ASCII strings; equal-length-and-bytes is the only acceptance
/// criterion.
#[must_use]
pub fn ct_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn session_id_is_url_safe_base64_no_pad() {
        let s = new_session_id();
        // 32 bytes → 43 base64 chars (no padding)
        assert_eq!(s.len(), 43);
        assert!(
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert!(!s.contains('='), "URL_SAFE_NO_PAD must not include padding");
    }

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<String> = (0..256).map(|_| new_session_id()).collect();
        assert_eq!(ids.len(), 256, "256 generated IDs must all be distinct");
    }

    #[test]
    fn ct_eq_matches_only_equal_strings() {
        assert!(ct_eq("foo", "foo"));
        assert!(!ct_eq("foo", "fop"));
        assert!(!ct_eq("foo", "fooo"));
        assert!(!ct_eq("", "x"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn token_kinds_are_independent() {
        let a = new_session_id();
        let b = new_csrf_token();
        let c = new_state();
        let d = new_nonce();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(c, d);
    }
}
