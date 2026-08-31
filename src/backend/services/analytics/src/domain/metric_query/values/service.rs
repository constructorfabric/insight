//! One request, end to end: the questions are planned together, read with a
//! bounded number in flight, and assembled in the order they were asked.
//!
//! INVARIANT: a request is answered whole or refused whole.

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use serde::de::DeserializeOwned;

use super::super::catalog::MetricCatalog;
use super::super::error::QueryError;
use super::super::execute::fetch;
use super::super::provenance::{metric_versions, provenance};
use super::super::question::bounded;
use super::assemble::{
    CombinedValueRow, SubjectSeriesRow, SubjectValueRow, combined_values, subject_series,
    subject_values,
};
use super::dto::{QueryResult, ResultBody, ValuesResponse};
use super::plan::{PlannedQuery, plan};
use super::validation::{QueryShape, ValidatedBatch, ValidatedQuery};

/// How many reads this endpoint keeps in flight for one request.
const QUERY_CONCURRENCY: usize = 4;

pub async fn answer(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    db: &DatabaseConnection,
    cache_reads_enabled: bool,
    tenant_id: Uuid,
    batch: ValidatedBatch,
) -> Result<ValuesResponse, QueryError> {
    let compiled = plan(
        catalog,
        clickhouse,
        db,
        cache_reads_enabled,
        tenant_id,
        &batch,
    )
    .await?;

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
        .zip(&compiled)
        .zip(bodies?)
        .map(|((query, planned), result)| QueryResult {
            metric: query.metric_key.clone(),
            provenance: provenance(&versions, &query.metric_key, planned.served_from()),
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
            subject_values(rows, compared, dimensions, query.entity_type)
        }
        QueryShape::CombinedSplit => {
            let (rows, compared) =
                read_both::<CombinedValueRow>(clickhouse, planned, &comment).await?;
            combined_values(rows, compared, dimensions)
        }
        QueryShape::SubjectSeries => {
            let (rows, compared) =
                read_both::<SubjectSeriesRow>(clickhouse, planned, &comment).await?;
            subject_series(rows, compared, dimensions, query.entity_type)
        }
    }
}

/// The question's own rows, and the compared window's when it asked for one.
/// Both are read before either is assembled, so a comparison is never half-read.
///
/// INVARIANT: a question no identity resolves for reads exactly as a scan that
/// matched nothing, so both are assembled into the same answer.
async fn read_both<T>(
    clickhouse: &insight_clickhouse::Client,
    planned: &PlannedQuery,
    comment: &str,
) -> Result<(Vec<T>, Option<Vec<T>>), QueryError>
where
    T: DeserializeOwned,
{
    let PlannedQuery::Read {
        current, compared, ..
    } = planned
    else {
        return Ok((Vec::new(), None));
    };

    let rows = bounded(fetch::<T>(clickhouse, current, comment).await?)?;
    let Some(compared) = compared else {
        return Ok((rows, None));
    };

    let compared_comment = format!("{comment}:compared");
    let compared = bounded(fetch::<T>(clickhouse, compared, &compared_comment).await?)?;
    Ok((rows, Some(compared)))
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
    use crate::domain::metric_query::dto::ServedFrom;

    use super::super::super::fixtures::offline_clickhouse;
    use super::super::dto::Grain;
    use super::super::fixtures::validated;
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

    fn planned() -> PlannedQuery {
        PlannedQuery::Read {
            current: CompiledMeasureQuery {
                sql: "SELECT 1".to_owned(),
                params: Vec::new(),
            },
            compared: None,
            served_from: ServedFrom::Computed,
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

    /// A person the mapping knows nothing about reads as no data, not as a bad
    /// request: the question is answerable, and its answer is that nothing was
    /// observed.
    #[tokio::test]
    async fn a_question_no_identity_resolves_for_is_answered_empty_without_a_read() {
        for shape in [
            QueryShape::SubjectTotal,
            QueryShape::SubjectSplit,
            QueryShape::CombinedSplit,
            QueryShape::SubjectSeries,
        ] {
            let body = body(
                &offline_clickhouse(),
                &query(shape),
                &PlannedQuery::NoIdentities,
            )
            .await
            .unwrap_or_else(|error| panic!("{shape:?} is answerable: {error}"));

            let empty = match shape {
                QueryShape::SubjectSeries => ResultBody::Series { series: Vec::new() },
                QueryShape::SubjectTotal | QueryShape::SubjectSplit | QueryShape::CombinedSplit => {
                    ResultBody::Values { values: Vec::new() }
                }
            };
            assert_eq!(body, empty, "{shape:?}");
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
