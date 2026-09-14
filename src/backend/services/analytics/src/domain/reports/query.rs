use async_trait::async_trait;
use chrono::NaiveDate;
use uuid::Uuid;

use crate::domain::metric_definitions::MetricDefinition;
use crate::domain::metric_results::ValidatedEntitySelection;
use crate::domain::metric_results::compiler::{
    CompiledQuery, QueryBucket, TimeseriesQueryRow, compile_report_timeseries_query,
};
use crate::domain::metric_results::query_row_limit;
use crate::infra::metrics::QueryKind;
use crate::infra::query::{QueryFetchError, fetch_json_rows};

use super::period::ReportBucket;
#[cfg(test)]
use super::period::containing_bucket_start;
use super::row::{ReportMetricValue, ReportMetricValues};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReportQuerySubject {
    People(Vec<Uuid>),
    Tenant(Uuid),
}

#[derive(Debug)]
pub(crate) struct ReportMetricQuery {
    pub(crate) metric_index: usize,
    pub(crate) metric_key: String,
    #[cfg(test)]
    pub(crate) subject: ReportQuerySubject,
    #[cfg(test)]
    pub(crate) first_bucket_start: NaiveDate,
    #[cfg(test)]
    pub(crate) last_bucket_start: NaiveDate,
    pub(crate) compiled: CompiledQuery,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReportQueryError {
    #[error("metric query failed")]
    Fetch(#[source] QueryFetchError),
    #[error("metric query returned an invalid result shape")]
    InvalidResultShape,
}

#[async_trait]
pub(crate) trait ReportQueryRunner: Sync {
    async fn run(&self, query: ReportMetricQuery) -> Result<ReportMetricValues, ReportQueryError>;
}

pub(crate) struct ClickHouseReportQueryRunner<'a> {
    client: &'a insight_clickhouse::Client,
}

impl<'a> ClickHouseReportQueryRunner<'a> {
    pub(crate) fn new(client: &'a insight_clickhouse::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ReportQueryRunner for ClickHouseReportQueryRunner<'_> {
    async fn run(&self, query: ReportMetricQuery) -> Result<ReportMetricValues, ReportQueryError> {
        let log_comment = format!("report:timeseries:{}", query.metric_key);
        let rows = fetch_json_rows::<TimeseriesQueryRow>(
            self.client,
            &query.compiled.sql,
            &query.compiled.params,
            QueryKind::Report,
            &log_comment,
        )
        .await
        .map_err(ReportQueryError::Fetch)?;
        if rows.len() >= query_row_limit() {
            return Err(ReportQueryError::InvalidResultShape);
        }
        let mut values = Vec::new();
        for row in rows {
            if row.is_total == 1 {
                continue;
            }
            if row.is_total != 0
                || row.rank.is_some()
                || row.remainder != 0
                || row.group_label.is_some()
                || !row.extra.is_empty()
            {
                return Err(ReportQueryError::InvalidResultShape);
            }
            let entity_id = row
                .entity_id
                .parse()
                .map_err(|_| ReportQueryError::InvalidResultShape)?;
            let bucket_start = NaiveDate::parse_from_str(&row.bucket_start, "%Y-%m-%d")
                .map_err(|_| ReportQueryError::InvalidResultShape)?;

            values.push(ReportMetricValue {
                entity_id,
                bucket_start,
                value: row.value,
            });
        }

        Ok(ReportMetricValues {
            metric_index: query.metric_index,
            values,
        })
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "query scope is explicit at the compiler boundary"
)]
pub(crate) fn compile_report_metric_query(
    metric_index: usize,
    metric: &MetricDefinition,
    tenant_id: Uuid,
    subject: &ReportQuerySubject,
    from: NaiveDate,
    to: NaiveDate,
    bucket: ReportBucket,
    enforce_tenant_scope: bool,
) -> ReportMetricQuery {
    let entity = match &subject {
        ReportQuerySubject::People(ids) => ValidatedEntitySelection::Person { ids: ids.clone() },
        ReportQuerySubject::Tenant(id) => ValidatedEntitySelection::Tenant { id: *id },
    };
    #[cfg(test)]
    let first_bucket_start = containing_bucket_start(from, bucket);
    #[cfg(test)]
    let last_bucket_start = containing_bucket_start(to, bucket);
    let bucket = match bucket {
        ReportBucket::Day => QueryBucket::Day,
        ReportBucket::Week => QueryBucket::Week,
        ReportBucket::Month => QueryBucket::Month,
        ReportBucket::Quarter => QueryBucket::Quarter,
        ReportBucket::Year => QueryBucket::Year,
    };
    let compiled = compile_report_timeseries_query(
        metric,
        tenant_id,
        entity,
        from,
        to,
        enforce_tenant_scope,
        bucket,
    );

    ReportMetricQuery {
        metric_index,
        metric_key: metric.key().to_owned(),
        #[cfg(test)]
        subject: subject.clone(),
        #[cfg(test)]
        first_bucket_start,
        #[cfg(test)]
        last_bucket_start,
        compiled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::definition::{
        AliasCollapse, ComputationSpec, MetricBase, MetricDirection, MetricFormat, MetricInput,
        MetricInputRole, ObservationRelation, ObservationSource,
    };

    #[test]
    fn report_query_uses_direct_quarter_aggregation_and_preserves_parameter_order() {
        let tenant_id = Uuid::from_u128(9);
        let person_id = Uuid::from_u128(2);
        let query = compile_report_metric_query(
            3,
            &metric(),
            tenant_id,
            &ReportQuerySubject::People(vec![person_id]),
            date("2026-01-10"),
            date("2026-06-20"),
            ReportBucket::Quarter,
            true,
        );

        assert!(
            query
                .compiled
                .sql
                .contains("toString(toStartOfQuarter(assumeNotNull(metric_date))) AS bucket_start")
        );
        assert!(query.compiled.sql.contains("GROUP BY GROUPING SETS"));
        assert_eq!(
            query.compiled.params,
            [
                person_id.to_string(),
                "2026-01-10".to_owned(),
                "2026-06-20".to_owned(),
                person_id.to_string(),
                tenant_id.to_string(),
                "test".to_owned(),
                "person".to_owned(),
                "2026-01-10".to_owned(),
                "2026-06-20".to_owned(),
                "value".to_owned(),
            ]
        );
    }

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .unwrap_or_else(|error| panic!("fixture date must parse: {error}"))
    }

    fn metric() -> MetricDefinition {
        MetricDefinition {
            base: MetricBase {
                key: "test.metric".to_owned(),
                label: "Test metric".to_owned(),
                short_label: None,
                description: None,
                explanation: None,
                entity_type: "person".to_owned(),
                format: MetricFormat::Integer,
                unit: None,
                direction: MetricDirection::Neutral,
                peer_cohort_key: None,
                allowed_dimensions: vec![],
            },
            spec: ComputationSpec::Sum {
                value: MetricInput {
                    role: MetricInputRole::Value,
                    observation: ObservationSource::Managed(
                        ObservationRelation::parse("test_metric_observations")
                            .unwrap_or_else(|| panic!("fixture relation must parse")),
                    ),
                    source_key: "test".to_owned(),
                    measure_key: "value".to_owned(),
                    alias_collapse: AliasCollapse::Sum,
                },
            },
            transform: None,
        }
    }
}
