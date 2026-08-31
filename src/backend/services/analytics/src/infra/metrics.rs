//! Domain instruments for metric-query execution.
//!
//! Registered against the global meter provider the toolkit bootstrap
//! installs — the `opentelemetry` major here must match the toolkit's (the
//! workspace pin), or these instruments record into a no-op global.

use std::sync::OnceLock;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, Meter};

// INVARIANT: one variant per query-building site (the `log_comment` prefixes),
// so the `kind` label on `analytics.metric_query.duration` stays bounded.
#[derive(Debug, Clone, Copy)]
pub(crate) enum QueryKind {
    Ranking,
    PeriodBatch,
    PeerBatch,
    Timeseries,
    Breakdown,
    Rollup,
    Histogram,
    PooledHistogram,
    DrilldownPage,
    DrilldownExport,
    Report,
}

impl QueryKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ranking => "ranking",
            Self::PeriodBatch => "period_batch",
            Self::PeerBatch => "peer_batch",
            Self::Timeseries => "timeseries",
            Self::Breakdown => "breakdown",
            Self::Rollup => "rollup",
            Self::Histogram => "histogram",
            Self::PooledHistogram => "pooled_histogram",
            Self::DrilldownPage => "drilldown_page",
            Self::DrilldownExport => "drilldown_export",
            Self::Report => "report",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum QueryOutcome {
    Success,
    Error,
}

impl QueryOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
        }
    }
}

// The resource-limit signatures ClickHouse reports when a query is refused
// rather than broken: memory (241), rows/bytes-to-read (158/307), execution
// time (159), simultaneous queries (202), quota (201).
const RESOURCE_LIMIT_MARKERS: [&str; 11] = [
    "MEMORY_LIMIT_EXCEEDED",
    "TOO_MANY_ROWS_OR_BYTES",
    "TOO_MANY_SIMULTANEOUS_QUERIES",
    "TIMEOUT_EXCEEDED",
    "QUOTA_EXCEEDED",
    "Code: 241.",
    "Code: 159.",
    "Code: 158.",
    "Code: 307.",
    "Code: 202.",
    "Code: 201.",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorClass {
    RelationMissing,
    ResourceExhausted,
    Timeout,
    ParseFailed,
    QueryFailed,
}

impl ErrorClass {
    pub(crate) fn classify(message: &str) -> Self {
        if message.contains("UNKNOWN_TABLE") || message.contains("Code: 60.") {
            return Self::RelationMissing;
        }
        if RESOURCE_LIMIT_MARKERS
            .iter()
            .any(|marker| message.contains(marker))
        {
            return Self::ResourceExhausted;
        }
        Self::QueryFailed
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::RelationMissing => "relation_missing",
            Self::ResourceExhausted => "resource_exhausted",
            Self::Timeout => "timeout",
            Self::ParseFailed => "parse_failed",
            Self::QueryFailed => "query_failed",
        }
    }
}

#[derive(Debug)]
struct Instruments {
    query_duration: Histogram<f64>,
    clickhouse_errors: Counter<u64>,
}

fn instruments() -> &'static Instruments {
    static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();
    INSTRUMENTS.get_or_init(|| {
        let meter: Meter = opentelemetry::global::meter("analytics.query");
        Instruments {
            query_duration: meter
                .f64_histogram("analytics.metric_query.duration")
                .with_unit("s")
                .with_description("Wall time per metric query, by kind and outcome.")
                .build(),
            clickhouse_errors: meter
                .u64_counter("analytics.clickhouse.errors")
                .with_description(
                    "Failed metric queries, by kind and failure class. relation_missing \
                     means the gold relation has not been built yet; resource_exhausted \
                     and timeout mean the query was refused or cut off, not broken.",
                )
                .build(),
        }
    })
}

pub(crate) fn record_query(kind: QueryKind, outcome: QueryOutcome, elapsed: Duration) {
    let attributes = [
        KeyValue::new("kind", kind.as_str()),
        KeyValue::new("outcome", outcome.as_str()),
    ];
    instruments()
        .query_duration
        .record(elapsed.as_secs_f64(), &attributes);
}

pub(crate) fn record_clickhouse_error(kind: QueryKind, class: ErrorClass) {
    let attributes = [
        KeyValue::new("kind", kind.as_str()),
        KeyValue::new("class", class.as_str()),
    ];
    instruments().clickhouse_errors.add(1, &attributes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_fixed_vocabularies() {
        assert_eq!(QueryKind::Ranking.as_str(), "ranking");
        assert_eq!(QueryKind::PeriodBatch.as_str(), "period_batch");
        assert_eq!(QueryKind::PeerBatch.as_str(), "peer_batch");
        assert_eq!(QueryKind::Timeseries.as_str(), "timeseries");
        assert_eq!(QueryKind::Breakdown.as_str(), "breakdown");
        assert_eq!(QueryKind::Rollup.as_str(), "rollup");
        assert_eq!(QueryKind::Histogram.as_str(), "histogram");
        assert_eq!(QueryKind::PooledHistogram.as_str(), "pooled_histogram");
        assert_eq!(QueryKind::DrilldownPage.as_str(), "drilldown_page");
        assert_eq!(QueryKind::DrilldownExport.as_str(), "drilldown_export");
        assert_eq!(QueryKind::Report.as_str(), "report");
        assert_eq!(QueryOutcome::Success.as_str(), "success");
        assert_eq!(QueryOutcome::Error.as_str(), "error");
        assert_eq!(ErrorClass::RelationMissing.as_str(), "relation_missing");
        assert_eq!(ErrorClass::ResourceExhausted.as_str(), "resource_exhausted");
        assert_eq!(ErrorClass::Timeout.as_str(), "timeout");
        assert_eq!(ErrorClass::ParseFailed.as_str(), "parse_failed");
        assert_eq!(ErrorClass::QueryFailed.as_str(), "query_failed");
    }

    #[test]
    fn classify_maps_ch_messages_to_the_bounded_classes() {
        for message in [
            "bad response: Code: 60. DB::Exception: Table x does not exist. (UNKNOWN_TABLE)",
            "Code: 60. something",
        ] {
            assert_eq!(
                ErrorClass::classify(message),
                ErrorClass::RelationMissing,
                "should be relation_missing: {message}"
            );
        }

        for message in [
            "Code: 241. Memory limit exceeded",
            "DB::Exception: (TIMEOUT_EXCEEDED)",
            "Code: 158. Too many rows",
            "Code: 307. Too many bytes",
        ] {
            assert_eq!(
                ErrorClass::classify(message),
                ErrorClass::ResourceExhausted,
                "should be resource_exhausted: {message}"
            );
        }

        // A longer code that shares a prefix must not match a shorter marker.
        for message in ["Code: 600. x", "Code: 2410. y", "Code: 60 no period"] {
            assert_eq!(
                ErrorClass::classify(message),
                ErrorClass::QueryFailed,
                "should be query_failed: {message}"
            );
        }
    }

    #[test]
    fn recording_without_a_provider_is_a_no_op_not_a_panic() {
        record_query(
            QueryKind::Timeseries,
            QueryOutcome::Success,
            Duration::from_millis(10),
        );
        record_clickhouse_error(QueryKind::Report, ErrorClass::Timeout);
    }
}
