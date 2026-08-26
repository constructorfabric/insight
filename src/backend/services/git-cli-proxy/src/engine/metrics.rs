//! The instruments DESIGN §4.3 specifies.
//!
//! Registered against the global meter provider the host installs, so the
//! `opentelemetry` major here must match the toolkit's — a different major is
//! a different global, and instruments would silently record into a no-op.
//!
//! Names are the `OpenTelemetry` spelling of the documented Prometheus
//! families: a
//! collector's Prometheus exporter renders `git_proxy.disk.used` as
//! `git_proxy_disk_used_bytes` once the `By` unit is applied.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, Meter};

/// Reclaim tier, for `git_proxy_evictions_total{tier=…}`.
#[derive(Debug, Clone, Copy)]
pub enum EvictionTier {
    /// Blobs purged; the entry stays warm.
    Blob,
    /// The whole entry deleted.
    Full,
}

impl EvictionTier {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Full => "full",
        }
    }
}

/// Outcome of a fetch, for `git_proxy_fetches_total{result=…}`.
#[derive(Debug, Clone, Copy)]
pub enum FetchResult {
    /// Origin had nothing new; the snapshot generation is unchanged.
    Noop,
    Updated,
    Error,
}

impl FetchResult {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Updated => "updated",
            Self::Error => "error",
        }
    }
}

/// Why a request was answered `429`. One label on one counter: the wire
/// collapses every cause into the same status, and without the split
/// "transient backpressure while a clone runs" is indistinguishable from
/// "the cache can never admit this work".
#[derive(Debug, Clone, Copy)]
pub enum RejectReason {
    /// The page-serve headroom check refused a blob prefetch.
    PrefetchHeadroom,
    /// A prefetch would take one entry past its own cap before a purge.
    EntryOverCap,
    /// Reclaim ran and the cache is still over the watermark.
    AdmissionExhausted,
    /// Preparation outlived the bounded inline wait.
    PreparationWait,
    /// Origin is throttling this client.
    OriginThrottled,
}

impl RejectReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PrefetchHeadroom => "prefetch_headroom",
            Self::EntryOverCap => "entry_over_cap",
            Self::AdmissionExhausted => "admission_exhausted",
            Self::PreparationWait => "preparation_wait",
            Self::OriginThrottled => "origin_throttled",
        }
    }
}

struct Instruments {
    evictions: Counter<u64>,
    rejections: Counter<u64>,
    handler_timeouts: Counter<u64>,
    origin_unavailable: Counter<u64>,
    purge_escalations: Counter<u64>,
    cold_clones: Counter<u64>,
    fetches: Counter<u64>,
    request_duration: Histogram<f64>,
    response_bytes: Histogram<u64>,
}

fn instruments() -> &'static Instruments {
    static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();
    INSTRUMENTS.get_or_init(|| {
        let meter: Meter = opentelemetry::global::meter("git-cli-proxy");
        Instruments {
            evictions: meter
                .u64_counter("git_proxy.evictions")
                .with_description("Cache entries reclaimed, by tier.")
                .build(),
            rejections: meter
                .u64_counter("git_proxy.rejections")
                .with_description(
                    "Requests answered 429, by reason. A sustained rise in admission_exhausted \
                     or prefetch_headroom means the budget is too small for the working set; \
                     preparation_wait is ordinary while a clone runs.",
                )
                .build(),
            handler_timeouts: meter
                .u64_counter("git_proxy.handler_timeouts")
                .with_description(
                    "Requests cut off at the handler budget and answered 503. Zero in \
                     health; any rise means a hold inside the store is never released \
                     and the cut-off is containing it — find the wedge in the error log.",
                )
                .build(),
            origin_unavailable: meter
                .u64_counter("git_proxy.origin_unavailable")
                .with_description(
                    "Requests refused because the origin declines to serve the repository \
                     (suspended, disabled, or over the vendor's own limit). Permanent per \
                     repository: the connector skips it, so a non-zero value means \
                     repositories are absent from bronze until the origin state changes.",
                )
                .build(),
            purge_escalations: meter
                .u64_counter("git_proxy.purge_escalations")
                .with_description(
                    "Post-serve purges that stopped yielding to readers and queued for the \
                     entry lock. Ordinary under continuous paging; a flat zero during one \
                     means the repack is being starved.",
                )
                .build(),
            cold_clones: meter
                .u64_counter("git_proxy.cold_clones")
                .with_description("Repositories cloned from scratch.")
                .build(),
            fetches: meter
                .u64_counter("git_proxy.fetches")
                .with_description("Fetches against an existing entry, by outcome.")
                .build(),
            request_duration: meter
                .f64_histogram("git_proxy.request.duration")
                .with_unit("s")
                .with_description("Wall time per API request, by endpoint and status.")
                .build(),
            response_bytes: meter
                .u64_histogram("git_proxy.response.size")
                .with_unit("By")
                .with_description("Serialized response size, by endpoint.")
                .build(),
        }
    })
}

