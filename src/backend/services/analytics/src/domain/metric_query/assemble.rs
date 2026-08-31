//! The rows a read answered, turned into the answer's own vocabulary: the
//! read's positional columns and sentinels become explicit fields and variants.
//!
//! INVARIANT: ordering is decided here, never left to how ties came back.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::domain::compiler::dimensions::dimension_aliases;

use super::dto::{Group, GroupDimension, GroupedSeries, GroupedValue, Point, ResultBody};
use super::error::QueryError;

/// One value per subject, and per split group where the question named one.
#[derive(Debug, Deserialize)]
pub(super) struct SubjectValueRow {
    entity_id: String,
    value: Option<f64>,
    #[serde(flatten)]
    columns: BTreeMap<String, Value>,
}

/// One value per split group, folded over every subject.
#[derive(Debug, Deserialize)]
pub(super) struct CombinedValueRow {
    value: Option<f64>,
    rank: Option<u32>,
    remainder: u8,
    #[serde(flatten)]
    columns: BTreeMap<String, Value>,
}

/// One value per subject per bucket, plus the window total's own row.
#[derive(Debug, Deserialize)]
pub(super) struct SubjectSeriesRow {
    entity_id: String,
    bucket_start: String,
    value: Option<f64>,
    is_total: u8,
    rank: Option<u32>,
    remainder: u8,
    #[serde(flatten)]
    columns: BTreeMap<String, Value>,
}

/// Where one group sits: a capped split keeps the ranking's order, an uncapped
/// one is ordered by its values, and the remainder is last either way.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum GroupOrder {
    Ungrouped,
    Ranked(u32),
    Named(Vec<String>),
    Remainder,
}

pub(super) fn subject_values(
    rows: Vec<SubjectValueRow>,
    dimensions: &[String],
) -> Result<ResultBody, QueryError> {
    let mut values = BTreeMap::new();
    for row in rows {
        let (order, group) = grouping(&row.columns, dimensions, None, false)?;
        values.insert(
            (row.entity_id.clone(), order),
            GroupedValue {
                subject: Some(row.entity_id),
                group,
                value: row.value,
            },
        );
    }

    Ok(ResultBody::Values {
        values: values.into_values().collect(),
    })
}

pub(super) fn combined_values(
    rows: Vec<CombinedValueRow>,
    dimensions: &[String],
) -> Result<ResultBody, QueryError> {
    let mut values = BTreeMap::new();
    for row in rows {
        let (order, group) = grouping(&row.columns, dimensions, row.rank, row.remainder == 1)?;
        values.insert(
            order,
            GroupedValue {
                subject: None,
                group,
                value: row.value,
            },
        );
    }

    Ok(ResultBody::Values {
        values: values.into_values().collect(),
    })
}

/// INVARIANT: the total arrives as its own row, so it stays absent until that
/// row is read rather than being derived from the points.
#[derive(Debug, Default)]
struct SeriesUnderway {
    group: Option<Group>,
    points: Vec<Point>,
    total: Option<f64>,
}

pub(super) fn subject_series(
    rows: Vec<SubjectSeriesRow>,
    dimensions: &[String],
) -> Result<ResultBody, QueryError> {
    let mut series: BTreeMap<(String, GroupOrder), SeriesUnderway> = BTreeMap::new();
    for row in rows {
        let (order, group) = grouping(&row.columns, dimensions, row.rank, row.remainder == 1)?;
        let underway = series.entry((row.entity_id, order)).or_default();
        underway.group = group;

        if row.is_total == 1 {
            underway.total = row.value;
        } else {
            underway.points.push(Point {
                date: row.bucket_start,
                value: row.value,
            });
        }
    }

    Ok(ResultBody::Series {
        series: series
            .into_iter()
            .map(|((subject, _), mut underway)| {
                underway.points.sort_by(|a, b| a.date.cmp(&b.date));
                GroupedSeries {
                    subject: Some(subject),
                    group: underway.group,
                    points: underway.points,
                    total: underway.total,
                }
            })
            .collect(),
    })
}

/// Which group a row belongs to, and where that group sits in the answer.
fn grouping(
    columns: &BTreeMap<String, Value>,
    dimensions: &[String],
    rank: Option<u32>,
    remainder: bool,
) -> Result<(GroupOrder, Option<Group>), QueryError> {
    if dimensions.is_empty() {
        return Ok((GroupOrder::Ungrouped, None));
    }
    if remainder {
        return Ok((GroupOrder::Remainder, Some(Group::Remainder {})));
    }

    let named = group_dimensions(columns, dimensions)?;
    let order = match rank {
        Some(rank) => GroupOrder::Ranked(rank),
        None => GroupOrder::Named(
            named
                .iter()
                .map(|dimension| dimension.value.clone())
                .collect(),
        ),
    };
    Ok((order, Some(Group::Dimensions { dimensions: named })))
}

