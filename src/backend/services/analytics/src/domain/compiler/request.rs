//! What a caller asks a measure for: the tenancy and entity scope to read
//! under, the inclusive date window, the time grain, and the dimension
//! narrowing and split. Everything here is already resolved — the compiler
//! translates, it does not look anything up.

use std::num::NonZeroU32;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::domain::field_catalog::model::EntityType;

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
    /// Named people with the identities the caller resolved. The read joins
    /// that pair relation, so its rows are keyed by the person.
    People(Vec<ResolvedPerson>),
}

impl EntityScope {
    /// Whether any row can fall inside this scope at all. INVARIANT: a scope
    /// naming entities but resolving none of them admits nothing — treating it
    /// as "no narrowing" would answer about the whole tenant instead.
    #[must_use]
    pub fn reaches_any_row(&self) -> bool {
        match self {
            Self::Tenant => true,
            Self::Identities(values) => !values.is_empty(),
            Self::People(members) => any_identity_resolved(members),
        }
    }
}

/// Whether any of these people resolved to an identity a row can carry.
///
/// INVARIANT: a pool none of them resolved for admits nothing. Every read that
/// joins one narrows by that join alone, so an empty pool must be decided
/// before the read rather than read as "everybody".
#[must_use]
pub fn any_identity_resolved(members: &[ResolvedPerson]) -> bool {
    members.iter().any(|member| !member.identities.is_empty())
}

/// A person and the source identities the caller resolved for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPerson {
    pub person_ref: String,
    pub identities: Vec<String>,
}

/// Narrows a read to named values of one of the measure's declared dimensions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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

/// The row shape a metric read produces, with the inputs that shape needs. A
/// view the window and scope fully determine carries nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewKind {
    /// One value per entity over the whole window; the bucket is not read.
    SubjectTotal,
    /// One value per entity per bucket, plus the window total per entity.
    SubjectSeries(SubjectSeriesView),
    /// One value per entity per combination of the named dimensions.
    SubjectSplit(SubjectSplitView),
    /// One value per combination of the named dimensions, folded over every
    /// entity the scope admits.
    CombinedSplit(CombinedSplitView),
    /// Per-entity bin counts over the measure's per-row values.
    Bins(BinsView),
    /// Per-entity quantiles of the measure's per-row values.
    Quantiles(QuantilesView),
    /// One row per target: the target's own value beside its peers' spread.
    Comparison(ComparisonView),
}

impl ViewKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SubjectTotal => "subject_total",
            Self::SubjectSeries(_) => "subject_series",
            Self::SubjectSplit(_) => "subject_split",
            Self::CombinedSplit(_) => "combined_split",
            Self::Bins(_) => "bins",
            Self::Quantiles(_) => "quantiles",
            Self::Comparison(_) => "comparison",
        }
    }
}

/// How many bins each entity's own range is cut into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinsView {
    pub bins: NonZeroU32,
}

/// The quantiles a read reports, in the order the caller asked for them.
#[derive(Debug, Clone, PartialEq)]
pub struct QuantilesView {
    /// Each in `(0, 1)`.
    pub quantiles: Vec<f64>,
}

/// Dimension keys of the metric's grain measure, in the order their
/// `dim_{index}_value` / `dim_{index}_label` columns take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectSplitView {
    pub dimensions: Vec<String>,
}

