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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScheduleOrigin {
    Api,
    Annotation,
    Descriptor,
    Default,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct LiveSchedule {
    pub crons: Vec<String>,
    pub timezone: Option<String>,
    pub origin: Option<ScheduleOrigin>,
    pub suspended: bool,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScheduleStatus {
    Applied,
    Pending,
    Inherited,
    Paused,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PendingReason {
    NoCronworkflow,
    AwaitingReconcile,
    DifferentSchedule,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ScheduleResponse {
    pub connector: String,
    pub source_id: String,
    pub revision: u64,
    pub requested: Option<RequestedSchedule>,
    pub live: Option<LiveSchedule>,
    pub inherited: Option<InheritedSchedule>,
    pub status: Option<ScheduleStatus>,
    pub read_error: Option<String>,
    pub pending_reason: Option<PendingReason>,
    pub next_reconcile_at: Option<DateTime<Utc>>,
    pub reconcile_interval_secs: Option<i64>,
}

impl toolkit::api::api_dto::RequestApiDto for ScheduleRequest {}
impl toolkit::api::api_dto::ResponseApiDto for ScheduleResponse {}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct InheritedSchedule {
    pub crons: Vec<String>,
    pub origin: ScheduleOrigin,
}

pub(crate) fn status(
    requested: &[String],
    live: Option<&LiveSchedule>,
    inherited: Option<&InheritedSchedule>,
) -> (ScheduleStatus, Option<PendingReason>) {
    if let Some(live) = live {
        if live.suspended {
            return (ScheduleStatus::Paused, None);
        }
        let matches = live.crons == requested
            && live.timezone.as_deref() == Some("UTC")
            && live.origin == Some(ScheduleOrigin::Api);
        if !requested.is_empty() && matches {
            return (ScheduleStatus::Applied, None);
        }
        if requested.is_empty()
            && inherited.is_some_and(|target| {
                target.origin != ScheduleOrigin::Api
                    && live.origin == Some(target.origin)
                    && live.crons == target.crons
                    && live.timezone.as_deref() == Some("UTC")
            })
        {
            return (ScheduleStatus::Inherited, None);
        }
    }
    let reason = if live.is_none() {
        PendingReason::NoCronworkflow
    } else if requested.is_empty() && inherited.is_none() {
        PendingReason::AwaitingReconcile
    } else {
        PendingReason::DifferentSchedule
    };
    (ScheduleStatus::Pending, Some(reason))
}
