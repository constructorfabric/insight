//! The groups a capped split keeps, ranked before the read consuming them
//! compiles, over its own metric definition.
//!
//! INVARIANT: the ranking runs over the scope and window of the capped read.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::domain::compiler::dimensions::dimension_aliases;
use crate::domain::compiler::request::{GroupRankingQuery, RankedDimension, RankedGroup};

use super::super::catalog::MetricCatalog;
use super::super::error::QueryError;
use super::super::execute::fetch;

/// One ranked group: the dimension columns of the position it earned. The
/// value it was ordered by is not read back — the order is the answer.
#[derive(Debug, Deserialize)]
struct RankedRow {
    #[serde(flatten)]
    columns: BTreeMap<String, Value>,
}

/// The groups a cap keeps, highest-valued first.
pub(super) async fn ranked_groups(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    rank_metric_key: &str,
    query: &GroupRankingQuery,
) -> Result<Vec<RankedGroup>, QueryError> {
    let Some(metric) = catalog.metric(rank_metric_key) else {
        return Err(QueryError::UnknownMetric {
            metric: rank_metric_key.to_owned(),
        });
    };

    let compiled = catalog.compile_ranking(metric, query)?;
    let comment = format!("metric-values:ranking:{rank_metric_key}");
    let rows = fetch::<RankedRow>(clickhouse, &compiled, &comment)
        .await
        .map_err(|error| {
            tracing::warn!(rank_metric_key, %error, "ranking the groups of a capped split failed");
            QueryError::SplitUnranked
        })?;

    decode(rows, &query.dimensions)
}

/// INVARIANT: rank 0 is the remainder row's, so kept groups rank from 1.
fn decode(rows: Vec<RankedRow>, dimensions: &[String]) -> Result<Vec<RankedGroup>, QueryError> {
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let rank = u32::try_from(index + 1).map_err(|_| QueryError::SplitUnranked)?;
            Ok(RankedGroup {
                rank,
                dimensions: group_dimensions(&row, dimensions)?,
            })
        })
        .collect()
}

fn group_dimensions(
    row: &RankedRow,
    dimensions: &[String],
) -> Result<Vec<RankedDimension>, QueryError> {
    (0..dimensions.len())
        .map(|index| {
            let (value_alias, label_alias) = dimension_aliases(index);
            let Some(value) = row.columns.get(&value_alias).and_then(text) else {
                tracing::error!(
                    alias = value_alias,
                    "a ranking row reports no dimension value"
                );
                return Err(QueryError::SplitUnranked);
            };
            Ok(RankedDimension {
                value,
                label: row.columns.get(&label_alias).and_then(text),
            })
        })
        .collect()
}

/// INVARIANT: the ranking read projects both dimension columns through
/// `coalesce`, so a missing value is a shape mismatch, not an absent group.
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
    use chrono::NaiveDate;

    use super::super::super::catalog::product_metric_catalog;
    use super::super::super::fixtures::{SHIPPED_METRIC, offline_clickhouse};
    use super::*;
    use crate::domain::compiler::request::EntityScope;

    fn row(pairs: &[(&str, Value)]) -> RankedRow {
        RankedRow {
            columns: pairs
                .iter()
                .map(|(alias, value)| ((*alias).to_owned(), value.clone()))
                .collect(),
        }
    }

    fn dimensions() -> Vec<String> {
        vec!["repository".to_owned()]
    }

    fn query() -> GroupRankingQuery {
        GroupRankingQuery {
            tenant_id: "acme-tenant".to_owned(),
            entity_scope: EntityScope::Tenant,
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            dimension_filters: Vec::new(),
            dimensions: dimensions(),
            count: 5,
        }
    }

    #[test]
    fn the_order_a_ranking_answers_in_is_the_rank_each_group_earns() {
        let decoded = decode(
            vec![
                row(&[
                    ("dim_0_value", Value::from("example/app")),
                    ("dim_0_label", Value::from("Example App")),
                ]),
                row(&[
                    ("dim_0_value", Value::from("example/api")),
                    ("dim_0_label", Value::Null),
                ]),
            ],
            &dimensions(),
        )
        .expect("both rows carry their dimension");

        assert_eq!(
            decoded,
            vec![
                RankedGroup {
                    rank: 1,
                    dimensions: vec![RankedDimension {
                        value: "example/app".to_owned(),
                        label: Some("Example App".to_owned()),
                    }],
                },
                RankedGroup {
                    rank: 2,
                    dimensions: vec![RankedDimension {
                        value: "example/api".to_owned(),
                        label: None,
                    }],
                },
            ]
        );
    }

    #[test]
    fn each_named_dimension_is_read_from_the_column_its_position_owns() {
        let decoded = decode(
            vec![row(&[
                ("dim_0_value", Value::from("example/app")),
                ("dim_0_label", Value::from("Example App")),
                ("dim_1_value", Value::from("github")),
                ("dim_1_label", Value::from("GitHub")),
            ])],
            &["repository".to_owned(), "source".to_owned()],
        )
        .expect("both dimensions are reported");

        assert_eq!(
            decoded[0]
                .dimensions
                .iter()
                .map(|dimension| dimension.value.as_str())
                .collect::<Vec<_>>(),
            vec!["example/app", "github"]
        );
    }

    #[test]
    fn a_ranking_row_missing_a_dimension_column_ranks_nothing() {
        let decoded = decode(
            vec![row(&[("dim_0_label", Value::from("Example App"))])],
            &dimensions(),
        );

        assert!(matches!(
            decoded.expect_err("the value column decides the group"),
            QueryError::SplitUnranked
        ));
    }

    #[tokio::test]
    async fn a_cap_ranked_by_a_metric_the_definitions_do_not_carry_ranks_nothing() {
        let error = ranked_groups(
            product_metric_catalog().expect("loads"),
            &offline_clickhouse(),
            "git.not_a_shipped_metric",
            &query(),
        )
        .await
        .expect_err("an undefined ranking metric names no groups");

        assert!(matches!(error, QueryError::UnknownMetric { .. }));
    }

    #[tokio::test]
    async fn a_ranking_read_no_server_answers_ranks_nothing() {
        let error = ranked_groups(
            product_metric_catalog().expect("loads"),
            &offline_clickhouse(),
            SHIPPED_METRIC,
            &query(),
        )
        .await
        .expect_err("a closed port cannot answer");

        assert!(matches!(error, QueryError::SplitUnranked));
    }

    #[tokio::test]
    async fn a_ranking_of_no_dimension_does_not_compile() {
        let mut query = query();
        query.dimensions.clear();

        let error = ranked_groups(
            product_metric_catalog().expect("loads"),
            &offline_clickhouse(),
            SHIPPED_METRIC,
            &query,
        )
        .await
        .expect_err("a cap ranks groups of a dimension");

        assert!(matches!(error, QueryError::Uncompilable(_)));
    }
}
