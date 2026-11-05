use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScheduleRequest {
    #[schema(min_items = 1, max_items = 10)]
    pub crons: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct RequestedSchedule {
    pub crons: Vec<String>,
    pub updated_by: Uuid,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct RecordedSchedule {
    pub crons: Vec<String>,
    pub timezone: Option<String>,
    pub origin: String,
    pub suspended: bool,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScheduleStatus {
    Applied,
    Pending,
    NotApplied,
    Inherited,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ScheduleResponse {
    pub connector: String,
    pub source_id: String,
    pub editable: bool,
    pub revision: u64,
    pub requested: Option<RequestedSchedule>,
    pub applied: Option<RecordedSchedule>,
    pub status: ScheduleStatus,
    pub not_applied_since: Option<DateTime<Utc>>,
    pub reconcile_schedules: Vec<String>,
    #[schema(minimum = 1, maximum = 1440)]
    pub min_interval_minutes: u16,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ReconcileSchedule {
    pub connector: String,
    pub source_id: String,
    pub crons: Vec<String>,
    pub revision: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ReconcileResponse {
    pub version: u8,
    pub schedules: Vec<ReconcileSchedule>,
}

impl toolkit::api::api_dto::RequestApiDto for ScheduleRequest {}
impl toolkit::api::api_dto::ResponseApiDto for ScheduleResponse {}
impl toolkit::api::api_dto::ResponseApiDto for ReconcileResponse {}

pub(super) fn status(
    requested: Option<&[String]>,
    applied: Option<&RecordedSchedule>,
) -> ScheduleStatus {
    status_for_crons(
        requested,
        applied
            .filter(|schedule| schedule.origin == "api")
            .map(|schedule| schedule.crons.as_slice()),
    )
}

pub(super) fn status_for_crons(
    requested: Option<&[String]>,
    api: Option<&[String]>,
) -> ScheduleStatus {
    let Some(requested) = requested.filter(|crons| !crons.is_empty()) else {
        return if api.is_some() {
            ScheduleStatus::Pending
        } else {
            ScheduleStatus::Inherited
        };
    };
    if api.is_some_and(|crons| {
        crons.len() == requested.len()
            && crons
                .iter()
                .zip(requested)
                .all(|(a, b)| super::validation::normalize(a) == super::validation::normalize(b))
    }) {
        ScheduleStatus::Applied
    } else {
        ScheduleStatus::Pending
    }
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct ScheduleRequestState {
    pub crons: Vec<String>,
    pub status: ScheduleStatus,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Uuid,
    pub not_applied_since: Option<DateTime<Utc>>,
}

pub(super) fn observed_status(
    requested: Option<&RequestedSchedule>,
    applied: Option<&RecordedSchedule>,
    completed_passes: &[DateTime<Utc>],
) -> (ScheduleStatus, Option<DateTime<Utc>>) {
    let status = status(requested.map(|request| request.crons.as_slice()), applied);
    after_pass(status, requested, completed_passes)
}

pub(super) fn after_pass(
    status: ScheduleStatus,
    requested: Option<&RequestedSchedule>,
    completed_passes: &[DateTime<Utc>],
) -> (ScheduleStatus, Option<DateTime<Utc>>) {
    let failed_pass = requested.and_then(|request| {
        let [latest, previous, ..] = completed_passes else {
            return None;
        };
        (*previous > request.updated_at).then_some(*latest)
    });
    if status == ScheduleStatus::Pending && failed_pass.is_some() {
        (ScheduleStatus::NotApplied, failed_pass)
    } else {
        (status, None)
    }
}

#[cfg(test)]
mod tests;
