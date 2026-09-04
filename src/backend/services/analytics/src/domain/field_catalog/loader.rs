//! Parses the schema snapshot into the catalog.

use serde::Deserialize;

use super::model::{CatalogColumn, CatalogRelation, FieldCatalog, FieldType, ReadDiscipline};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("schema snapshot does not parse: {0}")]
    Snapshot(String),
    #[error("schema snapshot carries `{database}.{relation}` twice")]
    DuplicateRelation { database: String, relation: String },
}

#[derive(Debug, Deserialize)]
struct SnapshotRelation {
    database: String,
    relation: String,
    engine: String,
    sorting_key: String,
    columns: Vec<SnapshotColumn>,
}

#[derive(Debug, Deserialize)]
struct SnapshotColumn {
    name: String,
    #[serde(rename = "type")]
    column_type: String,
}

pub fn load(snapshot_json: &str) -> Result<FieldCatalog, CatalogError> {
    let snapshot: Vec<SnapshotRelation> = serde_json::from_str(snapshot_json)
        .map_err(|error| CatalogError::Snapshot(error.to_string()))?;

    let mut relations: Vec<CatalogRelation> = Vec::with_capacity(snapshot.len());
    for raw in snapshot {
        if relations
            .iter()
            .any(|seen| seen.database == raw.database && seen.relation == raw.relation)
        {
            return Err(CatalogError::DuplicateRelation {
                database: raw.database,
                relation: raw.relation,
            });
        }

        relations.push(CatalogRelation {
            read_discipline: ReadDiscipline::for_engine(&raw.engine),
            sorting_key: parse_sorting_key(&raw.sorting_key),
            columns: raw
                .columns
                .into_iter()
                .map(|column| CatalogColumn {
                    name: column.name,
                    field_type: FieldType::parse(&column.column_type),
                })
                .collect(),
            database: raw.database,
            relation: raw.relation,
        });
    }

    Ok(FieldCatalog { relations })
}

fn parse_sorting_key(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::domain::field_catalog::model::TypeClass;

    const SNAPSHOT: &str = r#"[
      {
        "database": "insight",
        "relation": "git_commits",
        "engine": "MergeTree",
        "sorting_key": "tenant_id, author_email, authored_at",
        "columns": [
          {"name": "tenant_id", "type": "Nullable(String)"},
          {"name": "authored_at", "type": "DateTime"},
          {"name": "lines_added", "type": "Nullable(Int64)"},
          {"name": "payload", "type": "Map(String, String)"}
        ]
      }
    ]"#;

    #[test]
    fn a_snapshot_relation_becomes_typed_columns_with_a_read_discipline() {
        let catalog = load(SNAPSHOT).expect("the snapshot parses");
        let relation = catalog
            .relation("insight", "git_commits")
            .expect("the relation is catalogued");

        assert_eq!(relation.read_discipline, ReadDiscipline::Plain);
        assert_eq!(
            relation.sorting_key,
            ["tenant_id", "author_email", "authored_at"]
        );

        let tenant = relation.column("tenant_id").expect("column is present");
        assert_eq!(tenant.field_type.class, TypeClass::Text);
        assert!(tenant.field_type.nullable);

        let composite = relation.column("payload").expect("column is present");
        assert_eq!(composite.field_type.class, TypeClass::Composite);
    }

    #[test]
    fn a_relation_the_snapshot_lacks_is_absent_rather_than_invented() {
        let catalog = load(SNAPSHOT).expect("the snapshot parses");
        assert!(catalog.relation("insight", "git_tags").is_none());
        assert!(catalog.relation("silver", "git_commits").is_none());
    }

    #[test]
    fn a_snapshot_repeating_a_relation_is_rejected() {
        let doubled = format!("[{0},{0}]", &SNAPSHOT[1..SNAPSHOT.len() - 1]);
        assert_eq!(
            load(&doubled),
            Err(CatalogError::DuplicateRelation {
                database: "insight".to_owned(),
                relation: "git_commits".to_owned(),
            })
        );
    }

    #[test]
    fn a_snapshot_that_is_not_the_expected_shape_is_rejected() {
        assert!(matches!(
            load("{\"relations\": []}"),
            Err(CatalogError::Snapshot(_))
        ));
    }
}
