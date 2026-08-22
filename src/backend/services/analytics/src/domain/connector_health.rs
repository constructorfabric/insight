use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone)]
pub struct StreamState {
    pub namespace: String,
    #[expect(dead_code, reason = "names the source stream; only the fold is read today")]
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

/// Each connector declares its own pair; there is no instance-wide window.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code, reason = "the classifier is unused until the declared windows have a runtime source")]
pub struct Thresholds {
    pub warn_after: Duration,
    pub error_after: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "the classifier is unused until the declared windows have a runtime source")]
pub enum Freshness {
    NeverReceived,
    Fresh,
    Warn,
    Stale,
}

#[allow(dead_code, reason = "the classifier is unused until the declared windows have a runtime source")]
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

pub const BRONZE_PREFIX: &str = "bronze_";

const EXTRACT_COLUMN: &str = "_airbyte_extracted_at";

pub fn connector_name(namespace: &str) -> &str {
    namespace.strip_prefix(BRONZE_PREFIX).unwrap_or(namespace)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unsafe identifier: {0}")]
pub struct UnsafeIdentifier(pub String);

/// Table names are spliced, not bound: splicing stays safe only while every
/// name is a plain identifier.
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
             max({EXTRACT_COLUMN}) AS newest_extract \
             FROM `{namespace}`.`{stream}`"
        ));
    }

    Ok(Some(selects.join(" UNION ALL ")))
}

fn is_plain_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Inactive parts are excluded; a merged-away part would otherwise count twice.
const STREAM_CATALOGUE_SQL: &str = "\
    SELECT c.database AS namespace, \
           c.table AS stream, \
           p.rows AS rows \
    FROM system.columns AS c \
    LEFT JOIN ( \
        SELECT database, table, sum(rows) AS rows \
        FROM system.parts WHERE active GROUP BY database, table \
    ) AS p ON p.database = c.database AND p.table = c.table \
    WHERE c.name = ? AND startsWith(c.database, ?) \
    ORDER BY namespace, stream";

#[derive(Debug, clickhouse::Row, serde::Deserialize)]
struct CatalogueRow {
    namespace: String,
    stream: String,
    rows: u64,
}

#[derive(Debug, clickhouse::Row, serde::Deserialize)]
struct ExtractRow {
    namespace: String,
    stream: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    newest_extract: DateTime<Utc>,
}

pub async fn read_stream_states(
    ch: &insight_clickhouse::Client,
) -> Result<Vec<StreamState>, ReadError> {
    let catalogue: Vec<CatalogueRow> = ch
        .query(STREAM_CATALOGUE_SQL)
        .bind(EXTRACT_COLUMN)
        .bind(BRONZE_PREFIX)
        .fetch_all()
        .await?;

    let refs: Vec<(String, String)> = catalogue
        .iter()
        .map(|r| (r.namespace.clone(), r.stream.clone()))
        .collect();

    let Some(sql) = newest_extract_sql(&refs)? else {
        return Ok(Vec::new());
    };

    let extracts: Vec<ExtractRow> = ch.query(&sql).fetch_all().await?;
    let newest: BTreeMap<(&str, &str), DateTime<Utc>> = extracts
        .iter()
        .map(|r| ((r.namespace.as_str(), r.stream.as_str()), r.newest_extract))
        .collect();

    Ok(catalogue
        .iter()
        .map(|r| StreamState {
            namespace: r.namespace.clone(),
            stream: r.stream.clone(),
            rows: r.rows,
            newest_extract: newest
                .get(&(r.namespace.as_str(), r.stream.as_str()))
                .copied(),
        })
        .collect())
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Clickhouse(#[from] clickhouse::error::Error),
    #[error(transparent)]
    Identifier(#[from] UnsafeIdentifier),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn stream_ref(namespace: &str, stream: &str) -> (String, String) {
        (namespace.to_owned(), stream.to_owned())
    }

    #[test]
    fn the_bronze_prefix_is_dropped_from_the_reported_connector_name() {
        assert_eq!(connector_name("bronze_example"), "example");
    }

    #[test]
    fn a_schema_without_the_bronze_prefix_is_reported_as_it_stands() {
        assert_eq!(connector_name("example"), "example");
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
        let refused =
            newest_extract_sql(&[stream_ref("bronze_example", "alpha`; DROP TABLE x --")]);

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
            stream(
                "bronze_example",
                "alpha",
                4,
                Some(at("2020-01-01T00:00:00Z")),
            ),
            stream(
                "bronze_example",
                "beta",
                0,
                Some(DateTime::<Utc>::UNIX_EPOCH),
            ),
            stream(
                "bronze_example",
                "gamma",
                6,
                Some(at("2020-01-02T00:00:00Z")),
            ),
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
            stream(
                "bronze_example",
                "alpha",
                0,
                Some(DateTime::<Utc>::UNIX_EPOCH),
            ),
            stream(
                "bronze_example",
                "beta",
                0,
                Some(DateTime::<Utc>::UNIX_EPOCH),
            ),
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
