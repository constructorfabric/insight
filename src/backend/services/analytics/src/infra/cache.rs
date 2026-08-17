use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Values above this are served but not stored, so one pathological view
/// cannot dominate a Redis shared with session state.
const MAX_ENTRY_BYTES: usize = 256 * 1024;
/// Keys per MGET and writes per pipeline. A dashboard request can reference a
/// few thousand fragments; sending them as one command would occupy the shared
/// server for the whole reply, delaying session traffic behind it.
const MAX_KEYS_PER_COMMAND: usize = 256;
/// Bytes per command, alongside the key count: 256 maximum-sized entries would
/// otherwise put ~64 MiB on the wire in one round trip.
const MAX_COMMAND_BYTES: usize = 1024 * 1024;
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
pub(crate) struct MetricViewCache {
    conn: OnceLock<ConnectionManager>,
    ttl: Duration,
    writers: Arc<Semaphore>,
    connect_reported: AtomicBool,
    op_reported: AtomicBool,
}

impl MetricViewCache {
    pub(crate) fn disabled() -> Arc<Self> {
        Arc::new(Self::new(Duration::ZERO))
    }

    fn new(ttl: Duration) -> Self {
        Self {
            conn: OnceLock::new(),
            ttl,
            writers: Arc::new(Semaphore::new(MAX_CONCURRENT_WRITERS)),
            connect_reported: AtomicBool::new(false),
            op_reported: AtomicBool::new(false),
        }
    }

    /// Never fails: an empty URL disables the cache, and an unreachable Redis
    /// leaves it disabled while a background task keeps trying, so a cold Redis
    /// cannot hold up boot.
    pub(crate) fn connect(redis_url: &str, ttl: Duration) -> Arc<Self> {
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

    pub(crate) fn enabled(&self) -> bool {
        self.conn.get().is_some()
    }

    /// A write is admitted before its task is spawned, so a slow Redis cannot
    /// accumulate queued writers each retaining an encoded batch.
    pub(crate) fn try_admit_write(&self) -> Option<OwnedSemaphorePermit> {
        let Ok(permit) = Arc::clone(&self.writers).try_acquire_owned() else {
            tracing::debug!("metric-results cache write skipped; writer limit reached");
            return None;
        };
        Some(permit)
    }

    /// A chunk that fails or times out yields misses for its own keys only;
    /// everything already fetched is still served.
    pub(crate) async fn get_many(&self, keys: &[String]) -> Vec<Option<Vec<u8>>> {
        let mut found = Vec::with_capacity(keys.len());
        if keys.is_empty() {
            return found;
        }
        let Some(conn) = self.conn.get() else {
            return keys.iter().map(|_| None).collect();
        };

        let mut conn = conn.clone();
        for chunk in budgeted(keys, String::len) {
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

    // INVARIANT: the permit is held for the whole call — that hold is the
    // concurrency cap, so it is taken by the caller and dropped only on return.
    pub(crate) async fn set_many(
        &self,
        _permit: OwnedSemaphorePermit,
        entries: Vec<(String, Vec<u8>)>,
    ) {
        let Some(conn) = self.conn.get() else {
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
        for chunk in budgeted(&storable, |(key, value)| key.len() + value.len()) {
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

/// Splits into commands bounded by BOTH key count and serialized bytes, so one
/// command's size cannot scale with the caller's payload.
fn budgeted<T>(items: &[T], size_of: impl Fn(&T) -> usize) -> Vec<&[T]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < items.len() {
        let mut bytes = 0;
        let mut end = start;
        while end < items.len() && end - start < MAX_KEYS_PER_COMMAND {
            let next = bytes + size_of(&items[end]);
            if end > start && next > MAX_COMMAND_BYTES {
                break;
            }
            bytes = next;
            end += 1;
        }
        chunks.push(&items[start..end]);
        start = end;
    }
    chunks
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
        assert!(
            cache.try_admit_write().is_some(),
            "a disabled cache still admits, and then stores nothing"
        );
    }

    #[tokio::test]
    async fn an_absent_url_or_a_zero_ttl_disables_the_cache() {
        let cases = [
            ("", Duration::from_mins(1)),
            ("   ", Duration::from_mins(1)),
            ("redis://127.0.0.1:6379", Duration::ZERO),
        ];

        for (url, ttl) in cases {
            assert!(
                !MetricViewCache::connect(url, ttl).enabled(),
                "should stay disabled: url={url:?} ttl={ttl:?}"
            );
        }
    }

    #[tokio::test]
    async fn unreachable_redis_serves_uncached_without_erroring() {
        // Port 1 is never a Redis; the handle must degrade, not fail.
        let cache = MetricViewCache::connect("redis://127.0.0.1:1", Duration::from_mins(1));

        assert!(!cache.enabled());
        assert_eq!(cache.get_many(&["k".to_owned()]).await, vec![None]);
    }

    #[test]
    fn a_command_is_bounded_by_key_count_and_by_bytes() {
        let small: Vec<usize> = (0..=(MAX_KEYS_PER_COMMAND * 2)).collect();
        let by_count = budgeted(&small, |_| 1);

        assert!(
            by_count
                .iter()
                .all(|chunk| chunk.len() <= MAX_KEYS_PER_COMMAND),
            "a chunk exceeded the key cap"
        );
        assert_eq!(
            by_count.iter().map(|chunk| chunk.len()).sum::<usize>(),
            small.len(),
            "chunking dropped or duplicated items"
        );

        let heavy = vec![MAX_COMMAND_BYTES / 2; 8];
        let by_bytes = budgeted(&heavy, |size| *size);

        assert!(
            by_bytes.iter().all(|chunk| chunk.len() <= 2),
            "byte budget did not bound a chunk well under the key cap"
        );

        // An item larger than the whole budget still ships, alone.
        let oversized = vec![MAX_COMMAND_BYTES * 4];
        assert_eq!(budgeted(&oversized, |size| *size).len(), 1);
        assert!(budgeted::<usize>(&[], |_| 1).is_empty());
    }

    #[tokio::test]
    async fn malformed_url_disables_without_panicking() {
        let cache = MetricViewCache::connect("not-a-redis-url", Duration::from_mins(1));

        assert!(!cache.enabled());
    }
}
