//! Per-connector ingestion state, summarised from one row per bronze stream.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone)]
pub struct StreamState {
    pub namespace: String,
    pub stream: String,
    pub rows: u64,
    /// `max(_airbyte_extracted_at)` over the stream. A stream holding no rows
    /// answers with the Unix epoch rather than null, so the epoch is a sentinel
    /// here and never a real extract.
    pub newest_extract: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorState {
    pub namespace: String,
    pub streams: usize,
    pub populated_streams: usize,
    pub rows: u64,
    pub last_write: Option<DateTime<Utc>>,
}

pub fn summarize(streams: &[StreamState]) -> Vec<ConnectorState> {
    let mut by_namespace: BTreeMap<&str, ConnectorState> = BTreeMap::new();

    for s in streams {
        let entry = by_namespace
            .entry(s.namespace.as_str())
            .or_insert_with(|| ConnectorState {
                namespace: s.namespace.clone(),
                streams: 0,
                populated_streams: 0,
                rows: 0,
                last_write: None,
            });

        entry.streams += 1;
        entry.populated_streams += usize::from(s.rows > 0);
        entry.rows += s.rows;
        entry.last_write = entry.last_write.max(extract_written(s));
    }

    by_namespace.into_values().collect()
}

fn extract_written(stream: &StreamState) -> Option<DateTime<Utc>> {
    stream
        .newest_extract
        .filter(|t| *t != DateTime::<Utc>::UNIX_EPOCH)
}

/// How stale a connector's newest data is allowed to be before it is worth
/// reporting. Every connector declares its own pair, so there is no single
/// instance-wide window.
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub warn_after: Duration,
    pub error_after: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    NeverReceived,
    Fresh,
    Warn,
    Stale,
}

