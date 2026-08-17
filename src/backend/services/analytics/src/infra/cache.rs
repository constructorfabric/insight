use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use tokio::sync::Semaphore;

/// Values above this are served but not stored, so one pathological view
/// cannot dominate a Redis shared with session state.
const MAX_ENTRY_BYTES: usize = 256 * 1024;
/// Keys per MGET and writes per pipeline. A dashboard request can reference a
/// few thousand fragments; sending them as one command would occupy the shared
/// server for the whole reply, delaying session traffic behind it.
const MAX_KEYS_PER_COMMAND: usize = 256;
/// Budget for one Redis command. Applied per chunk so a large read degrades
/// chunk by chunk instead of discarding everything it already fetched.
const COMMAND_TIMEOUT: Duration = Duration::from_millis(150);
const CONNECT_RETRY: Duration = Duration::from_secs(30);
/// Write-back is optional work, so it is capped rather than queued: past this
/// many concurrent writers a request skips its write instead of adding load to
/// a Redis that also carries session state.
const MAX_CONCURRENT_WRITERS: usize = 8;

/// Read-through storage for metric-result view fragments.
///
/// Every operation fails open: a missing connection, a timeout, or a Redis
/// error degrades to "not cached" instead of an error, because the same Redis
/// carries authenticator sessions and a metric read must never be the reason a
/// login fails.
#[derive(Debug)]
pub struct MetricViewCache {
    conn: OnceLock<ConnectionManager>,
    ttl: Duration,
    writers: Semaphore,
    connect_reported: AtomicBool,
    op_reported: AtomicBool,
}

impl MetricViewCache {
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self::new(Duration::ZERO))
    }

    fn new(ttl: Duration) -> Self {
        Self {
            conn: OnceLock::new(),
            ttl,
            writers: Semaphore::new(MAX_CONCURRENT_WRITERS),
            connect_reported: AtomicBool::new(false),
            op_reported: AtomicBool::new(false),
        }
    }

    /// Never fails: an empty URL disables the cache, and an unreachable Redis
    /// leaves it disabled while a background task keeps trying, so a cold Redis
    /// cannot hold up boot.
    pub fn connect(redis_url: &str, ttl: Duration) -> Arc<Self> {
        if redis_url.trim().is_empty() || ttl.is_zero() {
            return Self::disabled();
        }

        let client = match redis::Client::open(redis_url) {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(error = %error, "metric-results cache URL unusable; serving uncached");
                return Self::disabled();
            }
        };

        let cache = Arc::new(Self::new(ttl));
        tokio::spawn(connect_until_ready(Arc::clone(&cache), client));

        cache
    }

    pub fn enabled(&self) -> bool {
        self.conn.get().is_some()
    }

    /// A chunk that fails or times out yields misses for its own keys only;
    /// everything already fetched is still served.
    pub async fn get_many(&self, keys: &[String]) -> Vec<Option<Vec<u8>>> {
        let mut found = Vec::with_capacity(keys.len());
        if keys.is_empty() {
            return found;
        }
        let Some(conn) = self.conn.get() else {
            return keys.iter().map(|_| None).collect();
        };

        let mut conn = conn.clone();
        for chunk in keys.chunks(MAX_KEYS_PER_COMMAND) {
            let read = conn.mget::<_, Vec<Option<Vec<u8>>>>(chunk);
            let batch = match tokio::time::timeout(COMMAND_TIMEOUT, read).await {
                Ok(Ok(batch)) if batch.len() == chunk.len() => batch,
                Ok(Ok(_)) => {
                    self.report("read", "reply length did not match the requested keys");
                    chunk.iter().map(|_| None).collect()
                }
                Ok(Err(error)) => {
                    self.report("read", &error.to_string());
                    chunk.iter().map(|_| None).collect()
                }
                Err(_) => {
                    self.report("read", "timed out");
                    chunk.iter().map(|_| None).collect()
                }
            };
            found.extend(batch);
        }

        found
    }

    pub async fn set_many(&self, entries: Vec<(String, Vec<u8>)>) {
        let Some(conn) = self.conn.get() else {
            return;
        };
        // INVARIANT: the permit is held across the writes it bounds — that hold
        // is the concurrency cap, not incidental.
        let Ok(_permit) = self.writers.try_acquire() else {
            tracing::debug!("metric-results cache write skipped; writer limit reached");
            return;
        };

        let storable: Vec<(String, Vec<u8>)> = entries
            .into_iter()
            .filter(|(_, value)| value.len() <= MAX_ENTRY_BYTES)
            .collect();
        if storable.is_empty() {
            return;
        }

        let ttl_secs = self.ttl.as_secs();
        let mut conn = conn.clone();
        for chunk in storable.chunks(MAX_KEYS_PER_COMMAND) {
            let mut pipe = redis::pipe();
            for (key, value) in chunk {
                pipe.set_ex::<_, _>(key, value, ttl_secs).ignore();
            }

            let write = pipe.query_async::<()>(&mut conn);
            match tokio::time::timeout(COMMAND_TIMEOUT, write).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    self.report("write", &error.to_string());
                    return;
                }
                Err(_) => {
                    self.report("write", "timed out");
                    return;
                }
            }
        }
    }

    /// One warning per process, then debug: a Redis outage would otherwise emit
    /// a warning per view per request. Read/write failures keep their own flag
    /// so ordinary boot-order connect retries cannot consume it.
    fn report(&self, op: &str, error: &str) {
        Self::report_once(&self.op_reported, op, error);
    }

    fn report_once(reported: &AtomicBool, op: &str, error: &str) {
        if reported.swap(true, Ordering::Relaxed) {
            tracing::debug!(op, error, "metric-results cache unavailable");
            return;
        }
        tracing::warn!(
            op,
            error,
            "metric-results cache unavailable; serving uncached"
        );
    }
}

