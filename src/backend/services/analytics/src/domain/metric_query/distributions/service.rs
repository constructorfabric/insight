//! One request, end to end: the questions are planned together, read with a
//! bounded number in flight, and assembled in the order they were asked.
//!
//! INVARIANT: a request is answered whole or refused whole.

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::domain::compiler::sql::CompiledMeasureQuery;

use super::super::catalog::MetricCatalog;
use super::super::error::QueryError;
use super::super::execute::fetch;
use super::super::provenance::{metric_versions, provenance};
use super::super::question::bounded;
use super::assemble::{HistogramRow, QuantileRow, subject_distributions};
use super::dto::{DistributionResult, DistributionsResponse};
use super::plan::{PlannedDistribution, plan};
use super::validation::{ValidatedDistribution, ValidatedDistributions};

/// How many questions this endpoint keeps in flight for one request.
const QUERY_CONCURRENCY: usize = 4;

/// What one question's reads answered.
#[derive(Debug)]
struct Observations {
    histogram: Vec<HistogramRow>,
    quantiles: Vec<QuantileRow>,
}

pub async fn answer(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    db: &DatabaseConnection,
    tenant_id: Uuid,
    batch: ValidatedDistributions,
) -> Result<DistributionsResponse, QueryError> {
    let compiled = plan(catalog, clickhouse, tenant_id, &batch).await?;

    let keys: Vec<String> = batch
        .queries
        .iter()
        .map(|query| query.metric_key.clone())
        .collect();
    let (observed, versions) = tokio::join!(
        read_all(clickhouse, &batch, &compiled),
        metric_versions(db, &keys)
    );

    let results = batch
        .queries
        .iter()
        .zip(observed?)
        .map(|(query, observations)| DistributionResult {
            metric: query.metric_key.clone(),
            provenance: provenance(&versions, &query.metric_key),
            subjects: subject_distributions(
                &query.subjects,
                query.bins,
                query.quantiles.as_deref(),
                observations.histogram,
                observations.quantiles,
            ),
        })
        .collect();
    Ok(DistributionsResponse { results })
}

async fn read_all(
    clickhouse: &insight_clickhouse::Client,
    batch: &ValidatedDistributions,
    compiled: &[PlannedDistribution],
) -> Result<Vec<Observations>, QueryError> {
    let mut observed = Vec::with_capacity(compiled.len());
    let pairs: Vec<(&ValidatedDistribution, &PlannedDistribution)> =
        batch.queries.iter().zip(compiled).collect();

    for chunk in pairs.chunks(QUERY_CONCURRENCY) {
        let reads = chunk
            .iter()
            .map(|(query, compiled)| observations(clickhouse, query, compiled));
        for read in futures::future::join_all(reads).await {
            observed.push(read?);
        }
    }
    Ok(observed)
}

async fn observations(
    clickhouse: &insight_clickhouse::Client,
    query: &ValidatedDistribution,
    planned: &PlannedDistribution,
) -> Result<Observations, QueryError> {
    let histogram = read::<HistogramRow>(
        clickhouse,
        planned.histogram.as_ref(),
        &format!("metric-distributions:histogram:{}", query.metric_key),
    )
    .await?;
    let quantiles = read::<QuantileRow>(
        clickhouse,
        planned.quantiles.as_ref(),
        &format!("metric-distributions:quantiles:{}", query.metric_key),
    )
    .await?;

    Ok(Observations {
        histogram,
        quantiles,
    })
}

/// A reading the question did not ask for runs no statement at all.
async fn read<T>(
    clickhouse: &insight_clickhouse::Client,
    compiled: Option<&CompiledMeasureQuery>,
    comment: &str,
) -> Result<Vec<T>, QueryError>
where
    T: serde::de::DeserializeOwned,
{
    let Some(compiled) = compiled else {
        return Ok(Vec::new());
    };
    bounded(fetch::<T>(clickhouse, compiled, comment).await?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::num::NonZeroU32;

    use chrono::NaiveDate;

    use super::super::super::fixtures::{SHIPPED_DISTRIBUTION_METRIC, offline_clickhouse};
    use super::*;

    fn validated() -> ValidatedDistribution {
        ValidatedDistribution {
            metric_key: SHIPPED_DISTRIBUTION_METRIC.to_owned(),
            subjects: vec![Uuid::from_u128(1)],
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            filters: Vec::new(),
            bins: NonZeroU32::new(10),
            quantiles: None,
        }
    }

    fn compiled() -> CompiledMeasureQuery {
        CompiledMeasureQuery {
            sql: "SELECT 1".to_owned(),
            params: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_reading_the_question_did_not_ask_for_asks_the_server_nothing() {
        let rows = read::<HistogramRow>(&offline_clickhouse(), None, "test")
            .await
            .expect("an unasked reading needs no server");

        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn one_question_that_cannot_be_read_refuses_the_whole_request() {
        let batch = ValidatedDistributions {
            queries: vec![validated(), validated()],
        };
        let planned = vec![
            PlannedDistribution {
                histogram: Some(compiled()),
                quantiles: None,
            },
            PlannedDistribution {
                histogram: Some(compiled()),
                quantiles: None,
            },
        ];

        let outcome = read_all(&offline_clickhouse(), &batch, &planned).await;

        assert!(matches!(
            outcome.expect_err("a request is answered whole or not at all"),
            QueryError::ReadFailed
        ));
    }
}
