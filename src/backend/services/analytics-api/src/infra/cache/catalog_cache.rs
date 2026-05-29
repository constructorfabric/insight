//! Catalog cache layer (`cpt-metric-cat-component-cache-layer`).
//!
//! Server-side cache front for `POST /catalog/get_metrics` per DESIGN §3.2 /
//! §3.6 (`cpt-metric-cat-seq-catalog-read`). Carries the read-latency hit-path
//! NFR and the cross-replica invalidation NFR.
//!
//! ## Storage shape: one Redis hash per tenant
//!
//! Each tenant's cache lives in a single Redis hash:
//!
//! - **Hash key**: `cat:v1:{tenant_id}` (lowercase hyphenated UUID).
//! - **Field name**: `{role_slug_or_empty}:{team_id_or_empty}`, every segment
//!   URL-safe-encoded (percent-encode any byte outside `[A-Za-z0-9_-]`). The
//!   empty-string sentinel for absent role/team matches the DB UNIQUE-composite
//!   convention from #519. A hostile `role_slug = "a:b"` becomes `a%3Ab` and
//!   cannot bleed into the team segment.
//! - **Field value**: JSON-encoded [`CatalogResponse`].
//! - **TTL**: per-hash (whole tenant), refreshed on every write via `EXPIRE`.
//!   Per-field TTL would need `HEXPIRE` (Redis 7.4+); per-hash is simpler,
//!   matches the "admin write invalidates the tenant atomically" mental model,
//!   and lets a hot tenant keep its cache warm indefinitely while a quiet one
//!   expires cleanly.
//!
//! Trade-off versus a key-per-entry layout (`cat:v1:{tenant}:{role}:{team}`)
//! + `SCAN cat:v1:{tenant}:* + UNLINK`:
//!
//! - **Tenant invalidation is now O(1)** — a single `UNLINK cat:v1:{tenant}`
//!   instead of an iterator-cursor loop over per-entry keys, and atomic
//!   (no race window for concurrent writes to land between `SCAN` cursor calls).
//! - **Better memory packing** — Redis encodes small hashes as `listpack`s
//!   below `hash-max-listpack-entries` (default 128). A tenant with ≤ 128
//!   distinct `(role, team)` combinations pays one allocation, not one per
//!   entry.
//! - `flush_all()` still uses `SCAN cat:v1:* + UNLINK`, but now enumerates
//!   tenant hashes (a small number) rather than per-entry keys (potentially
//!   thousands).
//!
//! DESIGN §3.2's swap-ability OQ (§4 γ) explicitly allows changing the cache
//! mechanism behind the trait — the public surface
//! (`get`/`put`/`invalidate`/`flush_all`) is unchanged.
//!
//! ## Invalidation contract
//!
//! - [`InvalidateMode::Standard`] — `UNLINK cat:v1:{tenant}`. NEVER `FLUSHDB`
//!   (the Redis instance is shared with `person_aliases:*` from Identity
//!   Resolution; FLUSHDB would clobber sibling namespaces).
//! - [`InvalidateMode::Lock`] — same hash UNLINK, **plus** opens a 5 s
//!   synchronous-resolver-bypass window for that tenant. Sized at
//!   `2 × cpt-metric-cat-nfr-cross-replica-invalidation`-p99 (= 2 × 2 s).
//!   While the window is open, [`CatalogCache::should_skip`] returns `true`
//!   and the reader skips the cache entirely — closing the stale-policy gap
//!   on compliance-critical lock writes during the broadcast window.
//!
//! ## Defense-in-depth on hydrate
//!
//! Cached payloads carry `tenant_id`; the cache re-asserts it on read against
//! the requesting tenant. A mismatch is a backend-misconfiguration smell, not
//! a normal failure mode — it logs a security warning and returns a miss so
//! the resolver repopulates from authoritative state.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use redis::AsyncCommands;
use uuid::Uuid;

use crate::domain::catalog::response::CatalogResponse;

/// Canonical Redis-key prefix for the catalog cache. Every catalog entry's
/// key is `cat:v1:{tenant_id}:{role_slug_or_empty}:{team_id_or_empty}` per
/// DESIGN §3.2; this constant is the prefix the seed-migration flush and
/// admin-write invalidations both walk.
pub const CACHE_KEY_PREFIX: &str = "cat:v1:";

