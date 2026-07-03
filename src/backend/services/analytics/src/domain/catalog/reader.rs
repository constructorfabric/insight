//! `catalog-reader` (`cpt-metric-cat-component-catalog-reader`).
//!
//! Orchestrates the read path per DESIGN §3.6:
//!
//! 1. If the tenant has the lock-bypass window armed
//!    (post-`invalidate(mode=lock)`), skip the cache entirely.
//! 2. Else ask the cache for `(tenant_id, role_slug, team_id)`; on hit,
//!    return; on miss or stale-tenant-mismatch, fall through.
//! 3. Call the resolver (single bulk SQL + in-memory walk).
//! 4. Populate the cache (best-effort — Redis blip MUST NOT 5xx).
//! 5. Return the serializable [`CatalogResponse`].
//!
//! The reader never builds cache keys or picks TTLs — those are internal
//! to the cache layer. We pass the request context (`tenant_id`, `role_slug`,
//! `team_id`) and let the cache decide both.

use std::sync::Arc;

use uuid::Uuid;

use crate::domain::catalog::resolver::ThresholdResolver;
use crate::domain::catalog::response::CatalogResponse;
use crate::infra::cache::catalog_cache::CatalogCache;

/// Catalog reader. Cheap to clone — internally `Arc`s the cache + resolver.
#[derive(Clone)]
pub struct CatalogReader {
    cache: Arc<dyn CatalogCache>,
    resolver: ThresholdResolver,
}

impl CatalogReader {
    #[must_use]
    pub fn new(cache: Arc<dyn CatalogCache>, resolver: ThresholdResolver) -> Self {
        Self { cache, resolver }
    }

    /// Run the read path for one request.
    ///
    /// `role_slug` / `team_id` are passed straight through to the cache-key
    /// builder and the resolver, which both apply the canonical empty-string
    /// sentinel — `None` and `Some("")` are equivalent on both layers.
    ///
    /// # Errors
    ///
    /// Surfaces resolver / DB errors. Cache failures are downgraded to
    /// degraded-mode warnings here and never propagate — the reader's
    /// contract is "always return a fresh resolver result rather than 5xx
    /// on a Redis blip".
    pub async fn read(
        &self,
        tenant_id: Uuid,
        role_slug: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<CatalogResponse, sea_orm::DbErr> {
        // Lock-event 5 s synchronous-bypass window per DESIGN §3.2: when the
        // cache was invalidated with `mode = lock`, the reader skips the
        // cache for that tenant for `2 × cross_replica_invalidation_p99` s.
        // Closes the stale-pre-lock-policy gap during the broadcast window.
        let bypass = self.cache.should_skip(tenant_id);

        if bypass {
            tracing::debug!(
                tenant_id = %tenant_id,
                "catalog_reader: lock-bypass window armed; skipping cache"
            );
        } else {
            match self.cache.get(tenant_id, role_slug, team_id).await {
                Ok(Some(hit)) => {
                    tracing::debug!(tenant_id = %tenant_id, "catalog_reader: cache hit");
                    return Ok(hit);
                }
                Ok(None) => {} // miss — fall through to resolver
                Err(e) => {
                    // Treat cache-layer errors as miss + degrade. The
                    // resolver is authoritative; serving a 500 because Redis
                    // blipped would be a worse user experience than a slower
                    // request.
                    tracing::warn!(
                        error = %e,
                        tenant_id = %tenant_id,
                        "catalog_reader: cache get failed; degrading to resolver"
                    );
                }
            }
        }

        // Resolver fallback (cache miss OR lock-bypass). Apply the same
        // empty-string sentinel the cache uses internally, so the bulk SQL
        // binds match what a follow-up `cache.get` would have looked for.
        let role = role_slug.unwrap_or("");
        let team = team_id.unwrap_or("");
        let response = self.resolver.resolve(tenant_id, role, team).await?;

        // Best-effort cache populate. Errors are logged + ignored — the user
        // still gets the resolver result this request, and the next read
        // either hits a healthier cache or pays one more miss.
        //
        // We skip the populate when the bypass window is armed: writing into
        // the cache during the bypass would re-cache the very payload the
        // bypass exists to suppress. The next non-bypassed read repopulates
        // naturally.
        if !bypass
            && let Err(e) = self
                .cache
                .put(tenant_id, role_slug, team_id, &response)
                .await
        {
            tracing::warn!(
                error = %e,
                tenant_id = %tenant_id,
                "catalog_reader: cache put failed; serving resolver result anyway"
            );
        }

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    //! Pure-Rust reader tests using a counting in-memory cache stub. DB-backed
    //! resolver behavior is exercised in `resolver`'s own tests + the
    //! live-MariaDB integration tests.

    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;
    use crate::infra::cache::catalog_cache::{InvalidateMode, LOCK_BYPASS_WINDOW};

    const T1: Uuid = Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111_u128);
    const T2: Uuid = Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222_u128);

