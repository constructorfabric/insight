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

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct RecordedSchedule {
    pub crons: Vec<String>,
    pub timezone: Option<String>,
    pub origin: String,
    pub suspended: bool,
    pub revision: u64,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScheduleStatus {
    Applied,
    Pending,
    Error,
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
    pub inherited: Option<InheritedSchedule>,
    pub error: Option<ScheduleAttemptError>,
    pub reconcile_schedules: Vec<String>,
    #[schema(minimum = 30, maximum = 1440)]
    pub min_interval_minutes: u16,
}

impl toolkit::api::api_dto::RequestApiDto for ScheduleRequest {}
impl toolkit::api::api_dto::ResponseApiDto for ScheduleResponse {}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct InheritedSchedule {
    pub crons: Vec<String>,
    pub origin: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct ScheduleAttemptError {
    pub attempted_revision: u64,
    pub attempted_at: DateTime<Utc>,
    pub message: String,
}

pub(super) fn status(
    revision: u64,
    requested: Option<&[String]>,
    acknowledgement: Option<&super::Acknowledgement>,
) -> ScheduleStatus {
    let Some(ack) = acknowledgement else {
        return ScheduleStatus::Pending;
    };
    if ack.attempted_revision == revision && ack.last_error.is_some() {
        return ScheduleStatus::Error;
    }
    if ack
        .applied_revision
        .is_none_or(|applied| revision > applied)
    {
        return ScheduleStatus::Pending;
    }
    if requested.is_none_or(<[String]>::is_empty) {
        return ScheduleStatus::Inherited;
    }
    ScheduleStatus::Applied
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct ScheduleRequestState {
    pub crons: Vec<String>,
    pub revision: u64,
    pub status: ScheduleStatus,
    pub updated_at: Option<DateTime<Utc>>,
    pub updated_by: Option<Uuid>,
    pub applied: Option<RecordedSchedule>,
    pub inherited: Option<InheritedSchedule>,
    pub error: Option<ScheduleAttemptError>,
}

#[cfg(test)]
mod tests;
