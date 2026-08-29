use uuid::Uuid;

use crate::infra::identity::IdentityProfile;

use super::columns::{PlannedColumn, plan_columns};
use super::period::{PlannedPeriod, ReportBucket, count_periods, enumerate_periods};
use super::validation::{ReportSubjectSelection, ValidatedReportRecipe};

pub(crate) const METRIC_QUERY_VALUE_LIMIT: usize = 5000;
pub(crate) const MAX_REPORT_PERIODS: usize = METRIC_QUERY_VALUE_LIMIT - 1;
const XLSX_MAX_ROWS: u64 = 1_048_576;
const XLSX_MAX_COLUMNS: u64 = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReportPlannerLimits {
    pub(crate) max_batch_cells: usize,
    pub(crate) max_total_cells: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReportSize {
    pub(crate) total_rows: u64,
    pub(crate) total_cells: u64,
    pub(crate) worksheet_rows: u64,
    pub(crate) worksheet_columns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct XlsxDimensions {
    pub(crate) rows: u32,
    pub(crate) columns: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersonQueryBatch {
    pub(crate) person_start: usize,
    pub(crate) person_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeriodQueryBatch {
    pub(crate) period_start: usize,
    pub(crate) period_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlannedReportSubject {
    People {
        ids: Vec<Uuid>,
        batches: Vec<PersonQueryBatch>,
    },
    Tenant {
        id: Uuid,
        batches: Vec<PeriodQueryBatch>,
    },
}

#[derive(Debug)]
pub(crate) struct ReportPlan {
    pub(crate) bucket: ReportBucket,
    pub(crate) periods: Vec<PlannedPeriod>,
    pub(crate) columns: Vec<PlannedColumn>,
    pub(crate) subject: PlannedReportSubject,
    pub(crate) size: ReportSize,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum ReportPlanningError {
    #[error("hydrated profiles do not match report subject")]
    ProfileSetMismatch,
    #[error("report size exceeds supported arithmetic")]
    SizeOverflow,
    #[error("report batch limits cannot fit one person")]
    BatchLimitTooSmall,
    #[error("report exceeds XLSX worksheet dimensions")]
    XlsxDimensionsExceeded,
    #[error("report exceeds the configured cell limit")]
    CellLimitExceeded,
    #[error("report exceeds the supported period limit")]
    PeriodLimitExceeded,
}

impl ReportSize {
    pub(crate) fn xlsx_dimensions(self) -> Result<XlsxDimensions, ReportPlanningError> {
        if self.worksheet_rows > XLSX_MAX_ROWS || self.worksheet_columns > XLSX_MAX_COLUMNS {
            return Err(ReportPlanningError::XlsxDimensionsExceeded);
        }

        let rows = u32::try_from(self.worksheet_rows)
            .map_err(|_| ReportPlanningError::XlsxDimensionsExceeded)?;
        let columns = u16::try_from(self.worksheet_columns)
            .map_err(|_| ReportPlanningError::XlsxDimensionsExceeded)?;

        Ok(XlsxDimensions { rows, columns })
    }
}

pub(crate) fn plan_report(
    recipe: &ValidatedReportRecipe,
    profiles: &[IdentityProfile],
    limits: ReportPlannerLimits,
) -> Result<ReportPlan, ReportPlanningError> {
    validate_profiles(&recipe.subject, profiles)?;

    let bucket = ReportBucket::from(recipe.granularity);
    let columns = plan_columns(profiles, &recipe.metrics);
    let subject_count = match &recipe.subject {
        ReportSubjectSelection::People { ids } => ids.len(),
        ReportSubjectSelection::Tenant { .. } => 1,
    };
    let period_count =
        count_periods(recipe.from, recipe.to, bucket).ok_or(ReportPlanningError::SizeOverflow)?;
    if period_count > MAX_REPORT_PERIODS {
        return Err(ReportPlanningError::PeriodLimitExceeded);
    }
    let size = calculate_size(subject_count, period_count, columns.len())?;
    if size.total_cells > limits.max_total_cells {
        return Err(ReportPlanningError::CellLimitExceeded);
    }
    let periods = enumerate_periods(recipe.from, recipe.to, bucket);
    let subject = match &recipe.subject {
        ReportSubjectSelection::People { ids } => PlannedReportSubject::People {
            ids: ids.clone(),
            batches: plan_person_batches(ids.len(), periods.len(), recipe.metrics.len(), limits)?,
        },
        ReportSubjectSelection::Tenant { id } => PlannedReportSubject::Tenant {
            id: *id,
            batches: plan_period_batches(periods.len(), recipe.metrics.len(), limits)?,
        },
    };

    Ok(ReportPlan {
        bucket,
        periods,
        columns,
        subject,
        size,
    })
}

fn plan_period_batches(
    periods: usize,
    metrics: usize,
    limits: ReportPlannerLimits,
) -> Result<Vec<PeriodQueryBatch>, ReportPlanningError> {
    let cell_periods = limits
        .max_batch_cells
        .checked_div(metrics)
        .unwrap_or(usize::MAX);
    let periods_per_batch = MAX_REPORT_PERIODS.min(cell_periods);
    if periods_per_batch == 0 {
        return Err(ReportPlanningError::BatchLimitTooSmall);
    }

    Ok((0..periods)
        .step_by(periods_per_batch)
        .map(|period_start| PeriodQueryBatch {
            period_start,
            period_end: periods.min(period_start + periods_per_batch),
        })
        .collect())
}

fn validate_profiles(
    subject: &ReportSubjectSelection,
    profiles: &[IdentityProfile],
) -> Result<(), ReportPlanningError> {
    let matches = match subject {
        ReportSubjectSelection::People { ids } => {
            ids.len() == profiles.len()
                && ids
                    .iter()
                    .zip(profiles)
                    .all(|(requested, profile)| requested == &profile.person_id)
        }
        ReportSubjectSelection::Tenant { .. } => profiles.is_empty(),
    };

    if matches {
        Ok(())
    } else {
        Err(ReportPlanningError::ProfileSetMismatch)
    }
}

fn calculate_size(
    subject_count: usize,
    period_count: usize,
    column_count: usize,
) -> Result<ReportSize, ReportPlanningError> {
    let subject_count =
        u64::try_from(subject_count).map_err(|_| ReportPlanningError::SizeOverflow)?;
    let period_count =
        u64::try_from(period_count).map_err(|_| ReportPlanningError::SizeOverflow)?;
    let column_count =
        u64::try_from(column_count).map_err(|_| ReportPlanningError::SizeOverflow)?;
    let total_rows = subject_count
        .checked_mul(period_count)
        .ok_or(ReportPlanningError::SizeOverflow)?;
    let total_cells = total_rows
        .checked_mul(column_count)
        .ok_or(ReportPlanningError::SizeOverflow)?;
    let worksheet_rows = total_rows
        .checked_add(1)
        .ok_or(ReportPlanningError::SizeOverflow)?;

    Ok(ReportSize {
        total_rows,
        total_cells,
        worksheet_rows,
        worksheet_columns: column_count,
    })
}

fn plan_person_batches(
    people: usize,
    periods: usize,
    metrics: usize,
    limits: ReportPlannerLimits,
) -> Result<Vec<PersonQueryBatch>, ReportPlanningError> {
    let query_values_per_person = periods
        .checked_add(1)
        .ok_or(ReportPlanningError::SizeOverflow)?;
    let cells_per_person = periods
        .checked_mul(metrics)
        .ok_or(ReportPlanningError::SizeOverflow)?;
    let query_people = METRIC_QUERY_VALUE_LIMIT / query_values_per_person;
    let cell_people = limits
        .max_batch_cells
        .checked_div(cells_per_person)
        .unwrap_or(usize::MAX);
    let people_per_batch = query_people.min(cell_people);
    if people_per_batch == 0 {
        return Err(ReportPlanningError::BatchLimitTooSmall);
    }

    Ok((0..people)
        .step_by(people_per_batch)
        .map(|person_start| PersonQueryBatch {
            person_start,
            person_end: people.min(person_start + people_per_batch),
        })
        .collect())
}