    /// Cache-key tuple for the in-memory store. The reader passes the same
    /// `(tenant_id, role_slug, team_id)` triple to both `get` and `put`, so a
    /// tuple key models the contract correctly. We canonicalize `None` and
    /// `Some("")` to the same `""` on store so the unit tests confirm the
    /// reader doesn't accidentally pass `Some("")` to one method and `None`
    /// to the other.
    type Key = (Uuid, String, String);

    fn key_of(tenant: Uuid, role: Option<&str>, team: Option<&str>) -> Key {
        (
            tenant,
            role.unwrap_or("").to_owned(),
            team.unwrap_or("").to_owned(),
        )
    }

    /// In-memory cache that counts get/put/invalidate calls so the reader's
    /// short-circuit + bypass behavior is observable without standing up Redis.
    #[derive(Default)]
    struct CountingCache {
        store: Mutex<HashMap<Key, CatalogResponse>>,
        skip_until: Mutex<HashMap<Uuid, std::time::Instant>>,
        get_count: AtomicUsize,
        put_count: AtomicUsize,
    }

    impl CountingCache {
        fn put_now(&self, k: Key, payload: CatalogResponse) {
            let mut g = self
                .store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            g.insert(k, payload);
        }

        fn arm_skip(&self, tenant_id: Uuid) {
            let mut g = self
                .skip_until
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            g.insert(tenant_id, std::time::Instant::now() + LOCK_BYPASS_WINDOW);
        }
    }

    #[async_trait]
    impl CatalogCache for CountingCache {
        async fn get(
            &self,
            tenant_id: Uuid,
            role_slug: Option<&str>,
            team_id: Option<&str>,
        ) -> anyhow::Result<Option<CatalogResponse>> {
            self.get_count.fetch_add(1, Ordering::SeqCst);
            let g = self
                .store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(payload) = g.get(&key_of(tenant_id, role_slug, team_id)).cloned() else {
                return Ok(None);
            };
            if payload.tenant_id != tenant_id {
                // Same cross-tenant guard the real cache implements — return
                // a miss so the resolver repopulates with the correct tenant.
                tracing::warn!("test counting cache: tenant mismatch hydrate");
                return Ok(None);
            }
            Ok(Some(payload))
        }

        async fn put(
            &self,
            tenant_id: Uuid,
            role_slug: Option<&str>,
            team_id: Option<&str>,
            payload: &CatalogResponse,
        ) -> anyhow::Result<()> {
            self.put_count.fetch_add(1, Ordering::SeqCst);
            self.put_now(key_of(tenant_id, role_slug, team_id), payload.clone());
            Ok(())
        }

        async fn invalidate(&self, tenant_id: Uuid, mode: InvalidateMode) -> anyhow::Result<()> {
            // Tenant-prefix purge: remove every entry whose tenant matches.
            // Matches the production `SCAN cat:v1:{tenant}:* + UNLINK`.
            let mut g = self
                .store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            g.retain(|(t, _, _), _| *t != tenant_id);
            drop(g);
            if mode == InvalidateMode::Lock {
                self.arm_skip(tenant_id);
            }
            Ok(())
        }

        async fn flush_all(&self) -> anyhow::Result<()> {
            let mut g = self
                .store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            g.clear();
            Ok(())
        }

        fn should_skip(&self, tenant_id: Uuid) -> bool {
            let g = self
                .skip_until
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            g.get(&tenant_id)
                .copied()
                .is_some_and(|until| std::time::Instant::now() < until)
        }
    }

    /// `ThresholdResolver` against `DatabaseConnection::Disconnected`. Any
    /// query call on a Disconnected variant fails — sea-orm 1.1.x panics in
    /// the spawned task, future versions may return `Err(DbErr)`.
    /// [`assert_resolver_was_reached`] treats BOTH signals as "the resolver
    /// path was reached" so the test contract is robust to either behavior.
    fn placeholder_resolver() -> ThresholdResolver {
        ThresholdResolver::new(sea_orm::DatabaseConnection::Disconnected)
    }

    fn sample_payload(tenant_id: Uuid) -> CatalogResponse {
        CatalogResponse {
            tenant_id,
            generated_at: Utc::now(),
            metrics: vec![],
            links: vec![],
        }
    }

    #[tokio::test]
    async fn cache_hit_short_circuits_resolver() -> Result<(), sea_orm::DbErr> {
        // Definition-of-done #4: integration test that cache hit short-circuits
        // the DB call. We don't need a real DB to verify this: the
        // disconnected `ThresholdResolver` would error on any SQL — so a
        // green test proves the resolver wasn't reached.
        let cache = Arc::new(CountingCache::default());
        cache.put_now(key_of(T1, Some("eng"), Some("alpha")), sample_payload(T1));

        let reader = CatalogReader::new(cache.clone(), placeholder_resolver());
        let out = reader.read(T1, Some("eng"), Some("alpha")).await?;

        assert_eq!(out.tenant_id, T1);
        assert_eq!(cache.get_count.load(Ordering::SeqCst), 1);
        // Put MUST NOT happen on a hit.
        assert_eq!(cache.put_count.load(Ordering::SeqCst), 0);
        Ok(())
    }

