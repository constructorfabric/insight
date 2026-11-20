use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncStatus {
    Pending,
    Running,
    Incomplete,
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}

impl SyncStatus {
    pub(crate) fn parse(raw: &str) -> Self {
        match raw {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "incomplete" => Self::Incomplete,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }

    pub(crate) const TERMINAL: [Self; 4] = [
        Self::Succeeded,
        Self::Failed,
        Self::Cancelled,
        Self::Incomplete,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Incomplete => "incomplete",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Attention {
    Failing,
    Unreadable,
    Settled,
    NeverRan,
    NoLongerConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct InstanceKey {
    pub connector: String,
    pub tenant_id: String,
    pub source_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LastSync {
    pub job_id: String,
    pub status: SyncStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub job_updated_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub records_reported: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct ConnectorSummary {
    pub instance: InstanceKey,
    pub configured: bool,
    pub schedule: Option<AppliedSchedule>,
    pub last_sync: Option<LastSync>,
}

impl ConnectorSummary {
    pub(crate) fn attention(&self) -> Attention {
        let Some(sync) = self.last_sync.as_ref() else {
            return if self.configured {
                Attention::NeverRan
            } else {
                Attention::NoLongerConfigured
            };
        };
        if !self.configured {
            return Attention::NoLongerConfigured;
        }
        match sync.status {
            SyncStatus::Failed | SyncStatus::Incomplete => Attention::Failing,
            SyncStatus::Unknown => Attention::Unreadable,
            SyncStatus::Pending
            | SyncStatus::Running
            | SyncStatus::Succeeded
            | SyncStatus::Cancelled => Attention::Settled,
        }
    }

    fn last_activity(&self) -> Option<DateTime<Utc>> {
        let sync = self.last_sync.as_ref()?;
        sync.started_at.or(sync.job_updated_at)
    }
}

pub(crate) fn by_attention(summaries: &mut [ConnectorSummary]) {
    summaries.sort_by(|left, right| {
        left.attention()
            .cmp(&right.attention())
            .then_with(|| right.last_activity().cmp(&left.last_activity()))
            .then_with(|| left.instance.cmp(&right.instance))
    });
}

#[derive(Debug, Default)]
pub(crate) struct LedgerFacts {
    pub sealed_at: Option<DateTime<Utc>>,
    pub typical_read_interval_ms: Option<u64>,
    pub summaries: Vec<ConnectorSummary>,
    pub has_history: bool,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SyncFact {
    /// The mover's own job identity.
    pub job_id: String,
    /// The mover's own word for how the sync ended, or `unknown` where the
    /// recorded word was outside its documented vocabulary.
    pub status: String,
    /// Absent for a job the mover had not started.
    pub started_at: Option<String>,
    /// Absent for a job still in flight, and for one the mover gave no usable
    /// pair of stamps for. Never zero to mean absent.
    pub duration_ms: Option<u64>,
    /// What the mover states it moved. Absent where it reported no count at
    /// all, which is a different answer from a reported zero.
    pub records_reported: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleOrigin {
    Annotation,
    Descriptor,
    Default,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AppliedSchedule {
    /// The non-empty list recorded from the instance's `CronWorkflow`.
    #[schema(min_items = 1)]
    pub crons: Vec<String>,
    /// Absent when the `CronWorkflow` uses the Argo controller's timezone.
    pub timezone: Option<String>,
    /// Where the applied schedule came from; absent when unrecorded or unknown.
    pub origin: Option<ScheduleOrigin>,
    /// Whether the recorded `CronWorkflow` is suspended.
    pub suspended: bool,
}

impl AppliedSchedule {
    pub(crate) fn from_record(
        crons: Vec<String>,
        timezone: String,
        origin: &str,
        suspended: bool,
    ) -> Option<Self> {
        if crons.is_empty() {
            return None;
        }

        let origin = match origin {
            "annotation" => Some(ScheduleOrigin::Annotation),
            "descriptor" => Some(ScheduleOrigin::Descriptor),
            "default" => Some(ScheduleOrigin::Default),
            _ => None,
        };
        Some(Self {
            crons,
            timezone: label(timezone),
            origin,
            suspended,
        })
    }
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ConnectorHealth {
    pub connector: String,
    /// Tenant of the installation this row is. Absent together with
    /// `source_id` on history recorded before the ledger carried the identity
    /// that no single instance could be shown to own — unattributed, which is a
    /// different answer from attributed to something named "".
    pub tenant_id: Option<String>,
    /// The installation's own id within that tenant. Absent with `tenant_id`.
    pub source_id: Option<String>,
    /// Present in the newest sealed snapshot of the set the controller manages.
    pub configured: bool,
    /// Applied schedule recorded on the newest sealed tick. Absent outside its
    /// configured set or when that row records no cron expressions.
    pub schedule: Option<AppliedSchedule>,
    /// Absent for a configured connector that has never synced.
    pub last_sync: Option<SyncFact>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ConnectorHealthResponse {
    /// When this answer was computed. Dates the answer; `checked_at` dates the
    /// facts in it.
    pub as_of: String,
    /// When the mover was last read. Absent before the first sweep sealed.
    pub checked_at: Option<String>,
    /// The median gap between the recent sealed ticks. Measured, not
    /// configured — nothing on this path knows what cadence was intended.
    /// Absent where too few ticks are recorded to establish one.
    pub typical_read_interval_ms: Option<u64>,
    /// False when nothing has been recorded at all, so the page can say so
    /// instead of implying health.
    pub history_available: bool,
    pub connectors: Vec<ConnectorHealth>,
}
impl toolkit::api::api_dto::ResponseApiDto for ConnectorHealthResponse {}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SyncHistoryResponse {
    pub connector: String,
    /// Tenant of the installation the window was narrowed to, echoed back.
    /// Absent where the caller asked for the connector rather than one
    /// installation of it, in which case the window spans every instance under
    /// that name.
    pub tenant_id: Option<String>,
    /// The installation's own id within that tenant. Absent with `tenant_id`.
    pub source_id: Option<String>,
    /// A bounded window, newest first — not the full retained history.
    pub syncs: Vec<SyncFact>,
    /// How many rows this window holds at most, so the page can say the list
    /// is a window rather than everything.
    pub window: u32,
}
impl toolkit::api::api_dto::ResponseApiDto for SyncHistoryResponse {}

impl ConnectorHealthResponse {
    pub(crate) fn from_facts(facts: LedgerFacts, as_of: DateTime<Utc>) -> Self {
        Self {
            as_of: stamp(as_of),
            checked_at: facts.sealed_at.map(stamp),
            typical_read_interval_ms: facts.typical_read_interval_ms,
            history_available: facts.has_history,
            connectors: facts.summaries.into_iter().map(Into::into).collect(),
        }
    }
}

impl SyncHistoryResponse {
    pub(crate) fn build(
        connector: String,
        tenant_id: Option<String>,
        source_id: Option<String>,
        syncs: Vec<LastSync>,
        window: u32,
    ) -> Self {
        Self {
            connector,
            tenant_id,
            source_id,
            syncs: syncs.into_iter().map(Into::into).collect(),
            window,
        }
    }
}

pub(crate) fn stamp(moment: DateTime<Utc>) -> String {
    moment.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn label(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

impl From<LastSync> for SyncFact {
    fn from(sync: LastSync) -> Self {
        Self {
            job_id: sync.job_id,
            status: sync.status.as_str().to_owned(),
            started_at: sync.started_at.map(stamp),
            duration_ms: sync.duration_ms,
            records_reported: sync.records_reported,
        }
    }
}

impl From<ConnectorSummary> for ConnectorHealth {
    fn from(summary: ConnectorSummary) -> Self {
        Self {
            connector: summary.instance.connector,
            tenant_id: label(summary.instance.tenant_id),
            source_id: label(summary.instance.source_id),
            configured: summary.configured,
            schedule: summary.schedule,
            last_sync: summary.last_sync.map(Into::into),
        }
    }
}