/// Default per-entry TTL — internal to this module. PRD §5.3
/// `cpt-metric-cat-fr-cache` mandates 5 minutes; admin writes invalidate
/// ahead of TTL so users don't observe "I changed the threshold, nothing
/// happened" stale-read gap. The constant is `pub(super)` only so the
/// live-test module in this `infra/cache/` directory can verify TTL
/// behavior; nothing outside the cache layer should care.
pub(super) const DEFAULT_TTL: Duration = Duration::from_mins(5);

/// Lock-event synchronous-bypass window — `2 ×` the cross-replica-invalidation
/// NFR p99 (2 s) per DESIGN §3.2 cache-layer.
pub const LOCK_BYPASS_WINDOW: Duration = Duration::from_secs(5);

/// Invalidation mode. `Lock` writes additionally open the bypass window;
/// every other admin write uses `Standard`.
///
/// `#[allow(dead_code)]` on `Standard` until admin-crud (#525) lands and
/// starts calling `invalidate(tenant, Standard)` on threshold writes. The
/// `Lock` variant is exercised by the tests in this module and `reader.rs`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidateMode {
    Standard,
    Lock,
}

/// Catalog cache trait — abstracts the Redis (or future pub-sub) backend so
/// the reader and admin paths don't depend on a concrete client.
///
/// **Cache-key shape is the cache's internal contract.** Callers pass the
/// request context — `(tenant_id, role_slug, team_id)` — and the cache
/// derives the key. Surfacing the key would force every caller to agree on
/// the encoding, and a future change to the key shape (e.g., a new segment
/// for a request flag) would ripple to every call site. Hiding the key
/// behind the trait keeps the encoding change a single-file refactor.
#[async_trait]
pub trait CatalogCache: Send + Sync {
    /// Fetch a cached response for `(tenant_id, role_slug, team_id)`. The
    /// cache re-asserts the embedded `tenant_id` against `tenant_id` on
    /// hydrate; a mismatch logs a security warning, drops the offending
    /// entry, and returns `None` (cache miss). Returns `Ok(None)` on miss;
    /// surfaces connection / decode failures as `Err`.
    ///
    /// `None` and `Some("")` for `role_slug` / `team_id` are equivalent
    /// (canonical empty-string sentinel) — the same convention as the DB
    /// UNIQUE composite from #519.
    ///
    /// # Errors
    ///
    /// Surfaces Redis transport and JSON decode failures. Callers MUST treat
    /// errors as "miss + degrade gracefully" — a Redis blip MUST NOT 5xx the
    /// read endpoint.
    async fn get(
        &self,
        tenant_id: Uuid,
        role_slug: Option<&str>,
        team_id: Option<&str>,
    ) -> anyhow::Result<Option<CatalogResponse>>;

    /// Store a response under `(tenant_id, role_slug, team_id)` using the
    /// cache's configured TTL (5 minutes per
    /// `cpt-metric-cat-fr-cache`). Callers do NOT pass a TTL — the cache
    /// owns the policy.
    ///
    /// `tenant_id` in the key MUST match `payload.tenant_id`; the cache
    /// uses `payload.tenant_id` as the authoritative tenant for the
    /// embedded-tenant-id round trip.
    ///
    /// # Errors
    ///
    /// Surfaces Redis transport / encode failures. As with [`Self::get`],
    /// callers treat errors as "skip cache; serve resolver result anyway".
    async fn put(
        &self,
        tenant_id: Uuid,
        role_slug: Option<&str>,
        team_id: Option<&str>,
        payload: &CatalogResponse,
    ) -> anyhow::Result<()>;

    /// Drop the tenant's entire catalog cache. With the per-tenant-hash
    /// storage shape (see module docs), this is a single atomic
    /// `UNLINK cat:v1:{tenant_id}` — no cursor walk, no per-entry purge.
    ///
    /// `mode` controls whether to *also* arm the synchronous-resolver-bypass
    /// window:
    ///
    /// - [`InvalidateMode::Standard`] — UNLINK only. Used for ordinary
    ///   threshold edits where the only thing at stake is "the next read
    ///   should see the new value", which the cache miss + resolver fetch
    ///   already guarantees.
    /// - [`InvalidateMode::Lock`] — UNLINK **plus** flip the
    ///   [`Self::should_skip`] flag for this tenant for [`LOCK_BYPASS_WINDOW`]
    ///   (≈ 5 s = 2× cross-replica-invalidation NFR). The reader then
    ///   bypasses the cache entirely for that tenant — both reads and
    ///   writes — for the duration. This closes the
    ///   read-resolve-then-put-stale race: a concurrent reader that
    ///   already loaded a pre-lock response from the DB but hasn't yet
    ///   `put` it into the cache will be told to skip the put, so the
    ///   stale payload never lands on a now-cleared cache. Reserved for
    ///   compliance-critical lock-set / lock-cleared transitions, where
    ///   serving a stale pre-lock policy for even a few hundred
    ///   milliseconds is a regulatory red flag.
    ///
    /// First call site is admin-crud (#525); the trait method is
    /// exercised by tests today, so the `dead_code` annotation only
    /// applies until that PR lands.
    ///
    /// # Errors
    ///
    /// Surfaces Redis transport failures.
    #[allow(dead_code)]
    async fn invalidate(&self, tenant_id: Uuid, mode: InvalidateMode) -> anyhow::Result<()>;

