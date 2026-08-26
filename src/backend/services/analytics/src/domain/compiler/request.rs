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
