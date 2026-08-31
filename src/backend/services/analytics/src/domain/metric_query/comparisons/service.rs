//! One request, end to end: the questions are planned together, read with a
//! bounded number in flight, and assembled in the order they were asked.
//!
//! INVARIANT: a request is answered whole or refused whole.

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use super::super::catalog::MetricCatalog;
use super::super::dto::ServedFrom;
use super::super::error::QueryError;
use super::super::execute::fetch;
use super::super::provenance::{metric_versions, provenance};
use super::super::question::bounded;
use super::assemble::{ComparisonRow, target_comparisons};
use super::dto::{ComparisonResult, ComparisonsResponse};
use super::plan::{PlannedComparison, plan};
use super::validation::{ValidatedComparison, ValidatedComparisons};

/// How many reads this endpoint keeps in flight for one request.
const QUERY_CONCURRENCY: usize = 4;

pub async fn answer(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    db: &DatabaseConnection,
    tenant_id: Uuid,
    batch: ValidatedComparisons,
) -> Result<ComparisonsResponse, QueryError> {
    let compiled = plan(catalog, clickhouse, tenant_id, &batch).await?;

    let keys: Vec<String> = batch
        .queries
        .iter()
        .map(|query| query.metric_key.clone())
        .collect();
    let (answered, versions) = tokio::join!(
        read_all(clickhouse, &batch, &compiled),
        metric_versions(db, &keys)
    );

    let results = batch
        .queries
        .iter()
        .zip(answered?)
        .map(|(query, rows)| ComparisonResult {
            metric: query.metric_key.clone(),
            provenance: provenance(&versions, &query.metric_key, ServedFrom::Computed),
            targets: target_comparisons(rows, &query.targets),
        })
        .collect();
    Ok(ComparisonsResponse { results })
}

async fn read_all(
    clickhouse: &insight_clickhouse::Client,
    batch: &ValidatedComparisons,
    compiled: &[PlannedComparison],
) -> Result<Vec<Vec<ComparisonRow>>, QueryError> {
    let mut answered = Vec::with_capacity(compiled.len());
    let pairs: Vec<(&ValidatedComparison, &PlannedComparison)> =
        batch.queries.iter().zip(compiled).collect();

    for chunk in pairs.chunks(QUERY_CONCURRENCY) {
        let reads = chunk
            .iter()
            .map(|(query, compiled)| rows(clickhouse, query, compiled));
        for read in futures::future::join_all(reads).await {
            answered.push(read?);
        }
    }
    Ok(answered)
}

/// INVARIANT: a comparison no identity resolves for reads exactly as a scan
/// that matched nothing, so both are assembled into the same answer: every
/// target reported, unobserved, over a population of nobody.
async fn rows(
    clickhouse: &insight_clickhouse::Client,
    query: &ValidatedComparison,
    planned: &PlannedComparison,
) -> Result<Vec<ComparisonRow>, QueryError> {
    let PlannedComparison::Read(compiled) = planned else {
        return Ok(Vec::new());
    };

    let comment = format!("metric-comparisons:{}", query.metric_key);
    bounded(fetch::<ComparisonRow>(clickhouse, compiled, &comment).await?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use crate::domain::compiler::sql::CompiledMeasureQuery;

    use super::super::super::fixtures::{SHIPPED_METRIC, offline_clickhouse};
    use super::super::dto::{PopulationSpread, TargetComparison};
    use super::super::validation::ValidatedPopulation;
    use super::*;

    fn validated() -> ValidatedComparison {
        ValidatedComparison {
            metric_key: SHIPPED_METRIC.to_owned(),
            targets: vec![Uuid::from_u128(1)],
            population: ValidatedPopulation::Tenant,
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            filters: Vec::new(),
        }
    }

    fn compiled() -> PlannedComparison {
        PlannedComparison::Read(CompiledMeasureQuery {
            sql: "SELECT 1".to_owned(),
            params: Vec::new(),
        })
    }

    /// A comparison the mapping knows no identity for anybody in reads as no
    /// data, not as a bad request: every target is reported, unobserved, over a
    /// population of nobody.
    #[tokio::test]
    async fn a_comparison_no_identity_resolves_for_is_answered_empty_without_a_read() {
        let query = validated();

        let read = rows(
            &offline_clickhouse(),
            &query,
            &PlannedComparison::NoIdentities,
        )
        .await
        .expect("a comparison nobody's identities reach is answerable");

        assert_eq!(
            target_comparisons(read, &query.targets),
            vec![TargetComparison {
                subject: query.targets[0].to_string(),
                value: None,
                population: PopulationSpread {
                    n: 0,
                    p25: None,
                    median: None,
                    p75: None,
                    min: None,
                    max: None,
                },
            }]
        );
    }

    #[tokio::test]
    async fn one_question_that_cannot_be_read_refuses_the_whole_request() {
        let batch = ValidatedComparisons {
            queries: vec![validated(), validated()],
        };

        let outcome = read_all(&offline_clickhouse(), &batch, &[compiled(), compiled()]).await;

        assert!(matches!(
            outcome.expect_err("a request is answered whole or not at all"),
            QueryError::ReadFailed
        ));
    }
}
