//! The wire pieces every question in this family names, and the one every
//! answer carries.
//!
//! INVARIANT: nothing here names a view, a relation or a row shape.

use serde::{Deserialize, Serialize};

/// Whose values a question is about, internally tagged on `type` so a subject
/// kind carries only its own fields.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Subjects {
    Persons { ids: Vec<String> },
    Tenant {},
}

/// Keeps only the rows whose dimension holds one of the named values.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DimensionFilter {
    /// A dimension key the metric's grain measure declares.
    pub dimension: String,
    pub values: Vec<String>,
}

/// What produced this answer.
#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Provenance {
    pub executor: Executor,
    /// The definition version the store holds; absent when it carries no row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_version: Option<i32>,
    pub served_from: ServedFrom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Executor {
    Semantic,
}

/// Where the rows behind this answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServedFrom {
    Computed,
}
