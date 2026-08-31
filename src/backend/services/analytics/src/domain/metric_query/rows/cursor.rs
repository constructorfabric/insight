//! Where a page resumes, and what that position is still valid against: the
//! question it was issued for, the table its rows came from, and the identity
//! mapping that keyed them.
//!
//! INVARIANT: a position is opaque to the caller and carries no row of its own,
//! so a cursor discloses nothing a page did not already report.

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::super::error::QueryError;
use super::validation::ValidatedRows;

const VERSION: u8 = 1;

/// What a page is bound to: a rebuild of either side invalidates the positions
/// issued before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Anchor {
    /// The identity the dataset relation currently has in the warehouse.
    pub snapshot_id: String,
    /// The marker of the identity mapping the rows were attributed through.
    pub identity_epoch: u64,
}

/// A position read off a cursor, once it is known to be this question's.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Resume {
    pub anchor: Anchor,
    /// What the previous page's last row carried in the sorted column. Null
    /// when the question sorts by nothing, and when that row carried no value
    /// in the column it does sort by.
    pub sort_value: serde_json::Value,
    /// The ordering values the previous page's last row carried.
    pub sort_values: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Envelope {
    v: u8,
    fp: String,
    snap: String,
    epoch: u64,
    key: Vec<String>,
    /// Absent on a question that sorts by nothing, which is what an envelope
    /// carrying no sorted value means.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    sort: serde_json::Value,
}