    /// Purge every key under [`CACHE_KEY_PREFIX`]. Used by the seed migration
    /// at boot and by `analytics-api migrate` after applying migrations.
    ///
    /// # Errors
    ///
    /// Surfaces Redis transport failures.
    async fn flush_all(&self) -> anyhow::Result<()>;

    /// True when the lock-bypass window for `tenant_id` is still open and the
    /// reader MUST skip the cache for that tenant. Synchronous and lock-free
    /// on the hot read path.
    fn should_skip(&self, tenant_id: Uuid) -> bool;
}

/// Build the per-tenant Redis hash key — internal to this module. Always
/// `cat:v1:{tenant_id}` with the UUID rendered as 36 lowercase hyphenated
/// chars (deterministic regardless of how the caller spelled the UUID).
#[must_use]
fn tenant_hash_key(tenant_id: Uuid) -> String {
    format!(
        "{CACHE_KEY_PREFIX}{tenant_id}",
        tenant_id = tenant_id.hyphenated()
    )
}

/// Build the field name inside a tenant's hash. `None` and `Some("")` map
/// to the same field (canonical empty-string sentinel — see module docs).
/// Both segments are URL-safe-encoded so a hostile `role_slug = "a:b"`
/// cannot bleed into the team segment.
#[must_use]
fn cache_field(role_slug: Option<&str>, team_id: Option<&str>) -> String {
    let role = url_safe_encode(role_slug.unwrap_or(""));
    let team = url_safe_encode(team_id.unwrap_or(""));
    format!("{role}:{team}")
}

/// Percent-encode bytes outside the URL-safe whitelist `[A-Za-z0-9_\-]`.
/// Empty input round-trips to empty output. Output is ASCII-only so the
/// composed key is always a valid Redis key string.
fn url_safe_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        let b = *byte;
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
            out.push(b as char);
        } else {
            // Uppercase hex per RFC 3986 §2.1 ("hexadecimal digits ... uppercase
            // recommended"). Lowercase would still be valid but mixing cases
            // across call sites is the silent-cache-miss footgun this constant
            // prevents.
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0F) as usize] as char);
        }
    }
    out
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

// ── Skip-until map (lock-bypass window) ──────────────────────────────────

/// Shared in-process map of "tenant → bypass-until instant". Reads are cheap
/// (the lock is held only across a single `HashMap::get`). Mutations occur on
/// admin writes (rare), so contention is bounded.
#[derive(Default)]
struct SkipUntilMap {
    inner: Mutex<HashMap<Uuid, Instant>>,
}

impl SkipUntilMap {
    fn arm(&self, tenant_id: Uuid, until: Instant) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                // Mutex poisoning is a panic-in-handler smell, not a normal
                // condition. Take the inner map anyway so the bypass map
                // doesn't permanently degrade after one panic — the lock is
                // semantically a "latest-write-wins" register, no invariant
                // is corrupted by a poison.
                tracing::warn!(
                    "catalog_cache: skip-until mutex was poisoned; \
                     reusing inner map (no invariant lost)"
                );
                poisoned.into_inner()
            }
        };
        guard.insert(tenant_id, until);
    }

    fn is_armed(&self, tenant_id: Uuid) -> bool {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard
            .get(&tenant_id)
            .copied()
            .is_some_and(|until| Instant::now() < until)
    }
}

// ── Redis-backed implementation ──────────────────────────────────────────

/// Production cache layer. Uses a `redis::aio::ConnectionManager` (auto-reconnect)
/// so transient Redis blips don't cascade into request errors.
pub struct RedisCatalogCache {
    conn: redis::aio::ConnectionManager,
    skip_until: SkipUntilMap,
}