async fn connect_until_ready(cache: Arc<MetricViewCache>, client: redis::Client) {
    loop {
        match client.get_connection_manager().await {
            Ok(conn) => {
                if cache.conn.set(conn).is_err() {
                    return;
                }
                tracing::info!("metric-results cache connected");
                return;
            }
            Err(error) => {
                MetricViewCache::report_once(
                    &cache.connect_reported,
                    "connect",
                    &error.to_string(),
                );
                tokio::time::sleep(CONNECT_RETRY).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disabled_cache_reads_empty_and_swallows_writes() {
        let cache = MetricViewCache::disabled();

        assert!(!cache.enabled());
        assert_eq!(
            cache.get_many(&["a".to_owned(), "b".to_owned()]).await,
            vec![None, None]
        );
        cache.set_many(vec![("a".to_owned(), vec![1, 2, 3])]).await;
    }

    #[tokio::test]
    async fn empty_url_and_zero_ttl_disable_the_cache() {
        assert!(!MetricViewCache::connect("", Duration::from_mins(1)).enabled());
        assert!(!MetricViewCache::connect("   ", Duration::from_mins(1)).enabled());
        assert!(!MetricViewCache::connect("redis://127.0.0.1:6379", Duration::ZERO).enabled());
    }

    #[tokio::test]
    async fn unreachable_redis_serves_uncached_without_erroring() {
        // Port 1 is never a Redis; the handle must degrade, not fail.
        let cache = MetricViewCache::connect("redis://127.0.0.1:1", Duration::from_mins(1));

        assert!(!cache.enabled());
        assert_eq!(cache.get_many(&["k".to_owned()]).await, vec![None]);
        cache.set_many(vec![("k".to_owned(), vec![0])]).await;
    }

    #[tokio::test]
    async fn malformed_url_disables_without_panicking() {
        let cache = MetricViewCache::connect("not-a-redis-url", Duration::from_mins(1));

        assert!(!cache.enabled());
    }
}