pub fn freshness(state: &ConnectorState, thresholds: Thresholds, now: DateTime<Utc>) -> Freshness {
    let Some(last_write) = state.last_write else {
        return Freshness::NeverReceived;
    };

    let age = now - last_write;

    if age >= thresholds.error_after {
        Freshness::Stale
    } else if age >= thresholds.warn_after {
        Freshness::Warn
    } else {
        Freshness::Fresh
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unsafe identifier: {0}")]
pub struct UnsafeIdentifier(pub String);

/// `max()` cannot be taken over a dynamic table list in one bound statement, so
/// the names are spliced. They come from the server's own catalogue, and
/// splicing stays safe only while every one is a plain identifier.
pub fn newest_extract_sql(
    streams: &[(String, String)],
) -> Result<Option<String>, UnsafeIdentifier> {
    if streams.is_empty() {
        return Ok(None);
    }

    let mut selects = Vec::with_capacity(streams.len());

    for (namespace, stream) in streams {
        for name in [namespace, stream] {
            if !is_plain_identifier(name) {
                return Err(UnsafeIdentifier(name.clone()));
            }
        }

        selects.push(format!(
            "SELECT '{namespace}' AS namespace, '{stream}' AS stream, \
             max(_airbyte_extracted_at) AS newest_extract \
             FROM `{namespace}`.`{stream}`"
        ));
    }

    Ok(Some(selects.join(" UNION ALL ")))
}

fn is_plain_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn stream_ref(namespace: &str, stream: &str) -> (String, String) {
        (namespace.to_owned(), stream.to_owned())
    }

    #[test]
    fn an_instance_with_no_bronze_streams_yields_no_statement_to_run() {
        assert_eq!(newest_extract_sql(&[]), Ok(None));
    }

    #[test]
    fn one_select_per_stream_is_joined_by_union_all() {
        let sql = newest_extract_sql(&[
            stream_ref("bronze_example", "alpha"),
            stream_ref("bronze_example", "beta"),
        ])
        .unwrap()
        .unwrap();

        assert_eq!(sql.matches("UNION ALL").count(), 1);
        assert!(sql.contains("FROM `bronze_example`.`alpha`"));
        assert!(sql.contains("FROM `bronze_example`.`beta`"));
    }

    #[test]
    fn a_stream_name_that_is_not_a_plain_identifier_is_refused() {
        let refused = newest_extract_sql(&[stream_ref("bronze_example", "alpha`; DROP TABLE x --")]);

        assert_eq!(
            refused,
            Err(UnsafeIdentifier("alpha`; DROP TABLE x --".to_owned()))
        );
    }

    fn stream(
        namespace: &str,
        stream: &str,
        rows: u64,
        newest_extract: Option<DateTime<Utc>>,
    ) -> StreamState {
        StreamState {
            namespace: namespace.to_owned(),
            stream: stream.to_owned(),
            rows,
            newest_extract,
        }
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn streams_of_one_connector_fold_into_a_single_row() {
        let summary = summarize(&[
            stream("bronze_example", "alpha", 4, Some(at("2020-01-01T00:00:00Z"))),
            stream("bronze_example", "beta", 0, Some(DateTime::<Utc>::UNIX_EPOCH)),
            stream("bronze_example", "gamma", 6, Some(at("2020-01-02T00:00:00Z"))),
        ]);

        assert_eq!(
            summary,
            vec![ConnectorState {
                namespace: "bronze_example".to_owned(),
                streams: 3,
                populated_streams: 2,
                rows: 10,
                last_write: Some(at("2020-01-02T00:00:00Z")),
            }]
        );
    }

    fn thresholds(warn_hours: i64, error_hours: i64) -> Thresholds {
        Thresholds {
            warn_after: Duration::hours(warn_hours),
            error_after: Duration::hours(error_hours),
        }
    }

    fn written_at(last_write: Option<DateTime<Utc>>) -> ConnectorState {
        ConnectorState {
            namespace: "bronze_example".to_owned(),
            streams: 1,
            populated_streams: 1,
            rows: 1,
            last_write,
        }
    }

    #[test]
    fn data_older_than_the_error_threshold_is_stale() {
        let state = written_at(Some(at("2020-01-01T00:00:00Z")));

        let verdict = freshness(&state, thresholds(36, 72), at("2020-01-05T00:00:00Z"));

        assert_eq!(verdict, Freshness::Stale);
    }

    #[test]
    fn a_connector_that_never_received_data_is_not_reported_as_stale() {
        let state = written_at(None);

        let verdict = freshness(&state, thresholds(36, 72), at("2020-01-05T00:00:00Z"));

        assert_eq!(verdict, Freshness::NeverReceived);
    }

    #[test]
    fn data_between_the_two_thresholds_only_warns() {
        let state = written_at(Some(at("2020-01-01T00:00:00Z")));

        let verdict = freshness(&state, thresholds(36, 72), at("2020-01-03T00:00:00Z"));

        assert_eq!(verdict, Freshness::Warn);
    }

    #[test]
    fn connectors_come_back_one_row_each_in_namespace_order() {
        let summary = summarize(&[
            stream("bronze_zeta", "alpha", 1, Some(at("2020-01-01T00:00:00Z"))),
            stream("bronze_alpha", "alpha", 1, Some(at("2020-01-01T00:00:00Z"))),
        ]);

        let namespaces: Vec<&str> = summary.iter().map(|c| c.namespace.as_str()).collect();
        assert_eq!(namespaces, vec!["bronze_alpha", "bronze_zeta"]);
    }

    #[test]
    fn a_connector_whose_every_stream_is_empty_has_never_received_data() {
        let summary = summarize(&[
            stream("bronze_example", "alpha", 0, Some(DateTime::<Utc>::UNIX_EPOCH)),
            stream("bronze_example", "beta", 0, Some(DateTime::<Utc>::UNIX_EPOCH)),
        ]);

        assert_eq!(summary[0].populated_streams, 0);
        assert_eq!(summary[0].last_write, None);
    }

    #[test]
    fn an_empty_stream_has_no_last_write_even_though_the_source_reports_the_epoch() {
        let summary = summarize(&[stream(
            "bronze_example",
            "alpha",
            0,
            Some(DateTime::<Utc>::UNIX_EPOCH),
        )]);

        assert_eq!(summary[0].last_write, None);
    }
}