/// What the question asks for, as a position is valid against it. INVARIANT:
/// the page size and the position itself are left out — paging the same
/// question in different-sized steps is the same question.
pub(super) fn fingerprint(tenant_id: Uuid, request: &ValidatedRows) -> Result<String, QueryError> {
    let mut filters: Vec<(String, Vec<String>)> = request
        .filters
        .iter()
        .map(|filter| {
            let mut values = filter.values.clone();
            values.sort();
            (filter.key.clone(), values)
        })
        .collect();
    filters.sort();

    let asked = serde_json::json!({
        "tenant": tenant_id.to_string(),
        "metric": request.metric_key,
        "subjects": request.subjects,
        "from": request.from.to_string(),
        "to": request.to.to_string(),
        "filters": filters,
        "input": request.input_role,
        "display_dimensions": request.display_dimensions,
        "sort": request.sort.as_ref().map(|sort| (&sort.column, sort.direction.keyword())),
    });

    let bytes = serde_json::to_vec(&asked).map_err(|error| {
        tracing::error!(%error, "a page question could not be fingerprinted");
        QueryError::PageUnanchored
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(super) fn encode(
    fingerprint: &str,
    anchor: &Anchor,
    sort_value: serde_json::Value,
    sort_values: Vec<String>,
) -> Result<String, QueryError> {
    let envelope = Envelope {
        v: VERSION,
        fp: fingerprint.to_owned(),
        snap: anchor.snapshot_id.clone(),
        epoch: anchor.identity_epoch,
        key: sort_values,
        sort: sort_value,
    };

    let bytes = serde_json::to_vec(&envelope).map_err(|error| {
        tracing::error!(%error, "a page position could not be written");
        QueryError::PageUnanchored
    })?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// A position that is not this question's is refused rather than resumed: the
/// ordering values of one question select unrelated rows under another.
pub(super) fn decode(value: &str, fingerprint: &str) -> Result<Resume, QueryError> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| QueryError::CursorUnreadable)?;
    let envelope: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| QueryError::CursorUnreadable)?;

    if envelope.v != VERSION {
        return Err(QueryError::CursorUnreadable);
    }
    if envelope.fp != fingerprint {
        return Err(QueryError::CursorMismatched);
    }

    Ok(Resume {
        anchor: Anchor {
            snapshot_id: envelope.snap,
            identity_epoch: envelope.epoch,
        },
        sort_value: envelope.sort,
        sort_values: envelope.key,
    })
}

/// The identity a relation currently has in the warehouse. A rebuild keeps the
/// name and takes a new table, so this changes exactly when the rows underneath
/// a page were replaced.
pub(super) async fn relation_snapshot(
    clickhouse: &insight_clickhouse::Client,
    database: &str,
    relation: &str,
) -> Result<String, QueryError> {
    #[derive(Debug, Deserialize, clickhouse::Row)]
    struct SnapshotRow {
        snapshot_id: String,
    }

    clickhouse
        .query(
            "SELECT toString(uuid) AS snapshot_id \
             FROM system.tables WHERE database = ? AND name = ?",
        )
        .bind(database)
        .bind(relation)
        .fetch_one::<SnapshotRow>()
        .await
        .map(|row| row.snapshot_id)
        .map_err(|error| {
            tracing::error!(%error, database, relation, "a page's relation identity went unread");
            QueryError::PageUnanchored
        })
}

/// A page whose anchor moved cannot be continued, however far the caller had
/// read: the rows after the position it holds are no longer the same rows.
pub(super) fn still_anchored(resumed: &Anchor, current: &Anchor) -> Result<(), QueryError> {
    if resumed == current {
        return Ok(());
    }
    Err(QueryError::PageExpired)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use crate::domain::compiler::request::{DimensionFilter, SortDirection};
    use crate::domain::metric_query::question::ValidatedSubjects;

    use super::super::super::fixtures::{SHIPPED_METRIC, offline_clickhouse, tenant};
    use super::super::validation::ValidatedSort;
    use super::*;

    fn anchor() -> Anchor {
        Anchor {
            snapshot_id: "dataset-uuid".to_owned(),
            identity_epoch: 42,
        }
    }

    fn validated() -> ValidatedRows {
        ValidatedRows {
            metric_key: SHIPPED_METRIC.to_owned(),
            subjects: ValidatedSubjects::Persons(vec![Uuid::from_u128(1)]),
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            filters: Vec::new(),
            input_role: "value".to_owned(),
            display_dimensions: Vec::new(),
            sort: None,
            page_size: 100,
            cursor: None,
        }
    }

    fn fingerprinted(request: &ValidatedRows) -> String {
        fingerprint(tenant(), request).expect("a question fingerprints")
    }

    #[test]
    fn a_position_survives_the_round_trip_it_was_issued_for() {
        let fingerprint = fingerprinted(&validated());
        let sort_values = vec!["row-7".to_owned(), "github".to_owned()];

        let encoded = encode(
            &fingerprint,
            &anchor(),
            serde_json::Value::Null,
            sort_values.clone(),
        )
        .expect("encodes");
        let resumed = decode(&encoded, &fingerprint).expect("decodes");

        assert_eq!(
            resumed,
            Resume {
                anchor: anchor(),
                sort_value: serde_json::Value::Null,
                sort_values,
            }
        );
    }

    #[test]
    fn a_position_issued_for_another_question_is_refused_rather_than_resumed() {
        let issued = fingerprinted(&validated());
        let encoded = encode(
            &issued,
            &anchor(),
            serde_json::Value::Null,
            vec!["row-7".to_owned()],
        )
        .expect("encodes");

        let elsewhere = ValidatedRows {
            to: NaiveDate::from_ymd_opt(2026, 2, 28).expect("valid date"),
            ..validated()
        };

        assert!(matches!(
            decode(&encoded, &fingerprinted(&elsewhere)),
            Err(QueryError::CursorMismatched)
        ));
    }

    #[test]
    fn a_cursor_that_is_not_one_is_refused_rather_than_half_read() {
        let fingerprint = fingerprinted(&validated());
        let cases = [
            ("not base64!".to_owned(), "text that is not a cursor"),
            (
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("not json"),
                "bytes that are not an envelope",
            ),
            (
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&serde_json::json!({ "v": 1, "fp": "x" }))
                        .expect("serializes"),
                ),
                "an envelope missing what its version requires",
            ),
            (
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&serde_json::json!({
                        "v": 7,
                        "fp": "x",
                        "snap": "s",
                        "epoch": 1,
                        "key": [],
                    }))
                    .expect("serializes"),
                ),
                "an envelope of an unknown version",
            ),
        ];

        for (value, named) in cases {
            assert!(
                matches!(
                    decode(&value, &fingerprint),
                    Err(QueryError::CursorUnreadable)
                ),
                "should refuse: {named}"
            );
        }
    }

    /// A page size is how much of the same question a caller asks for at once,
    /// so changing it must not throw away a position already issued.
    #[test]
    fn the_same_question_fingerprints_the_same_whatever_page_size_it_is_read_in() {
        let smaller = ValidatedRows {
            page_size: 10,
            ..validated()
        };
        let larger = ValidatedRows {
            page_size: 250,
            cursor: Some("a-position".to_owned()),
            ..validated()
        };

        assert_eq!(fingerprinted(&smaller), fingerprinted(&larger));
    }

    /// The narrowing decides which rows a page holds, so two differently
    /// narrowed questions must never resume each other's positions.
    #[test]
    fn a_question_narrowed_differently_fingerprints_differently() {
        let narrowed = ValidatedRows {
            filters: vec![DimensionFilter {
                key: "repository".to_owned(),
                values: vec!["example/app".to_owned()],
            }],
            ..validated()
        };

        assert_ne!(fingerprinted(&validated()), fingerprinted(&narrowed));
    }

    /// One narrowing written two ways is one narrowing: the values it names are
    /// a set, and a caller that reorders them holds the same page.
    #[test]
    fn one_narrowing_written_in_either_order_is_the_same_question() {
        let filter = |values: &[&str]| ValidatedRows {
            filters: vec![DimensionFilter {
                key: "repository".to_owned(),
                values: values.iter().map(|value| (*value).to_owned()).collect(),
            }],
            ..validated()
        };

        assert_eq!(
            fingerprinted(&filter(&["example/app", "example/api"])),
            fingerprinted(&filter(&["example/api", "example/app"]))
        );
    }

    #[test]
    fn a_page_whose_anchor_moved_is_refused_and_one_that_held_is_served() {
        let moved = [
            Anchor {
                snapshot_id: "rebuilt".to_owned(),
                ..anchor()
            },
            Anchor {
                identity_epoch: 43,
                ..anchor()
            },
        ];

        assert!(still_anchored(&anchor(), &anchor()).is_ok());
        for current in moved {
            assert!(matches!(
                still_anchored(&anchor(), &current),
                Err(QueryError::PageExpired)
            ));
        }
    }

    #[tokio::test]
    async fn a_relation_whose_identity_cannot_be_read_leaves_the_page_unanchored() {
        let outcome = relation_snapshot(&offline_clickhouse(), "silver", "class_git_commits").await;

        assert!(matches!(
            outcome.expect_err("a closed port cannot answer"),
            QueryError::PageUnanchored
        ));
    }

    fn ordered(column: &str, direction: SortDirection) -> ValidatedRows {
        ValidatedRows {
            sort: Some(ValidatedSort {
                column: column.to_owned(),
                direction,
            }),
            ..validated()
        }
    }

    /// The order decides which rows a page holds and in which sequence, so a
    /// position issued under one order selects unrelated rows under another.
    #[test]
    fn a_question_ordered_differently_fingerprints_differently() {
        let unordered = fingerprinted(&validated());
        let ascending = fingerprinted(&ordered("date", SortDirection::Ascending));
        let descending = fingerprinted(&ordered("date", SortDirection::Descending));
        let elsewhere = fingerprinted(&ordered("value", SortDirection::Ascending));

        for (named, other) in [
            ("an unordered question", &unordered),
            ("the other direction", &descending),
            ("another column", &elsewhere),
        ] {
            assert_ne!(&ascending, other, "should differ from {named}");
        }
    }

    /// A caller cannot resume a sorted page from a value the order it asked for
    /// never produced: the order is fingerprinted, so the position is refused
    /// before it can select anything.
    #[test]
    fn a_position_issued_under_another_order_is_refused_rather_than_resumed() {
        let issued = fingerprinted(&ordered("date", SortDirection::Ascending));
        let encoded = encode(
            &issued,
            &anchor(),
            serde_json::Value::from("2026-01-05"),
            vec!["row-7".to_owned()],
        )
        .expect("encodes");

        let refused = decode(
            &encoded,
            &fingerprinted(&ordered("date", SortDirection::Descending)),
        );

        assert!(
            matches!(refused, Err(QueryError::CursorMismatched)),
            "{refused:?}"
        );
    }

    #[test]
    fn a_sorted_position_carries_the_value_its_page_ended_on() {
        let fingerprint = fingerprinted(&ordered("value", SortDirection::Ascending));
        let cases = [
            ("a number", serde_json::Value::from(12)),
            ("text", serde_json::Value::from("a title")),
            ("no value at all", serde_json::Value::Null),
        ];

        for (named, sort_value) in cases {
            let encoded = encode(
                &fingerprint,
                &anchor(),
                sort_value.clone(),
                vec!["row-7".to_owned()],
            )
            .expect("encodes");

            let resumed = decode(&encoded, &fingerprint).expect("decodes");

            assert_eq!(resumed.sort_value, sort_value, "should round-trip: {named}");
        }
    }

    /// A question that orders by nothing writes no ordered value, so the
    /// positions it issues stay exactly what they were.
    #[test]
    fn an_unordered_question_issues_a_position_carrying_no_sorted_value() {
        let fingerprint = fingerprinted(&validated());

        let encoded = encode(
            &fingerprint,
            &anchor(),
            serde_json::Value::Null,
            vec!["row-7".to_owned()],
        )
        .expect("encodes");
        let decoded: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(&encoded)
                .expect("decodes"),
        )
        .expect("an envelope");

        assert!(
            decoded.get("sort").is_none(),
            "an unordered position carries no sorted value: {decoded}"
        );
    }
}
