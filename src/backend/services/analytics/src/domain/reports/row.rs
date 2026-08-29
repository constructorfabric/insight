use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::infra::identity::IdentityProfile;

use super::columns::{PlannedColumn, PlannedColumnSource, profile_text};
use super::period::PlannedPeriod;

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(untagged)]
pub enum ReportCell {
    Text(String),
    Number(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(transparent)]
pub struct ReportRow(Vec<Option<ReportCell>>);

impl std::ops::Deref for ReportRow {
    type Target = [Option<ReportCell>];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<Option<ReportCell>>> for ReportRow {
    fn from(cells: Vec<Option<ReportCell>>) -> Self {
        Self(cells)
    }
}

impl FromIterator<Option<ReportCell>> for ReportRow {
    fn from_iter<T: IntoIterator<Item = Option<ReportCell>>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a ReportRow {
    type Item = &'a Option<ReportCell>;
    type IntoIter = std::slice::Iter<'a, Option<ReportCell>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReportMetricValue {
    pub(crate) entity_id: Uuid,
    pub(crate) bucket_start: NaiveDate,
    pub(crate) value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReportMetricValues {
    pub(crate) metric_index: usize,
    pub(crate) values: Vec<ReportMetricValue>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum ReportRowError {
    #[error("metric result shape does not match report plan")]
    MetricShapeMismatch,
}

pub(crate) fn assemble_people_rows(
    columns: &[PlannedColumn],
    periods: &[PlannedPeriod],
    profiles: &[IdentityProfile],
    metrics: &[ReportMetricValues],
) -> Result<Vec<ReportRow>, ReportRowError> {
    let entity_ids = profiles
        .iter()
        .map(|profile| profile.person_id)
        .collect::<HashSet<_>>();
    let metric_values = index_metric_values(columns, periods, &entity_ids, metrics)?;
    let mut rows = Vec::with_capacity(profiles.len().saturating_mul(periods.len()));

    for profile in profiles {
        for period in periods {
            rows.push(assemble_row(
                columns,
                period,
                Some(profile),
                profile.person_id,
                &metric_values,
            ));
        }
    }

    Ok(rows)
}

pub(crate) fn assemble_tenant_rows(
    columns: &[PlannedColumn],
    periods: &[PlannedPeriod],
    tenant_id: Uuid,
    metrics: &[ReportMetricValues],
) -> Result<Vec<ReportRow>, ReportRowError> {
    let entity_ids = HashSet::from([tenant_id]);
    let metric_values = index_metric_values(columns, periods, &entity_ids, metrics)?;

    Ok(periods
        .iter()
        .map(|period| assemble_row(columns, period, None, tenant_id, &metric_values))
        .collect())
}

type MetricValueIndex = Vec<HashMap<(Uuid, NaiveDate), Option<f64>>>;

fn index_metric_values(
    columns: &[PlannedColumn],
    periods: &[PlannedPeriod],
    entity_ids: &HashSet<Uuid>,
    metrics: &[ReportMetricValues],
) -> Result<MetricValueIndex, ReportRowError> {
    let metric_count = columns
        .iter()
        .filter_map(|column| match column.source {
            PlannedColumnSource::Metric(index) => Some(index + 1),
            PlannedColumnSource::PersonDisplay
            | PlannedColumnSource::PersonAttribute(_)
            | PlannedColumnSource::SupervisorDisplay
            | PlannedColumnSource::SupervisorAttribute(_)
            | PlannedColumnSource::PeriodLabel
            | PlannedColumnSource::PeriodFrom
            | PlannedColumnSource::PeriodTo => None,
        })
        .max()
        .unwrap_or_default();
    if metrics.len() != metric_count {
        return Err(ReportRowError::MetricShapeMismatch);
    }

    let bucket_starts = periods
        .iter()
        .map(|period| period.bucket_start)
        .collect::<HashSet<_>>();
    let mut indexed = (0..metric_count)
        .map(|_| None)
        .collect::<Vec<Option<HashMap<_, _>>>>();
    for metric in metrics {
        let Some(slot) = indexed.get_mut(metric.metric_index) else {
            return Err(ReportRowError::MetricShapeMismatch);
        };
        if slot.is_some() {
            return Err(ReportRowError::MetricShapeMismatch);
        }

        let mut values = HashMap::with_capacity(metric.values.len());
        for value in &metric.values {
            if !entity_ids.contains(&value.entity_id)
                || !bucket_starts.contains(&value.bucket_start)
                || value.value.is_some_and(|number| !number.is_finite())
                || values
                    .insert((value.entity_id, value.bucket_start), value.value)
                    .is_some()
            {
                return Err(ReportRowError::MetricShapeMismatch);
            }
        }
        *slot = Some(values);
    }

    indexed
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or(ReportRowError::MetricShapeMismatch)
}

fn assemble_row(
    columns: &[PlannedColumn],
    period: &PlannedPeriod,
    profile: Option<&IdentityProfile>,
    entity_id: Uuid,
    metrics: &MetricValueIndex,
) -> ReportRow {
    columns
        .iter()
        .map(|column| match &column.source {
            PlannedColumnSource::PersonDisplay
            | PlannedColumnSource::PersonAttribute(_)
            | PlannedColumnSource::SupervisorDisplay
            | PlannedColumnSource::SupervisorAttribute(_) => profile
                .and_then(|profile| profile_text(&column.source, profile))
                .map(text),
            PlannedColumnSource::PeriodLabel => Some(text(&period.label)),
            PlannedColumnSource::PeriodFrom => Some(text(&period.from.to_string())),
            PlannedColumnSource::PeriodTo => Some(text(&period.to.to_string())),
            PlannedColumnSource::Metric(index) => metrics[*index]
                .get(&(entity_id, period.bucket_start))
                .copied()
                .flatten()
                .map(ReportCell::Number),
        })
        .collect()
}

fn text(value: &str) -> ReportCell {
    ReportCell::Text(value.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::NaiveDate;
    use uuid::Uuid;

    use super::*;
    use crate::domain::reports::columns::{
        PlannedColumn, PlannedColumnSource, ReportColumnDataType, ReportColumnMetadata,
    };
    use crate::domain::reports::period::PlannedPeriod;
    use crate::infra::identity::IdentityProfile;

    #[test]
    fn people_rows_follow_person_then_period_order_and_leave_missing_metrics_empty() {
        let profiles = [profile(2, "Second"), profile(1, "First")];
        let periods = [
            period("2026-01", "2026-01-01"),
            period("2026-02", "2026-02-01"),
        ];
        let columns = [
            column("person", PlannedColumnSource::PersonDisplay),
            column("period", PlannedColumnSource::PeriodLabel),
            column("metric.one", PlannedColumnSource::Metric(0)),
        ];
        let values = [metric_values(
            0,
            &[(2, "2026-01-01", Some(2.0)), (1, "2026-02-01", Some(1.0))],
        )];

        let rows = assemble_people_rows(&columns, &periods, &profiles, &values)
            .unwrap_or_else(|error| panic!("rows should assemble: {error}"));

        assert_eq!(
            rows,
            vec![
                vec![text("Second"), text("2026-01"), number(2.0)].into(),
                vec![text("Second"), text("2026-02"), None].into(),
                vec![text("First"), text("2026-01"), None].into(),
                vec![text("First"), text("2026-02"), number(1.0)].into(),
            ]
        );
    }

    #[test]
    fn rejects_non_finite_metric_values() {
        let profiles = [profile(1, "First")];
        let periods = [period("2026-01", "2026-01-01")];
        let columns = [column("metric.one", PlannedColumnSource::Metric(0))];
        let values = [metric_values(0, &[(1, "2026-01-01", Some(f64::NAN))])];

        assert_eq!(
            assemble_people_rows(&columns, &periods, &profiles, &values),
            Err(ReportRowError::MetricShapeMismatch)
        );
    }

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .unwrap_or_else(|error| panic!("fixture date must parse: {error}"))
    }

    fn profile(id: u128, display_name: &str) -> IdentityProfile {
        IdentityProfile {
            person_id: Uuid::from_u128(id),
            attributes: BTreeMap::from([("display_name".to_owned(), display_name.to_owned())]),
            supervisor: None,
        }
    }

    fn period(label: &str, bucket_start: &str) -> PlannedPeriod {
        let date = date(bucket_start);
        PlannedPeriod {
            label: label.to_owned(),
            bucket_start: date,
            from: date,
            to: date,
        }
    }

    fn column(key: &str, source: PlannedColumnSource) -> PlannedColumn {
        PlannedColumn {
            metadata: ReportColumnMetadata {
                key: key.to_owned(),
                label: key.to_owned(),
                data_type: ReportColumnDataType::Text,
                format: None,
                unit: None,
            },
            source,
        }
    }

    fn metric_values(
        metric_index: usize,
        values: &[(u128, &str, Option<f64>)],
    ) -> ReportMetricValues {
        ReportMetricValues {
            metric_index,
            values: values
                .iter()
                .map(|(entity_id, bucket_start, value)| ReportMetricValue {
                    entity_id: Uuid::from_u128(*entity_id),
                    bucket_start: date(bucket_start),
                    value: *value,
                })
                .collect(),
        }
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "matches the positional row cell shape"
    )]
    fn text(value: &str) -> Option<ReportCell> {
        Some(ReportCell::Text(value.to_owned()))
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "matches the positional row cell shape"
    )]
    fn number(value: f64) -> Option<ReportCell> {
        Some(ReportCell::Number(value))
    }
}