/// Grouped by bucket before anything else, so a split subject series
/// reports a series per dimension combination rather than one series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectSeriesView {
    pub dimensions: Vec<String>,
    /// Absent means every group is reported. A cap needs a dimension to rank
    /// groups of, and folds every other group into one remainder series.
    pub group_limit: Option<GroupLimit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombinedSplitView {
    pub dimensions: Vec<String>,
    /// Absent means every group is reported; a cap folds the rest into one row.
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
/// The caller resolves each pool member's identities and the read joins the two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparisonView {
    pub population: ComparisonPopulation,
    /// Person references the view answers for.
    pub targets: Vec<String>,
    /// Every person the read may evaluate, with their resolved identities.
    /// A target absent from the pool reads no value of its own.
    pub pool: Vec<ResolvedPerson>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComparisonPopulation {
    /// Peers are the members of the target's own declared cohort, read from
    /// the cohort relation.
    DeclaredCohort { cohort_key: String },
    /// Peers are every member of the supplied pool.
    Tenant,
}

/// What a caller asks a metric for. INVARIANT: a metric read is never grouped
/// by a measure dimension the metric does not declare.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricQuery {
    pub tenant_id: String,
    /// A comparison read takes its entities from its pool, so this narrows every
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

/// What a caller asks a metric for row by row. INVARIANT: tenancy, scope,
/// window and dimension narrowing are the aggregate request's.
#[derive(Debug, Clone, PartialEq)]
pub struct DrilldownQuery {
    pub tenant_id: String,
    pub entity_scope: EntityScope,
    /// Inclusive, compared against the measure's event time taken as a date.
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub dimension_filters: Vec<DimensionFilter>,
    /// Dimension keys to project beyond the measure's own. Each one must be a
    /// dimension the measure declares.
    pub display_dimensions: Vec<String>,
    /// Rows per page. The statement reads one more than this, so the caller
    /// can tell whether a further page follows without counting the whole read.
    pub page_size: u64,
    /// The column to order by ahead of the total order. Absent leaves a page
    /// in the only order it is guaranteed to have.
    pub sort: Option<DrilldownSort>,
    pub cursor: Option<DrilldownCursor>,
}

/// Which column a page is ordered by, named as the page reports it rather than
/// as the dataset spells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrilldownSort {
    pub column: String,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Ascending => "ASC",
            Self::Descending => "DESC",
        }
    }

    /// The comparison selecting the rows a page has not reported yet.
    pub(super) fn after(self) -> &'static str {
        match self {
            Self::Ascending => ">",
            Self::Descending => "<",
        }
    }
}

/// Where a page resumes: the ordering values the previous page's last row
/// carried, in the order the read sorts by.
#[derive(Debug, Clone, PartialEq)]
pub struct DrilldownCursor {
    /// What the last row carried in the sorted column, absent when that value
    /// was null and the page resumes inside the tail nulls sort into.
    pub sort_value: Option<SortValue>,
    pub sort_values: Vec<String>,
}

/// A value a page resumes at in its sorted column, in the type that column
/// compares as: ordering `value` as text would rank 100 below 9.
#[derive(Debug, Clone, PartialEq)]
pub enum SortValue {
    Text(String),
    Number(f64),
}

/// What the comparison pre-pass asks for: everyone who shares a declared cohort with
/// one of the targets, so the caller can resolve them into a [`ComparisonView`] pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CohortMembersQuery {
    pub tenant_id: String,
    /// The entity type the metric declares, which cohort membership is under.
    pub entity_type: EntityType,
    pub cohort_key: String,
    /// Person references whose own cohorts the membership is read from.
    pub targets: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_naming_entities_none_of_which_resolved_admits_no_row() {
        let cases = [
            (EntityScope::Tenant, true),
            (EntityScope::Identities(Vec::new()), false),
            (
                EntityScope::Identities(vec!["dev@example.com".to_owned()]),
                true,
            ),
            (EntityScope::People(Vec::new()), false),
            (
                EntityScope::People(vec![ResolvedPerson {
                    person_ref: "person-1".to_owned(),
                    identities: Vec::new(),
                }]),
                false,
            ),
            (
                EntityScope::People(vec![
                    ResolvedPerson {
                        person_ref: "person-1".to_owned(),
                        identities: Vec::new(),
                    },
                    ResolvedPerson {
                        person_ref: "person-2".to_owned(),
                        identities: vec!["dev@example.com".to_owned()],
                    },
                ]),
                true,
            ),
        ];

        for (scope, reaches) in cases {
            let named = format!("{scope:?}");

            assert_eq!(scope.reaches_any_row(), reaches, "should decide: {named}");
        }
    }
}
