//! HTTP handlers for `/auth/*`. Each handler is sync-thin: it pulls
//! request-scoped data, calls the BFF service layer, and returns a
//! `Response`. Long-lived state (Redis client, OIDC client, config)
//! flows in via `Arc<BffState>`.

pub mod callback;
pub mod login;
pub mod me;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum_extra::TypedHeader;
use axum_extra::headers::CacheControl;

use crate::bff::config::BffConfig;
use crate::bff::oidc_client::OidcClient;
use crate::bff::session_store::SessionStore;
use crate::redis_client::RedisShared;

/// Shared service state passed into handlers. Cheap to clone — every
/// field is `Arc` or `Clone`.
#[derive(Clone)]
pub struct BffState {
    pub cfg: Arc<BffConfig>,
    pub oidc: Arc<OidcClient>,
    pub store: SessionStore,
    pub redis: Arc<RedisShared>,
}

/// `Cache-Control: no-store` for every `/auth/*` response.
///
/// All BFF responses carry auth-sensitive payloads (cookies, user data,
/// id_token-hint URLs) that browsers, proxies, and shared caches must
/// never store. Typed via `axum-extra` so the header value is built by
/// the `headers` crate rather than hand-formatted in every handler.
pub fn no_store() -> TypedHeader<CacheControl> {
    TypedHeader(CacheControl::new().with_no_store())
}

/// Current UNIX time in seconds, as `i64` for direct use against the
/// `i64` epoch fields stored on `SessionRecord`.
///
/// Panics if the system clock is before `UNIX_EPOCH` (operator-grade
/// invariant violation — JWTs/TLS/cookies are already broken in that
/// state, so loud failure is correct) or after `i64::MAX` seconds
/// (~year 292 billion; not reachable). Bare `panic!` keeps the
/// workspace-wide `unwrap_used`/`expect_used` denies satisfied.
#[must_use]
pub(crate) fn unix_now() -> i64 {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| panic!("system clock is before UNIX_EPOCH"));
    i64::try_from(d.as_secs()).unwrap_or_else(|_| panic!("system clock past i64::MAX seconds"))
}

/// Compute jittered `refresh_at = expires_at - safety_margin + uniform(±jitter/2)`.
///
/// Pulled out so handlers and tests can call it without dragging in the
/// rest of the state.
#[must_use]
pub fn jittered_refresh_at(
    expires_at: i64,
    safety_margin_seconds: u16,
    jitter_window_seconds: u16,
) -> i64 {
    use rand::Rng;

    let base = expires_at.saturating_sub(i64::from(safety_margin_seconds));
    if jitter_window_seconds == 0 {
        return base;
    }
    let half = i64::from(jitter_window_seconds / 2);
    let offset = rand::thread_rng().gen_range(-half..=half);
    base.saturating_add(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_zero_window_returns_base() {
        assert_eq!(jittered_refresh_at(1000, 30, 0), 970);
    }

    #[test]
    fn jitter_stays_within_window() {
        for _ in 0..200 {
            let r = jittered_refresh_at(1000, 30, 10);
            // base = 970, half = 5 → range [965, 975]
            assert!((965..=975).contains(&r), "got {r}");
        }
    }

    #[test]
    fn unix_now_returns_recent_epoch_seconds() {
        // Loosely sanity-check that we're returning seconds, not millis or
        // a stale constant. Anchor against 2024-01-01 (1_704_067_200) so
        // the test stays valid for as long as the CI clock advances.
        let n = unix_now();
        assert!(n > 1_704_067_200, "got {n}");
        assert!(n < 32_503_680_000, "got {n}");
    }

    #[test]
    fn no_store_helper_encodes_to_cache_control_no_store() {
        // Lock the helper's rendered value so any future bump of
        // `axum-extra` / `headers` that changes encoding fails CI
        // before it can silently change `Cache-Control` semantics.
        use axum_extra::headers::Header;
        let tv = no_store();
        let mut values: Vec<axum::http::HeaderValue> = Vec::new();
        tv.0.encode(&mut values);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].to_str().expect("ascii"), "no-store");
    }
}
