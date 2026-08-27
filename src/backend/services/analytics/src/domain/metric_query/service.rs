//! One request, end to end: the questions are planned together, read with a
//! bounded number in flight, and assembled in the order they were asked.
//!
//! INVARIANT: a request is answered whole or refused whole.

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use serde::de::DeserializeOwned;

use super::assemble::{
    CombinedValueRow, SubjectSeriesRow, SubjectValueRow, combined_values, subject_series,
    subject_values,
};
use super::catalog::MetricCatalog;
use super::dto::{QueryResult, ResultBody, ValuesResponse};
use super::error::QueryError;
use super::execute::fetch;
use super::plan::{PlannedQuery, plan};
use super::provenance::{metric_versions, provenance};
use super::validation::{QueryShape, ValidatedBatch, ValidatedQuery, row_limit};

/// How many reads this endpoint keeps in flight for one request.
const QUERY_CONCURRENCY: usize = 4;

pub async fn answer(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    db: &DatabaseConnection,
    tenant_id: Uuid,
    batch: ValidatedBatch,
) -> Result<ValuesResponse, QueryError> {
    let compiled = plan(catalog, clickhouse, tenant_id, &batch).await?;

    let keys: Vec<String> = batch
        .queries
        .iter()
        .map(|query| query.metric_key.clone())
        .collect();
    let (bodies, versions) = tokio::join!(
        read_all(clickhouse, &batch, &compiled),
        metric_versions(db, &keys)
    );

    let results = batch
        .queries
        .iter()
        .zip(bodies?)
        .map(|(query, result)| QueryResult {
            metric: query.metric_key.clone(),
            provenance: provenance(&versions, &query.metric_key),
            result,
        })
        .collect();
    Ok(ValuesResponse { results })
}

async fn read_all(
    clickhouse: &insight_clickhouse::Client,
    batch: &ValidatedBatch,
    compiled: &[PlannedQuery],
) -> Result<Vec<ResultBody>, QueryError> {
    let mut bodies = Vec::with_capacity(compiled.len());
    let pairs: Vec<(&ValidatedQuery, &PlannedQuery)> = batch.queries.iter().zip(compiled).collect();

    for chunk in pairs.chunks(QUERY_CONCURRENCY) {
        let reads = chunk
            .iter()
            .map(|(query, compiled)| body(clickhouse, query, compiled));
        for read in futures::future::join_all(reads).await {
            bodies.push(read?);
        }
    }
    Ok(bodies)
}

async fn body(
    clickhouse: &insight_clickhouse::Client,
    query: &ValidatedQuery,
    planned: &PlannedQuery,
) -> Result<ResultBody, QueryError> {
    let comment = format!(
        "metric-values:{}:{}",
        shape_name(query.shape),
        query.metric_key
    );
    let dimensions = query.dimensions();

    match query.shape {
        QueryShape::SubjectTotal | QueryShape::SubjectSplit => {
            let (rows, compared) =
                read_both::<SubjectValueRow>(clickhouse, planned, &comment).await?;
            subject_values(rows, compared, dimensions)
        }
        QueryShape::CombinedSplit => {
            let (rows, compared) =
                read_both::<CombinedValueRow>(clickhouse, planned, &comment).await?;
            combined_values(rows, compared, dimensions)
        }
        QueryShape::SubjectSeries => {
            let (rows, compared) =
                read_both::<SubjectSeriesRow>(clickhouse, planned, &comment).await?;
            subject_series(rows, compared, dimensions)
        }
    }
}

/// The question's own rows, and the compared window's when it asked for one.
/// Both are read before either is assembled, so a comparison is never half-read.
async fn read_both<T>(
    clickhouse: &insight_clickhouse::Client,
    planned: &PlannedQuery,
    comment: &str,
) -> Result<(Vec<T>, Option<Vec<T>>), QueryError>
where
    T: DeserializeOwned,
{
    let rows = bounded(fetch::<T>(clickhouse, &planned.current, comment).await?)?;
    let Some(compared) = &planned.compared else {
        return Ok((rows, None));
    };

    let compared_comment = format!("{comment}:compared");
    let compared = bounded(fetch::<T>(clickhouse, compared, &compared_comment).await?)?;
    Ok((rows, Some(compared)))
}

/// INVARIANT: the read binds one row over the ceiling, so an answer past it is
/// refused rather than served short.
fn bounded<T>(rows: Vec<T>) -> Result<Vec<T>, QueryError> {
    if rows.len() > row_limit() {
        return Err(QueryError::ResultTooLarge { limit: row_limit() });
    }
    Ok(rows)
}

fn shape_name(shape: QueryShape) -> &'static str {
    match shape {
        QueryShape::SubjectTotal => "subject-total",
        QueryShape::SubjectSplit => "subject-split",
        QueryShape::CombinedSplit => "combined-split",
        QueryShape::SubjectSeries => "subject-series",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::domain::compiler::sql::CompiledMeasureQuery;

    use super::super::dto::Grain;
    use super::super::fixtures::{offline_clickhouse, validated};
    use super::*;

    fn query(shape: QueryShape) -> ValidatedQuery {
        let grain = match shape {
            QueryShape::SubjectSeries => Grain::Day,
            QueryShape::SubjectTotal | QueryShape::SubjectSplit | QueryShape::CombinedSplit => {
                Grain::Total
            }
        };
        let dimensions: &[&str] = match shape {
            QueryShape::SubjectTotal | QueryShape::SubjectSeries => &[],
            QueryShape::SubjectSplit | QueryShape::CombinedSplit => &["repository"],
        };
        validated(shape, grain, dimensions)
    }

    #[test]
    fn an_answer_within_the_ceiling_is_served_and_one_past_it_is_refused() {
        assert_eq!(
            bounded(vec![0_u8; row_limit()]).map(|rows| rows.len()).ok(),
            Some(row_limit())
        );

        assert!(matches!(
            bounded(vec![0_u8; row_limit() + 1]).expect_err("one row over the ceiling is refused"),
            QueryError::ResultTooLarge { .. }
        ));
    }

    fn planned() -> PlannedQuery {
        PlannedQuery {
            current: CompiledMeasureQuery {
                sql: "SELECT 1".to_owned(),
                params: Vec::new(),
            },
            compared: None,
        }
    }

    #[tokio::test]
    async fn every_shape_reads_and_fails_as_its_own_question() {
        let compiled = planned();

        for shape in [
            QueryShape::SubjectTotal,
            QueryShape::SubjectSplit,
            QueryShape::CombinedSplit,
            QueryShape::SubjectSeries,
        ] {
            let outcome = body(&offline_clickhouse(), &query(shape), &compiled).await;

            assert!(
                matches!(
                    outcome.expect_err("a closed port cannot answer"),
                    QueryError::ReadFailed
                ),
                "{shape:?}"
            );
        }
    }

    #[tokio::test]
    async fn one_question_that_cannot_be_read_refuses_the_whole_request() {
        let batch = ValidatedBatch {
            queries: vec![
                query(QueryShape::SubjectTotal),
                query(QueryShape::SubjectSeries),
            ],
        };
        let compiled = vec![planned(), planned()];

        let outcome = read_all(&offline_clickhouse(), &batch, &compiled).await;

        assert!(matches!(
            outcome.expect_err("a request is answered whole or not at all"),
            QueryError::ReadFailed
        ));
    }
}
