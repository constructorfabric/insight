//! The catalogued datasets the compiler tests read: one collapsing relation
//! carrying every field role a measure binds, and one direct relation. Both
//! load through the real snapshot and role loader, so a fixture cannot admit
//! what the catalog would not.

#![allow(clippy::expect_used)]

use crate::domain::field_catalog::loader;
use crate::domain::field_catalog::model::{CatalogDataset, FieldCatalog};

const SNAPSHOT: &str = r#"[
  {
    "database": "silver",
    "relation": "class_git_pull_requests",
    "engine": "ReplacingMergeTree",
    "sorting_key": "unique_key",
    "columns": [
      {"name": "tenant_id", "type": "Nullable(String)"},
      {"name": "author_email", "type": "String"},
      {"name": "closed_on", "type": "Nullable(DateTime)"},
      {"name": "state", "type": "String"},
      {"name": "repo_slug", "type": "String"},
      {"name": "data_source", "type": "String"},
      {"name": "data_source_label", "type": "String"},
      {"name": "is_draft", "type": "Bool"},
      {"name": "lines_added", "type": "Nullable(Int64)"},
      {"name": "pull_request_id", "type": "String"}
    ]
  },
  {
    "database": "silver",
    "relation": "class_git_commits",
    "engine": "MergeTree",
    "sorting_key": "unique_key",
    "columns": [
      {"name": "tenant_id", "type": "Nullable(String)"},
      {"name": "author_email", "type": "String"},
      {"name": "committed_on", "type": "Nullable(DateTime)"},
      {"name": "repo_slug", "type": "String"}
    ]
  }
]"#;

const ROLES: &str = "
datasets:
  - key: git_pull_requests
    database: silver
    relation: class_git_pull_requests
    fields:
      tenant_id: tenant
      author_email: entity
      closed_on: event_time
      state: dimension
      repo_slug: dimension
      data_source: dimension
      is_draft: dimension
      lines_added: measurable
      pull_request_id: dimension
  - key: git_commits
    database: silver
    relation: class_git_commits
    fields:
      tenant_id: tenant
      author_email: entity
      committed_on: event_time
      repo_slug: dimension
";

pub fn catalog() -> FieldCatalog {
    loader::load(SNAPSHOT, ROLES).expect("catalog loads")
}

pub fn dataset(key: &str) -> CatalogDataset {
    catalog()
        .dataset(key)
        .expect("dataset is catalogued")
        .clone()
}
