use anyhow::Context as _;
use redis::aio::ConnectionManager;

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

#[derive(Debug, Clone, Copy)]
pub struct BucketSpec {
    pub burst: u32,
    pub per_minute: u32,
}

impl BucketSpec {
    fn refill_per_second(self) -> f64 {
        f64::from(self.per_minute) / 60.0
    }

    fn ttl_seconds(self) -> u64 {
        if self.per_minute == 0 {
            return 3600;
        }
        (u64::from(self.burst) * 60)
            .div_ceil(u64::from(self.per_minute))
            .max(60)
    }
}

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
    let digest = toolkit_security::crypto::hash::Sha256::digest(key.as_bytes());
    let key_hex = base16(&digest);
    let mut conn = conn.clone();
    let script = redis::Script::new(TOKEN_BUCKET_LUA);
    let allowed: i64 = script
        .key(format!("{{asm}}:rl:{class}:{key_hex}"))
        .arg(spec.burst)
        .arg(spec.refill_per_second())
        .arg(now)
        .arg(spec.ttl_seconds())
        .invoke_async(&mut conn)
        .await
        .context("token bucket take")?;
    Ok(allowed == 1)
}

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
        let spec = BucketSpec {
            burst: 5,
            per_minute: 6,
        };
        assert_eq!(spec.ttl_seconds(), 60);
        let spec = BucketSpec {
            burst: 60,
            per_minute: 6,
        };
        assert_eq!(spec.ttl_seconds(), 600);
    }
}
