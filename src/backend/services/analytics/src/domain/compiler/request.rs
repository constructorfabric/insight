//! What a caller asks a measure for: the tenancy and entity scope to read
//! under, the inclusive date window, the time grain, and the dimension
//! narrowing and breakdown. Everything here is already resolved — the compiler
//! translates, it does not look anything up.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// The time grain a measure is folded to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Day,
    Week,
    Month,
}

/// Which entities the read may see, beyond tenancy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityScope {
    /// Every entity in the tenant; tenancy is the only row predicate.
    Tenant,
    /// Source-identity values matched against the measure's entity field.
    /// Resolving person identifiers into these belongs above the compiler.
    Identities(Vec<String>),
}

/// Narrows a read to named values of one of the measure's declared dimensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DimensionFilter {
    pub key: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureQuery {
    pub tenant_id: String,
    pub entity_scope: EntityScope,
    /// Inclusive, compared against the measure's event time taken as a date.
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub bucket: Bucket,
    pub dimension_filters: Vec<DimensionFilter>,
    /// The measure dimension key to break the result down by.
    pub group_by: Option<String>,
    /// Row ceiling for the statement, bound rather than written into the SQL.
    pub row_limit: u64,
}

/// The row shape a metric read produces, together with the inputs that shape
/// needs. A view whose row shape is fully determined by the request's window
/// and scope carries nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewKind {
    /// One value per entity over the whole window; the bucket is not read.
    Period,
    /// One value per entity per bucket, plus the window total per entity.
    Timeseries(TimeseriesView),
    /// One value per entity per combination of the named dimensions.
    Breakdown(BreakdownView),
    /// One value per combination of the named dimensions, folded over every
    /// entity the scope admits.
    Rollup(RollupView),
    /// Per-entity bin counts over the measure's per-row values.
    Histogram,
    /// One row per target: the target's own value beside its peers' spread.
    Peer(PeerView),
}

impl ViewKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Period => "period",
            Self::Timeseries(_) => "timeseries",
            Self::Breakdown(_) => "breakdown",
            Self::Rollup(_) => "rollup",
            Self::Histogram => "histogram",
            Self::Peer(_) => "peer",
        }
    }
}

/// Dimension keys of the metric's grain measure, in the order their
/// `dim_{index}_value` / `dim_{index}_label` columns take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakdownView {
    pub dimensions: Vec<String>,
}

/// A timeseries is grouped by bucket before anything else, so a broken-down
/// one reports a series per dimension combination rather than one series. It
/// carries no dimensions when the whole entity is the series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeseriesView {
    pub dimensions: Vec<String>,
    /// Keeps only the named groups, with every other group folded into one
    /// remainder series. Absent means every group is reported. A cap needs a
    /// dimension to rank groups of.
    pub group_limit: Option<GroupLimit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollupView {
    pub dimensions: Vec<String>,
    /// Keeps only the named groups, with every other group folded into one
    /// remainder row. Absent means every group is reported.
    pub group_limit: Option<GroupLimit>,
}

/// The groups a capped read keeps, already ranked. Ranking them is a separate
/// read — see [`super::ranking`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupLimit {
    pub groups: Vec<RankedGroup>,
    /// Whether the rows outside the kept groups are reported as one remainder
    /// row, or dropped.
    pub include_remainder: bool,
}

/// One kept group: its position and the dimension values that select it. Rank
/// 0 is the remainder row's, so a kept group ranks from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedGroup {
    pub rank: u32,
    /// One entry per requested dimension, in the request's order.
    pub dimensions: Vec<RankedDimension>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedDimension {
    pub value: String,
    pub label: Option<String>,
}

/// The peers a distribution is taken over, and the targets it is reported for.
///
/// Entities in a cohort are named by person reference, while a dataset keys
/// its rows by source identity, so the caller resolves each pool member's
/// identities and the compiled read joins the two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerView {
    pub population: PeerPopulation,
    /// Person references the view answers for.
    pub targets: Vec<String>,
    /// Every person the read may evaluate, with their resolved identities.
    /// A target absent from the pool reads no value of its own.
    pub pool: Vec<PeerMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerPopulation {
    /// Peers are the members of the target's own declared cohort, read from
    /// the cohort relation.
    DeclaredCohort { cohort_key: String },
    /// Peers are every member of the supplied pool.
    Tenant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerMember {
    pub person_ref: String,
    pub identities: Vec<String>,
}

/// What a caller asks a metric for. A metric owns its own breakdown vocabulary
/// through the measures it composes, so a metric read is never grouped by a
/// measure dimension it does not declare — it narrows by one and folds the
/// rest away, or groups by the ones the view names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricQuery {
    pub tenant_id: String,
    /// A peer read takes its entities from its pool, so this narrows every
    /// view but that one.
    pub entity_scope: EntityScope,
    /// Inclusive, compared against the measure's event time taken as a date.
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub bucket: Bucket,
    pub dimension_filters: Vec<DimensionFilter>,
    pub view: ViewKind,
    /// Row ceiling for the statement, bound rather than written into the SQL.
    pub row_limit: u64,
}

/// What the group-cap pre-pass asks for: a metric read's scope, the dimensions
/// to rank groups of, and how many of them to keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRankingQuery {
    pub tenant_id: String,
    pub entity_scope: EntityScope,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub dimension_filters: Vec<DimensionFilter>,
    pub dimensions: Vec<String>,
    pub count: u64,
}