fn group_dimensions(
    columns: &BTreeMap<String, Value>,
    dimensions: &[String],
) -> Result<Vec<GroupDimension>, QueryError> {
    dimensions
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let (value_alias, label_alias) = dimension_aliases(index);
            let Some(value) = columns.get(&value_alias).and_then(text) else {
                tracing::error!(
                    alias = value_alias,
                    "a values row reports no dimension value"
                );
                return Err(QueryError::RowsUndecodable);
            };
            Ok(GroupDimension {
                key: key.clone(),
                value,
                label: columns.get(&label_alias).and_then(text),
            })
        })
        .collect()
}

/// INVARIANT: both dimension columns come through `coalesce`, so a missing
/// value is a shape mismatch rather than an absent group.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            Some(value.to_string())
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn dimensions() -> Vec<String> {
        vec!["repository".to_owned()]
    }

    fn columns(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(alias, value)| ((*alias).to_owned(), value.clone()))
            .collect()
    }

    fn repository(value: &str, label: Option<&str>) -> BTreeMap<String, Value> {
        columns(&[
            ("dim_0_value", Value::from(value)),
            ("dim_0_label", label.map_or(Value::Null, Value::from)),
        ])
    }

    fn named_group(value: &str, label: Option<&str>) -> Group {
        Group::Dimensions {
            dimensions: vec![GroupDimension {
                key: "repository".to_owned(),
                value: value.to_owned(),
                label: label.map(str::to_owned),
            }],
        }
    }

    fn values_of(body: ResultBody) -> Vec<GroupedValue> {
        match body {
            ResultBody::Values { values } => values,
            ResultBody::Series { .. } => panic!("a total-grain question answers with values"),
        }
    }

    fn series_of(body: ResultBody) -> Vec<GroupedSeries> {
        match body {
            ResultBody::Series { series } => series,
            ResultBody::Values { .. } => panic!("a bucketed question answers with series"),
        }
    }

    #[test]
    fn an_ungrouped_value_names_its_subject_and_no_group() {
        let body = subject_values(
            vec![SubjectValueRow {
                entity_id: "person-1".to_owned(),
                value: Some(12.0),
                columns: BTreeMap::new(),
            }],
            &[],
        )
        .expect("an ungrouped row needs no dimension column");

        assert_eq!(
            values_of(body),
            vec![GroupedValue {
                subject: Some("person-1".to_owned()),
                group: None,
                value: Some(12.0),
            }]
        );
    }

    #[test]
    fn a_split_value_carries_the_keys_the_question_asked_for() {
        let body = subject_values(
            vec![SubjectValueRow {
                entity_id: "person-1".to_owned(),
                value: Some(3.0),
                columns: repository("example/app", Some("Example App")),
            }],
            &dimensions(),
        )
        .expect("the row carries its dimension");

        assert_eq!(
            values_of(body),
            vec![GroupedValue {
                subject: Some("person-1".to_owned()),
                group: Some(named_group("example/app", Some("Example App"))),
                value: Some(3.0),
            }]
        );
    }

    #[test]
    fn a_row_missing_its_dimension_column_decodes_nothing() {
        let outcome = subject_values(
            vec![SubjectValueRow {
                entity_id: "person-1".to_owned(),
                value: Some(1.0),
                columns: columns(&[("dim_0_label", Value::from("Example App"))]),
            }],
            &dimensions(),
        );

        assert!(matches!(
            outcome.expect_err("the value column decides the group"),
            QueryError::RowsUndecodable
        ));
    }

    #[test]
    fn a_capped_split_reports_its_kept_groups_in_rank_order_and_the_leftover_last() {
        let body = combined_values(
            vec![
                CombinedValueRow {
                    value: Some(1.0),
                    rank: None,
                    remainder: 1,
                    columns: columns(&[("dim_0_value", Value::Null), ("dim_0_label", Value::Null)]),
                },
                CombinedValueRow {
                    value: Some(3.0),
                    rank: Some(2),
                    remainder: 0,
                    columns: repository("example/api", None),
                },
                CombinedValueRow {
                    value: Some(9.0),
                    rank: Some(1),
                    remainder: 0,
                    columns: repository("example/app", Some("Example App")),
                },
            ],
            &dimensions(),
        )
        .expect("every row carries its group");

        assert_eq!(
            values_of(body),
            vec![
                GroupedValue {
                    subject: None,
                    group: Some(named_group("example/app", Some("Example App"))),
                    value: Some(9.0),
                },
                GroupedValue {
                    subject: None,
                    group: Some(named_group("example/api", None)),
                    value: Some(3.0),
                },
                GroupedValue {
                    subject: None,
                    group: Some(Group::Remainder {}),
                    value: Some(1.0),
                },
            ]
        );
    }

    #[test]
    fn an_uncapped_split_is_ordered_by_the_values_it_grouped_by() {
        let body = combined_values(
            vec![
                CombinedValueRow {
                    value: Some(1.0),
                    rank: None,
                    remainder: 0,
                    columns: repository("example/lib", None),
                },
                CombinedValueRow {
                    value: Some(2.0),
                    rank: None,
                    remainder: 0,
                    columns: repository("example/api", None),
                },
            ],
            &dimensions(),
        )
        .expect("every row carries its group");

        assert_eq!(
            values_of(body)
                .into_iter()
                .map(|value| value.value)
                .collect::<Vec<_>>(),
            vec![Some(2.0), Some(1.0)]
        );
    }

    #[test]
    fn a_series_reports_the_window_total_as_its_own_field() {
        let body = subject_series(
            vec![
                SubjectSeriesRow {
                    entity_id: "person-1".to_owned(),
                    bucket_start: "2026-01-02".to_owned(),
                    value: Some(4.0),
                    is_total: 0,
                    rank: None,
                    remainder: 0,
                    columns: BTreeMap::new(),
                },
                SubjectSeriesRow {
                    entity_id: "person-1".to_owned(),
                    bucket_start: "1970-01-01".to_owned(),
                    value: Some(7.0),
                    is_total: 1,
                    rank: None,
                    remainder: 0,
                    columns: BTreeMap::new(),
                },
                SubjectSeriesRow {
                    entity_id: "person-1".to_owned(),
                    bucket_start: "2026-01-01".to_owned(),
                    value: Some(3.0),
                    is_total: 0,
                    rank: None,
                    remainder: 0,
                    columns: BTreeMap::new(),
                },
            ],
            &[],
        )
        .expect("an ungrouped series needs no dimension column");

        assert_eq!(
            series_of(body),
            vec![GroupedSeries {
                subject: Some("person-1".to_owned()),
                group: None,
                points: vec![
                    Point {
                        date: "2026-01-01".to_owned(),
                        value: Some(3.0),
                    },
                    Point {
                        date: "2026-01-02".to_owned(),
                        value: Some(4.0),
                    },
                ],
                total: Some(7.0),
            }]
        );
    }

    #[test]
    fn a_split_series_reports_one_series_per_subject_and_group() {
        let rows = vec![
            SubjectSeriesRow {
                entity_id: "person-1".to_owned(),
                bucket_start: "2026-01-01".to_owned(),
                value: Some(1.0),
                is_total: 0,
                rank: Some(1),
                remainder: 0,
                columns: repository("example/app", None),
            },
            SubjectSeriesRow {
                entity_id: "person-1".to_owned(),
                bucket_start: "2026-01-01".to_owned(),
                value: Some(2.0),
                is_total: 0,
                rank: None,
                remainder: 1,
                columns: columns(&[("dim_0_value", Value::Null), ("dim_0_label", Value::Null)]),
            },
            SubjectSeriesRow {
                entity_id: "person-2".to_owned(),
                bucket_start: "2026-01-01".to_owned(),
                value: Some(5.0),
                is_total: 0,
                rank: Some(1),
                remainder: 0,
                columns: repository("example/app", None),
            },
        ];

        let series = series_of(subject_series(rows, &dimensions()).expect("every row groups"));

        assert_eq!(
            series
                .iter()
                .map(|series| (
                    series.subject.clone(),
                    matches!(series.group, Some(Group::Remainder {}))
                ))
                .collect::<Vec<_>>(),
            vec![
                (Some("person-1".to_owned()), false),
                (Some("person-1".to_owned()), true),
                (Some("person-2".to_owned()), false),
            ]
        );
    }

    #[test]
    fn a_series_the_read_reported_no_total_row_for_reports_none() {
        let body = subject_series(
            vec![SubjectSeriesRow {
                entity_id: "person-1".to_owned(),
                bucket_start: "2026-01-01".to_owned(),
                value: Some(1.0),
                is_total: 0,
                rank: None,
                remainder: 0,
                columns: BTreeMap::new(),
            }],
            &[],
        )
        .expect("a series without its total row still reports its points");

        assert_eq!(series_of(body)[0].total, None);
    }
}
