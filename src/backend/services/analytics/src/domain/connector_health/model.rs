//! The shape the page reads, and the order it reads rows in. No I/O.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

/// Every status the mover's job listing documents, plus the one this service
/// adds for a word it did not recognise.
///
/// Parsed at the boundary so nothing downstream holds a string it cannot
/// interpret. The variants carry the mover's own words rather than a
/// translation of them: this surface reports someone else's account, and
/// renaming `succeeded` to `ok` on the way in would be this service asserting
/// something the mover did not say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncStatus {
    Pending,
    Running,
    Incomplete,
    Succeeded,
    Failed,
    Cancelled,
    /// The recorded word was outside the mover's documented vocabulary. Not a
    /// failure and not a success — a state the page could not read.
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

    /// The statuses that will not change again. A job carrying one of these has
    /// said its last word, so within a job a terminal row outranks a
    /// provisional one however the clocks compare — a provisional state never
    /// closes a job, and never reopens one either.
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

/// What a row needs from the reader, worst first.
///
/// Never serialised. A verdict on the wire is a verdict some other client
/// reads differently, so the service sorts by this and ships only the facts
/// the sort was derived from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Attention {
    /// A sync that did not finish what it set out to do.
    Failing,
    /// A state the page cannot read.
    Unreadable,
    /// Something is happening, or the last thing that happened was fine.
    Settled,
    /// Configured, with nothing recorded against it yet.
    NeverRan,
    /// Absent from the newest sealed snapshot, but holding history.
    NoLongerConfigured,
}

/// One connector's newest recorded sync, as the ledger holds it.
#[derive(Debug, Clone)]
pub(crate) struct LastSync {
    pub job_id: String,
    pub status: SyncStatus,
    pub started_at: Option<DateTime<Utc>>,
    /// The axis the ledger orders jobs along — the mover's own last-update
    /// stamp for the job. Used to sort within a band and never serialised: the
    /// page states when a sync started, which is a different fact.
    pub job_updated_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub records_reported: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ConnectorSummary {
    pub connector: String,
    pub configured: bool,
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

    /// When this connector was last heard from, for ordering inside a band.
    fn last_activity(&self) -> Option<DateTime<Utc>> {
        let sync = self.last_sync.as_ref()?;
        sync.started_at.or(sync.job_updated_at)
    }
}

/// Worst first; inside a band, most recent activity first.
///
/// A connector with no activity at all sorts after one that has some, in both
/// bands where that can happen — the alternative is a row with nothing in it
/// leading a band it shares with rows that do.
pub(crate) fn by_attention(summaries: &mut [ConnectorSummary]) {
    summaries.sort_by(|left, right| {
        left.attention()
            .cmp(&right.attention())
            // Descending, and `None` is the smallest `Option` — so a row with
            // no activity lands at the end of its band rather than the front.
            .then_with(|| right.last_activity().cmp(&left.last_activity()))
            .then_with(|| left.connector.cmp(&right.connector))
    });
}

/// What the summary read found.
///
/// An absent ledger yields the same shape as an empty one with `has_history`
/// false, so the caller has one path rather than a special case for an install
/// whose migration has not run.
#[derive(Debug, Default)]
pub(crate) struct LedgerFacts {
    pub sealed_at: Option<DateTime<Utc>>,
    pub typical_read_interval_ms: Option<u64>,
    pub summaries: Vec<ConnectorSummary>,
    pub has_history: bool,
}

// ── the wire ───────────────────────────────────────────────────────────────

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
pub struct ConnectorHealth {
    pub connector: String,
    /// Present in the newest sealed snapshot of the set the controller manages.
    pub configured: bool,
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
    /// A bounded window, newest first — not the full retained history.
    pub syncs: Vec<SyncFact>,
    /// How many rows this window holds at most, so the page can say the list
    /// is a window rather than everything.
    pub window: u32,
}
impl toolkit::api::api_dto::ResponseApiDto for SyncHistoryResponse {}

impl ConnectorHealthResponse {
    /// Assembles the answer from recorded facts and the reader's own clock.
    ///
    /// `as_of` is passed in rather than read here so the assembly is a function
    /// of its inputs — and so a test can state the instant the page was built
    /// instead of racing it.
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
    pub(crate) fn build(connector: String, syncs: Vec<LastSync>, window: u32) -> Self {
        Self {
            connector,
            syncs: syncs.into_iter().map(Into::into).collect(),
            window,
        }
    }
}

pub(crate) fn stamp(moment: DateTime<Utc>) -> String {
    moment.to_rfc3339_opts(SecondsFormat::Millis, true)
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
            connector: summary.connector,
            configured: summary.configured,
            last_sync: summary.last_sync.map(Into::into),
        }
    }
}
