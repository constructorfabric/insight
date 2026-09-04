//! A dataset for the engine's own tests, validated the way a shipped one is.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::domain::field_catalog::loader;

use super::contract::dto::QueryRequest;
use super::datasets::declaration::{Dataset, DatasetDocument};
use super::datasets::validate;

const SNAPSHOT: &str = r#"[
  {
    "database": "insight",
    "relation": "git_commits",
    "engine": "MergeTree",
    "columns": [
      {"name": "tenant_id", "type": "Nullable(String)"},
      {"name": "source_id", "type": "Nullable(String)"},
      {"name": "commit_hash", "type": "String"},
      {"name": "author_email", "type": "String"},
      {"name": "message", "type": "String"},
      {"name": "authored_at", "type": "DateTime"},
      {"name": "authored_date", "type": "Date"},
      {"name": "branch_scope", "type": "String"},
      {"name": "branch_scope_label", "type": "String"},
      {"name": "repository", "type": "String"},
      {"name": "repository_label", "type": "String"},
      {"name": "source", "type": "String"},
      {"name": "source_label", "type": "String"},
      {"name": "lines_added", "type": "Nullable(Int64)"},
      {"name": "lines_removed", "type": "Nullable(Int64)"}
    ]
  }
]"#;

const DECLARATION: &str = "
key: git_commits
database: insight
relation: git_commits
read_discipline: plain
tenant_field: tenant_id
time_fields:
  - field: authored_at
    default: true
  - field: authored_date
dimensions:
  - field: author_email
  - field: branch_scope
    label_field: branch_scope_label
  - field: repository
    label_field: repository_label
  - field: source
    label_field: source_label
  - field: source_id
    absent_value: __unknown__
measurables:
  - field: lines_added
  - field: lines_removed
row_identity:
  - tenant_id
  - source
  - commit_hash
";

pub fn commits() -> Dataset {
    let catalog = loader::load(SNAPSHOT).expect("the fixture snapshot parses");
    let document: DatasetDocument =
        serde_yaml::from_str(DECLARATION).expect("the fixture declaration parses");
    validate::validate(document, &catalog).expect("the fixture declaration is admissible")
}

pub fn query(json: &str) -> QueryRequest {
    serde_json::from_str(json).expect("the fixture query is in the contract")
}
