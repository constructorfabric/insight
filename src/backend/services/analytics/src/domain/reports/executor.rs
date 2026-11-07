use async_trait::async_trait;
use futures::StreamExt;
use futures::stream;
use uuid::Uuid;

use crate::domain::not_ready::NotReadyReason;
use crate::infra::identity::IdentityProfile;

use super::planner::{PlannedReportSubject, ReportPlan};
use super::query::{
    ReportMetricQuery, ReportMetricRead, ReportQueryError, ReportQueryRunner, ReportQuerySubject,
    compile_report_metric_query,
};
use super::row::{
    ReportMetricValues, ReportRow, ReportRowError, assemble_people_rows, assemble_tenant_rows,
};
use super::validation::ValidatedReportRecipe;

const REPORT_QUERY_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ReportExecutionContext {
    pub(crate) tenant_id: Uuid,
}

#[derive(Debug, thiserror::Error)]
#[error("report sink failed: {0}")]
pub(crate) struct ReportSinkError(pub(crate) String);

impl ReportSinkError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

#[async_trait]
pub(crate) trait ReportRowSink {
    type Output;

    async fn write_rows(&mut self, rows: &[ReportRow]) -> Result<(), ReportSinkError>;
    async fn finish(self) -> Result<Self::Output, ReportSinkError>;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReportExecutionError {
    #[error("report metric query failed")]
    Query(#[source] ReportQueryError),
    #[error("report metric result shape is invalid")]
    Row(#[source] ReportRowError),
    #[error("report plan does not match its inputs")]
    PlanMismatch,
    #[error("report output failed")]
    Sink(#[source] ReportSinkError),
}

/// A finished report, and whether the warehouse could answer all of it.
///
/// A relation nothing has built yet stops the rows there: the output carries
/// what was written before it, and `not_ready` says why there is no more.
#[derive(Debug)]
pub(crate) struct ReportExecution<T> {
    pub(crate) output: T,
    pub(crate) not_ready: Option<NotReadyReason>,
}

pub(crate) async fn execute_report<R, S>(
    recipe: &ValidatedReportRecipe,
    plan: &ReportPlan,
    profiles: &[IdentityProfile],
    context: ReportExecutionContext,
    runner: &R,
    mut sink: S,
) -> Result<ReportExecution<S::Output>, ReportExecutionError>
where
    R: ReportQueryRunner,
    S: ReportRowSink,
{
    let mut not_ready = None;
    match &plan.subject {
        PlannedReportSubject::People { ids, batches } => {
            if ids.len() != profiles.len()
                || !ids
                    .iter()
                    .zip(profiles)
                    .all(|(id, profile)| id == &profile.person_id)
            {
                return Err(ReportExecutionError::PlanMismatch);
            }
            for batch in batches {
                let batch_ids = ids
                    .get(batch.person_start..batch.person_end)
                    .ok_or(ReportExecutionError::PlanMismatch)?;
                let batch_profiles = profiles
                    .get(batch.person_start..batch.person_end)
                    .ok_or(ReportExecutionError::PlanMismatch)?;
                let values = match execute_metric_queries(
                    recipe,
                    plan,
                    &plan.periods,
                    context,
                    ReportQuerySubject::People(batch_ids.to_vec()),
                    runner,
                )
                .await?
                {
                    MetricQueryBatch::Values(values) => values,
                    MetricQueryBatch::NotReady(reason) => {
                        not_ready = Some(reason);
                        break;
                    }
                };
                let rows =
                    assemble_people_rows(&plan.columns, &plan.periods, batch_profiles, &values)
                        .map_err(ReportExecutionError::Row)?;

                sink.write_rows(&rows)
                    .await
                    .map_err(ReportExecutionError::Sink)?;
            }
        }
        PlannedReportSubject::Tenant { id, batches } => {
            if !profiles.is_empty() || *id != context.tenant_id {
                return Err(ReportExecutionError::PlanMismatch);
            }
            for batch in batches {
                let periods = plan
                    .periods
                    .get(batch.period_start..batch.period_end)
                    .ok_or(ReportExecutionError::PlanMismatch)?;
                let values = match execute_metric_queries(
                    recipe,
                    plan,
                    periods,
                    context,
                    ReportQuerySubject::Tenant(*id),
                    runner,
                )
                .await?
                {
                    MetricQueryBatch::Values(values) => values,
                    MetricQueryBatch::NotReady(reason) => {
                        not_ready = Some(reason);
                        break;
                    }
                };
                let rows = assemble_tenant_rows(&plan.columns, periods, *id, &values)
                    .map_err(ReportExecutionError::Row)?;

                sink.write_rows(&rows)
                    .await
                    .map_err(ReportExecutionError::Sink)?;
            }
        }
    }

    let output = sink.finish().await.map_err(ReportExecutionError::Sink)?;

    Ok(ReportExecution { output, not_ready })
}

/// One batch's metric reads, or the reason the batch has none.
enum MetricQueryBatch {
    Values(Vec<ReportMetricValues>),
    NotReady(NotReadyReason),
}

async fn execute_metric_queries<R: ReportQueryRunner>(
    recipe: &ValidatedReportRecipe,
    plan: &ReportPlan,
    periods: &[super::period::PlannedPeriod],
    context: ReportExecutionContext,
    subject: ReportQuerySubject,
    runner: &R,
) -> Result<MetricQueryBatch, ReportExecutionError> {
    let from = periods
        .first()
        .ok_or(ReportExecutionError::PlanMismatch)?
        .from;
    let to = periods.last().ok_or(ReportExecutionError::PlanMismatch)?.to;
    let queries = recipe
        .metrics
        .iter()
        .enumerate()
        .map(|(metric_index, metric)| {
            compile_report_metric_query(
                metric_index,
                metric,
                context.tenant_id,
                &subject,
                from,
                to,
                plan.bucket,
            )
        })
        .collect::<Vec<ReportMetricQuery>>();
    let mut results = stream::iter(queries)
        .map(|query: ReportMetricQuery| runner.run(query))
        .buffer_unordered(REPORT_QUERY_CONCURRENCY);
    let mut ordered = (0..recipe.metrics.len())
        .map(|_| None)
        .collect::<Vec<Option<ReportMetricValues>>>();
    while let Some(result) = results.next().await {
        let values = match result.map_err(ReportExecutionError::Query)? {
            ReportMetricRead::Values(values) => values,
            // One metric reading a relation nothing has built makes the whole
            // report not-ready: a table with that column silently blank would
            // read as "measured, and empty".
            ReportMetricRead::NotReady(reason) => return Ok(MetricQueryBatch::NotReady(reason)),
        };
        let Some(slot) = ordered.get_mut(values.metric_index) else {
            return Err(ReportExecutionError::PlanMismatch);
        };
        if slot.is_some() {
            return Err(ReportExecutionError::PlanMismatch);
        }
        *slot = Some(values);
    }

    ordered
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .map(MetricQueryBatch::Values)
        .ok_or(ReportExecutionError::PlanMismatch)
}