    /// Drive `reader.read(...)` in a spawned task. The placeholder resolver
    /// is wired to a `Disconnected` SeaORM connection — any query call on it
    /// either panics (sea-orm 1.1.x) or returns `Err(DbErr)` (forward-
    /// compatible). Both signals are "resolver was reached"; only
    /// `Ok(Ok(_))` proves the cache short-circuit served the request without
    /// touching the resolver — which is the regression we want to catch.
    async fn assert_resolver_was_reached(cache: Arc<CountingCache>, tenant: Uuid) -> bool {
        let reader = CatalogReader::new(cache, placeholder_resolver());
        let handle = tokio::spawn(async move { reader.read(tenant, None, None).await });
        match handle.await {
            // Spawned task panicked (sea-orm 1.1.x Disconnected path) OR
            // returned `Err(DbErr)` from the resolver (forward-compatible).
            // Both mean the resolver was reached.
            Err(_) | Ok(Err(_)) => true,
            // Cache short-circuit served the request; resolver was NOT reached.
            Ok(Ok(_)) => false,
        }
    }

    #[tokio::test]
    async fn cross_tenant_cached_payload_forces_resolver_path() {
        // A cached payload whose embedded `tenant_id` is T2 MUST NOT be served
        // to a T1 request. The cache returns None on the mismatch; the reader
        // then goes to the resolver — proving the cross-tenant payload was
        // NOT served. A regression where the wrong-tenant payload IS served
        // would skip the resolver and return `Ok(_)`.
        let cache = Arc::new(CountingCache::default());
        cache.put_now(key_of(T1, None, None), sample_payload(T2));

        let reached_resolver = assert_resolver_was_reached(cache.clone(), T1).await;
        assert!(
            reached_resolver,
            "cross-tenant cache mismatch MUST force a resolver call. \
             A green return here would mean the T2-tagged payload was \
             served on a T1 request — a security regression."
        );
        assert_eq!(
            cache.get_count.load(Ordering::SeqCst),
            1,
            "cache.get must be attempted exactly once"
        );
    }

    #[tokio::test]
    async fn lock_bypass_window_skips_cache_entirely() {
        // After invalidate(Lock), `should_skip(T1)` returns true. The reader
        // MUST NOT call `cache.get` for T1 during the window — that's the
        // whole point of the synchronous-bypass: a stale pre-lock payload
        // returned by a peer replica still in the broadcast window MUST be
        // skipped, not served.
        let cache = Arc::new(CountingCache::default());
        cache.put_now(key_of(T1, None, None), sample_payload(T1));
        cache
            .invalidate(T1, InvalidateMode::Lock)
            .await
            .unwrap_or_else(|e| panic!("counting invalidate must succeed: {e}"));

        let reached_resolver = assert_resolver_was_reached(cache.clone(), T1).await;
        assert!(
            reached_resolver,
            "during the bypass window, reader MUST go straight to resolver \
             (a fresh cache entry was present; a non-bypassed path would \
             have returned it)."
        );
        assert_eq!(
            cache.get_count.load(Ordering::SeqCst),
            0,
            "cache.get MUST NOT be called during the lock-bypass window"
        );
    }

    #[tokio::test]
    async fn standard_invalidate_does_not_open_bypass_window() {
        // Counterpart to the lock-bypass test: a standard admin write (no
        // lock change) MUST NOT trigger the synchronous-bypass. Otherwise
        // every threshold edit would force a resolver round-trip for ~5 s
        // across the deployment — exactly the latency degradation the
        // bypass-only-on-lock contract avoids.
        let cache = Arc::new(CountingCache::default());
        cache.put_now(key_of(T1, None, None), sample_payload(T1));
        cache
            .invalidate(T1, InvalidateMode::Standard)
            .await
            .unwrap_or_else(|e| panic!("counting invalidate must succeed: {e}"));

        // Re-populate the cache after the standard invalidation (the standard
        // invalidate IS a prefix purge — the cache is now empty). We need a
        // hit to demonstrate the bypass window is NOT open.
        cache.put_now(key_of(T1, None, None), sample_payload(T1));

        let reader = CatalogReader::new(cache.clone(), placeholder_resolver());
        let out = reader
            .read(T1, None, None)
            .await
            .unwrap_or_else(|e| panic!("read must succeed on cache hit: {e}"));
        assert_eq!(out.tenant_id, T1);
        assert!(
            !cache.should_skip(T1),
            "standard invalidate MUST NOT arm the bypass window"
        );
    }
}