pub fn record_eviction(tier: EvictionTier) {
    instruments()
        .evictions
        .add(1, &[KeyValue::new("tier", tier.as_str())]);
}

pub fn record_rejection(reason: RejectReason) {
    instruments()
        .rejections
        .add(1, &[KeyValue::new("reason", reason.as_str())]);
}

pub fn record_handler_timeout() {
    instruments().handler_timeouts.add(1, &[]);
}

pub fn record_origin_unavailable() {
    instruments().origin_unavailable.add(1, &[]);
}

pub fn record_purge_escalation() {
    instruments().purge_escalations.add(1, &[]);
}

pub fn record_cold_clone() {
    instruments().cold_clones.add(1, &[]);
}

pub fn record_fetch(result: FetchResult) {
    instruments()
        .fetches
        .add(1, &[KeyValue::new("result", result.as_str())]);
}

pub fn record_request(endpoint: &str, status: u16, seconds: f64, bytes: usize) {
    // INVARIANT: `endpoint` is a ROUTE template, never a request path, so the
    // label set stays bounded by the route table.
    let endpoint = endpoint.to_owned();
    let attributes = [
        KeyValue::new("endpoint", endpoint.clone()),
        KeyValue::new("status", i64::from(status)),
    ];
    instruments().request_duration.record(seconds, &attributes);
    instruments().response_bytes.record(
        u64::try_from(bytes).unwrap_or(u64::MAX),
        &[KeyValue::new("endpoint", endpoint)],
    );
}

/// Disk figures the gauges read.
///
/// Updated whenever admission recomputes them, and read by the observable
/// callbacks. The callbacks are synchronous and run on the collector's
/// schedule, so they must never touch the filesystem — hence a cached
/// snapshot rather than a live `statvfs` per scrape.
#[derive(Debug, Default)]
pub struct DiskGauges {
    used_bytes: AtomicU64,
    budget_bytes: AtomicU64,
    repos: AtomicU64,
}

impl DiskGauges {
    pub fn set(&self, used_bytes: u64, budget_bytes: u64, repos: u64) {
        self.used_bytes.store(used_bytes, Ordering::Relaxed);
        self.budget_bytes.store(budget_bytes, Ordering::Relaxed);
        self.repos.store(repos, Ordering::Relaxed);
    }
}

/// Attach the observable gauges of §4.3 to `gauges`. Called once at gear init.
pub fn register_disk_gauges(gauges: &Arc<DiskGauges>) {
    let meter = opentelemetry::global::meter("git-cli-proxy");

    let used = Arc::clone(gauges);
    let _ = meter
        .u64_observable_gauge("git_proxy.disk.used")
        .with_unit("By")
        .with_description("Cache bytes in use, the stricter of accounting and the volume.")
        .with_callback(move |observer| {
            observer.observe(used.used_bytes.load(Ordering::Relaxed), &[]);
        })
        .build();

    let budget = Arc::clone(gauges);
    let _ = meter
        .u64_observable_gauge("git_proxy.disk.budget")
        .with_unit("By")
        .with_description("Configured cache budget.")
        .with_callback(move |observer| {
            observer.observe(budget.budget_bytes.load(Ordering::Relaxed), &[]);
        })
        .build();

    let repos = Arc::clone(gauges);
    let _ = meter
        .u64_observable_gauge("git_proxy.repos")
        .with_description("Repositories currently cached.")
        .with_callback(move |observer| {
            observer.observe(repos.repos.load(Ordering::Relaxed), &[]);
        })
        .build();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_and_result_labels_are_the_documented_values() {
        assert_eq!(EvictionTier::Blob.as_str(), "blob");
        assert_eq!(EvictionTier::Full.as_str(), "full");
        assert_eq!(FetchResult::Noop.as_str(), "noop");
        assert_eq!(FetchResult::Updated.as_str(), "updated");
        assert_eq!(FetchResult::Error.as_str(), "error");
    }

    #[test]
    fn recording_without_a_provider_is_a_no_op_not_a_panic() {
        // No global provider is installed in tests; the instruments must still
        // build and record, or every handler would panic under a host that
        // disables metrics.
        record_eviction(EvictionTier::Blob);
        record_rejection(RejectReason::AdmissionExhausted);
        record_purge_escalation();
        record_cold_clone();
        record_fetch(FetchResult::Updated);
        record_request("/v1/commits", 200, 0.01, 128);

        let gauges = Arc::new(DiskGauges::default());
        gauges.set(1, 2, 3);
        register_disk_gauges(&gauges);
    }
}
