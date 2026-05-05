//! HTTP handlers for `/auth/*`. Each handler is sync-thin: it pulls
//! request-scoped data, calls the BFF service layer, and returns a
//! `Response`. Long-lived state (Redis client, OIDC client, config)
//! flows in via `Arc<BffState>`.

pub mod callback;
pub mod login;
pub mod me;

use std::sync::Arc;

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

/// Compute jittered `refresh_at = expires_at - safety_margin + uniform(±jitter/2)`.
///
/// Pulled out so handlers and tests can call it without dragging in the
/// rest of the state.
#[must_use]
pub fn jittered_refresh_at(
    expires_at: i64,
    safety_margin_seconds: u64,
    jitter_window_seconds: u64,
) -> i64 {
    use rand::Rng;

    let base = expires_at.saturating_sub(i64::try_from(safety_margin_seconds).unwrap_or(i64::MAX));
    if jitter_window_seconds == 0 {
        return base;
    }
    let half = i64::try_from(jitter_window_seconds / 2).unwrap_or(i64::MAX);
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
    fn jitter_clamps_overflowing_safety_margin_without_panic() {
        // Safety margin > i64::MAX is clamped to i64::MAX before subtraction;
        // the saturating subtraction never overflows, never panics. We
        // don't care about the exact value — only that it's far in the
        // past so the SPA refreshes immediately.
        let r = jittered_refresh_at(10, u64::MAX, 0);
        assert!(r < 0, "expected far-past refresh_at, got {r}");
    }
}