impl RedisCatalogCache {
    /// Connect with `redis::aio::ConnectionManager` so reconnects are handled
    /// transparently. Caller passes a `redis://...` URL from `cfg.redis_url`.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is malformed or the initial connection
    /// fails. Callers that get an error should fall back to [`NoopCatalogCache`]
    /// rather than refusing to boot — single-replica dev installs need to
    /// keep working without Redis.
    pub async fn connect(redis_url: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let conn = client.get_connection_manager().await?;
        Ok(Self {
            conn,
            skip_until: SkipUntilMap::default(),
        })
    }

    /// `SCAN MATCH pattern + UNLINK` — used only by `flush_all`, which has
    /// to enumerate tenant hashes. NEVER `KEYS`, NEVER `FLUSHDB`. `UNLINK`
    /// is preferred over `DEL` because it is asynchronous on the server
    /// side and won't block large purges.
    ///
    /// Per-tenant invalidation does NOT use this — it's a single
    /// `UNLINK cat:v1:{tenant}` against the hash key (see `invalidate`).
    async fn scan_and_unlink(&self, pattern: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let mut cursor: u64 = 0;
        loop {
            let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await?;
            if !batch.is_empty() {
                let _: i64 = redis::cmd("UNLINK")
                    .arg(&batch)
                    .query_async(&mut conn)
                    .await?;
            }
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl CatalogCache for RedisCatalogCache {
    async fn get(
        &self,
        tenant_id: Uuid,
        role_slug: Option<&str>,
        team_id: Option<&str>,
    ) -> anyhow::Result<Option<CatalogResponse>> {
        let hash_key = tenant_hash_key(tenant_id);
        let field = cache_field(role_slug, team_id);
        let mut conn = self.conn.clone();
        // `HGET` returns the field value as bytes (or nil → None).
        let raw: Option<Vec<u8>> = conn.hget(&hash_key, &field).await?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        match serde_json::from_slice::<CatalogResponse>(&raw) {
            Ok(payload) => {
                if payload.tenant_id == tenant_id {
                    Ok(Some(payload))
                } else {
                    // Defense in depth — the hash key already encodes tenant;
                    // a mismatch on the embedded `tenant_id` means either a
                    // backend misconfig or an attacker-controlled collision.
                    // Drop the field, log, and serve a miss so the resolver
                    // repopulates authoritative state. `HDEL` is field-scoped
                    // — sibling fields (other (role, team) combinations for
                    // this tenant) are untouched.
                    tracing::warn!(
                        hash_key = %hash_key,
                        field = %field,
                        embedded_tenant = %payload.tenant_id,
                        requesting_tenant = %tenant_id,
                        "catalog_cache: tenant_id mismatch on hydrate; \
                         dropping cached field and forcing miss"
                    );
                    let _: Result<i64, _> = conn.hdel::<_, _, i64>(&hash_key, &field).await;
                    Ok(None)
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    hash_key = %hash_key,
                    field = %field,
                    "catalog_cache: decode failed; forcing miss"
                );
                let _: Result<i64, _> = conn.hdel::<_, _, i64>(&hash_key, &field).await;
                Ok(None)
            }
        }
    }

    async fn put(
        &self,
        tenant_id: Uuid,
        role_slug: Option<&str>,
        team_id: Option<&str>,
        payload: &CatalogResponse,
    ) -> anyhow::Result<()> {
        let hash_key = tenant_hash_key(tenant_id);
        let field = cache_field(role_slug, team_id);
        let mut conn = self.conn.clone();
        let bytes = serde_json::to_vec(payload)?;
        // `HSET` has no TTL of its own. Refresh the per-hash TTL with
        // `EXPIRE` after every write so an actively-read tenant keeps its
        // cache warm; a quiet tenant lets its hash expire cleanly. Both
        // operations ship in a single pipelined round-trip.
        let secs: u64 = DEFAULT_TTL.as_secs();
        let _: ((), bool) = redis::pipe()
            .atomic()
            .hset(&hash_key, &field, bytes)
            .expire(&hash_key, i64::try_from(secs).unwrap_or(i64::MAX))
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    async fn invalidate(&self, tenant_id: Uuid, mode: InvalidateMode) -> anyhow::Result<()> {
        // O(1) tenant-wide purge: drop the whole hash in one command.
        // `UNLINK` (async-server-side delete) avoids blocking on a large
        // hash. No `SCAN` cursor needed — the entire tenant's cache lives
        // under one key.
        let hash_key = tenant_hash_key(tenant_id);
        let mut conn = self.conn.clone();
        let _: i64 = conn.unlink(&hash_key).await?;
        if mode == InvalidateMode::Lock {
            self.skip_until
                .arm(tenant_id, Instant::now() + LOCK_BYPASS_WINDOW);
        }
        Ok(())
    }

    async fn flush_all(&self) -> anyhow::Result<()> {
        // `cat:v1:*` — NEVER `FLUSHDB`. The Redis instance is shared with
        // sibling namespaces and a global flush would clobber them.
        self.scan_and_unlink(&format!("{CACHE_KEY_PREFIX}*")).await
    }

    fn should_skip(&self, tenant_id: Uuid) -> bool {
        self.skip_until.is_armed(tenant_id)
    }
}

// ── No-op implementation ─────────────────────────────────────────────────

/// No-op cache. Used when `cfg.redis_url` is unset (single-replica dev) and
/// also as the test default — `get` always returns miss, `put`/`invalidate`/
/// `flush_all` are no-ops, and `should_skip` is always `false`.
///
/// The skip-until map is still populated on `invalidate(Lock)` so unit tests
/// can exercise the bypass-window contract without standing up Redis.
#[derive(Default)]
pub struct NoopCatalogCache {
    skip_until: SkipUntilMap,
}

#[async_trait]
impl CatalogCache for NoopCatalogCache {
    async fn get(
        &self,
        _tenant_id: Uuid,
        _role_slug: Option<&str>,
        _team_id: Option<&str>,
    ) -> anyhow::Result<Option<CatalogResponse>> {
        Ok(None)
    }

    async fn put(
        &self,
        _tenant_id: Uuid,
        _role_slug: Option<&str>,
        _team_id: Option<&str>,
        _payload: &CatalogResponse,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn invalidate(&self, tenant_id: Uuid, mode: InvalidateMode) -> anyhow::Result<()> {
        if mode == InvalidateMode::Lock {
            self.skip_until
                .arm(tenant_id, Instant::now() + LOCK_BYPASS_WINDOW);
        }
        Ok(())
    }

    async fn flush_all(&self) -> anyhow::Result<()> {
        tracing::info!(
            prefix = CACHE_KEY_PREFIX,
            "catalog_cache: flush_all called on no-op stub"
        );
        Ok(())
    }

    fn should_skip(&self, tenant_id: Uuid) -> bool {
        self.skip_until.is_armed(tenant_id)
    }
}

#[cfg(test)]
mod tests {
    //! Pure-unit coverage. Live Redis tests live in `live_tests.rs`.

    use super::*;
    use chrono::Utc;

    const T1: Uuid = Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111_u128);
    const T2: Uuid = Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222_u128);

    // ── Tenant hash key + field shape ──────────────────────────────────

    #[test]
    fn prefix_is_cat_v1_colon() {
        // Cross-component pin: tenant hash keys, admin invalidate, and
        // `flush_all` all derive from this constant. A typo silently desyncs.
        assert_eq!(CACHE_KEY_PREFIX, "cat:v1:");
    }

    #[test]
    fn tenant_hash_key_is_prefix_plus_hyphenated_uuid() {
        // The hash key carries only the tenant — every (role, team)
        // combination lives as a field inside that hash. Renders the UUID
        // lowercase hyphenated, deterministic regardless of input casing.
        assert_eq!(
            tenant_hash_key(T1),
            "cat:v1:11111111-1111-1111-1111-111111111111"
        );
    }

    #[test]
    fn cache_field_includes_role_and_team_in_canonical_order() {
        // The field is `{role}:{team}` with both segments URL-safe-encoded.
        // No tenant component here — the tenant is the hash key, not the
        // field.
        assert_eq!(cache_field(Some("eng"), Some("alpha")), "eng:alpha");
    }

    #[test]
    fn none_and_empty_produce_same_field() {
        // Determinism gate per DESIGN §3.2: the empty-string sentinel for
        // `role_slug` / `team_id` produces the SAME field regardless of
        // whether the caller passed `None` or `Some("")`. A regression
        // here is a silent cache miss on every request whose other side
        // spells it the other way.
        let none_field = cache_field(None, None);
        let empty_field = cache_field(Some(""), Some(""));
        assert_eq!(none_field, empty_field);
        assert_eq!(none_field, ":");
    }

    #[test]
    fn none_role_only_collapses_to_empty_segment() {
        assert_eq!(cache_field(None, Some("alpha")), ":alpha");
    }

    #[test]
    fn role_with_colon_is_percent_encoded_cannot_collide_with_team() {
        // Hostile / accidental `role_slug = "a:b"` MUST percent-encode to
        // `a%3Ab` so it cannot bleed into the team-segment position. Without
        // encoding, `role = "a"` + `team = "b"` would produce the SAME field
        // as `role = "a:b"` + `team = ""` — a cross-context collision.
        let a_b_team = cache_field(Some("a"), Some("b"));
        let role_colon = cache_field(Some("a:b"), Some(""));
        assert_ne!(a_b_team, role_colon, "colon in role MUST encode");
        assert_eq!(role_colon, "a%3Ab:");
    }

    #[test]
    fn role_with_percent_is_percent_encoded() {
        // `%` itself is not in the whitelist — must encode to `%25` to avoid
        // ambiguity with an already-encoded segment.
        assert_eq!(cache_field(Some("100%"), Some("")), "100%25:");
    }

    #[test]
    fn allowed_chars_pass_through_unchanged() {
        // The whitelist `[A-Za-z0-9_\-]` must round-trip cleanly so the
        // common case (alphanumeric role/team identifiers) doesn't pay
        // encoding overhead in the hot path.
        assert_eq!(
            cache_field(Some("eng_lead-1"), Some("Team_42")),
            "eng_lead-1:Team_42"
        );
    }

    #[test]
    fn url_safe_encode_uppercase_hex() {
        // Pin the hex case — mixing cases across call sites would create
        // silent cache misses where the encoder on the read path and the
        // write path disagree. RFC 3986 §2.1 recommends uppercase.
        assert_eq!(url_safe_encode(":"), "%3A");
        assert_eq!(url_safe_encode("/"), "%2F");
    }

    #[test]
    fn url_safe_encode_high_bytes() {
        // Non-ASCII bytes (UTF-8 multi-byte sequences) must each encode.
        // `ñ` = 0xC3 0xB1.
        assert_eq!(url_safe_encode("ñ"), "%C3%B1");
    }

    // ── Skip-until map (lock-bypass window) ─────────────────────────────

    #[tokio::test]
    async fn noop_invalidate_lock_arms_skip_window() {
        let cache = NoopCatalogCache::default();
        assert!(!cache.should_skip(T1));
        cache
            .invalidate(T1, InvalidateMode::Lock)
            .await
            .unwrap_or_else(|e| panic!("noop invalidate must succeed: {e}"));
        assert!(
            cache.should_skip(T1),
            "lock invalidate MUST arm skip window"
        );
        // Other tenants are not affected by T1's window.
        assert!(!cache.should_skip(T2));
    }

    #[tokio::test]
    async fn noop_invalidate_standard_does_not_arm_skip_window() {
        let cache = NoopCatalogCache::default();
        cache
            .invalidate(T1, InvalidateMode::Standard)
            .await
            .unwrap_or_else(|e| panic!("noop invalidate must succeed: {e}"));
        assert!(
            !cache.should_skip(T1),
            "standard invalidate MUST NOT arm the skip window — \
             the window is reserved for compliance-critical lock writes"
        );
    }

    #[tokio::test]
    async fn noop_get_always_misses_and_put_is_noop() {
        let cache = NoopCatalogCache::default();
        let miss = cache
            .get(T1, None, None)
            .await
            .unwrap_or_else(|e| panic!("noop get must succeed: {e}"));
        assert!(miss.is_none(), "no-op cache MUST always miss on get");

        let payload = CatalogResponse {
            tenant_id: T1,
            generated_at: Utc::now(),
            metrics: vec![],
        };
        cache
            .put(T1, None, None, &payload)
            .await
            .unwrap_or_else(|e| panic!("noop put must succeed: {e}"));
        // Still misses after put — that's the contract.
        let still_miss = cache
            .get(T1, None, None)
            .await
            .unwrap_or_else(|e| panic!("noop get must succeed: {e}"));
        assert!(still_miss.is_none());
    }

    #[tokio::test]
    async fn noop_flush_is_ok() {
        let cache = NoopCatalogCache::default();
        cache
            .flush_all()
            .await
            .unwrap_or_else(|e| panic!("no-op must never error: {e}"));
    }

    // The 5 s window decay (`should_skip(T1) == false` after sleeping past
    // LOCK_BYPASS_WINDOW) is exercised by the live-Redis tests rather than
    // here so unit tests don't pay 5 s of wall-clock per `cargo test` run.
    // Coverage of the time-bounded property still lives in the test suite —
    // see `infra/cache/live_tests.rs::lock_bypass_window_expires`.
}
