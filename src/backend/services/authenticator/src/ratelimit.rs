//! Rate limiting, layer 2 (PRD `nfr-auth-rate-limit`, DESIGN §4.4, G8).
//!
//! The gateway's per-IP `limit_req` zone is the coarse flood guard (layer 1);
//! this is the precise, multi-replica-correct layer in Redis:
//!
//! - a **token bucket** (atomic Lua script) keyed by what actually identifies
//!   the caller — the session for `/auth/refresh`, the OIDC `state` for
//!   `/auth/callback`. Never IP: corporate NAT makes per-IP keys wrong at the
//!   precise layer.
//! - a **global live login-state cap** for `/auth/login`: pre-auth there is
//!   no per-caller key, so the guarded resource is the store itself —
//!   `asm:login_state_live` (ZSET, score = expiry) counts live entries and
//!   excess logins get 429 before any state is written, stopping a
//!   slow-trickle Redis-exhaustion attack the edge cannot see.

use anyhow::Context as _;
use redis::aio::ConnectionManager;

/// Atomic token-bucket take. KEYS[1] = bucket; ARGV = capacity,
/// refill-per-second, now (epoch seconds), key TTL. Returns 1 when a token
/// was taken, 0 when the bucket is empty. State is one HASH per key, expiring
/// once idle long enough to refill fully.
const TOKEN_BUCKET_LUA: &str = r"
local data = redis.call('HMGET', KEYS[1], 'tokens', 'ts')
local capacity = tonumber(ARGV[1])
local refill = tonumber(ARGV[2])
local now = tonumber(ARGV[3])
local tokens = tonumber(data[1])
local ts = tonumber(data[2])
if tokens == nil then tokens = capacity end
if ts == nil then ts = now end
-- Never let ts move backwards: on a clock step-back (NTP correction) or
-- multi-pod skew, `now < ts` would otherwise re-add the skipped window next
-- time and over-refill. Clamp to the newest timestamp seen (review L5).
if now < ts then now = ts end
if now > ts then
  tokens = math.min(capacity, tokens + (now - ts) * refill)
end
local allowed = 0
if tokens >= 1 then
  tokens = tokens - 1
  allowed = 1
end
redis.call('HSET', KEYS[1], 'tokens', tokens, 'ts', now)
redis.call('EXPIRE', KEYS[1], tonumber(ARGV[4]))
return allowed
";

/// One bucket class: capacity (burst) + refill rate.
#[derive(Debug, Clone, Copy)]
pub struct BucketSpec {
    pub burst: u32,
    pub per_minute: u32,
}

impl BucketSpec {
    fn refill_per_second(self) -> f64 {
        f64::from(self.per_minute) / 60.0
    }

    /// Key TTL: long enough to refill from empty, floored at one minute.
    fn ttl_seconds(self) -> u64 {
        if self.per_minute == 0 {
            return 3600;
        }
        // Whole seconds to refill `burst` at `per_minute` per minute.
        (u64::from(self.burst) * 60)
            .div_ceil(u64::from(self.per_minute))
            .max(60)
    }
}

/// Take one token from `asm:rl:{class}:{key-digest}`. `Ok(true)` = allowed.
/// A zero/absent spec (burst 0) disables the bucket (always allowed).
///
/// The key component is SHA-256-hashed (hex) before use, so an attacker-chosen
/// `key` (e.g. the OIDC `state` on `/auth/callback`) is bounded to a
/// fixed-width digest — it cannot inflate Redis with arbitrarily long keys or
/// smuggle control characters (review L4).
///
/// # Errors
/// Fails on a Redis error — the caller decides fail-open vs fail-closed.
pub async fn take(
    conn: &ConnectionManager,
    class: &str,
    key: &str,
    spec: BucketSpec,
    now: u64,
) -> anyhow::Result<bool> {
    if spec.burst == 0 {
        return Ok(true);
    }
    let digest = <sha2::Sha256 as sha2::Digest>::digest(key.as_bytes());
    let key_hex = base16(&digest);
    let mut conn = conn.clone();
    let script = redis::Script::new(TOKEN_BUCKET_LUA);
    let allowed: i64 = script
        .key(format!("asm:rl:{class}:{key_hex}"))
        .arg(spec.burst)
        .arg(spec.refill_per_second())
        .arg(now)
        .arg(spec.ttl_seconds())
        .invoke_async(&mut conn)
        .await
        .context("token bucket take")?;
    Ok(allowed == 1)
}

/// Lowercase hex of a byte slice (avoids a hex-crate dependency).
fn base16(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_covers_a_full_refill() {
        // burst 5 at 6/min → refill 0.1/s → 50 s to refill, floored to 60.
        let spec = BucketSpec {
            burst: 5,
            per_minute: 6,
        };
        assert_eq!(spec.ttl_seconds(), 60);
        // burst 60 at 6/min → 600 s.
        let spec = BucketSpec {
            burst: 60,
            per_minute: 6,
        };
        assert_eq!(spec.ttl_seconds(), 600);
    }
}
