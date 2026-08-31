//! Joins the two halves of the catalog: the generated schema snapshot and the
//! authored roles. The join is the validation — a role naming a field the
//! warehouse lacks, or a type that cannot hold it, fails here, not at query time.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;

use super::model::{
    CatalogDataset, CatalogField, DisplayRole, EntityType, FieldCatalog, FieldRole, FieldType,
    ReadDiscipline,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("schema snapshot does not parse: {0}")]
    Snapshot(String),
    #[error("authored roles do not parse: {0}")]
    Roles(String),
    #[error("dataset `{0}` is declared twice")]
    DuplicateDataset(String),
    #[error("dataset `{dataset}` names relation `{database}.{relation}`, absent from the snapshot")]
    RelationNotFound {
        dataset: String,
        database: String,
        relation: String,
    },
    #[error(
        "dataset `{dataset}` gives field `{field}` a role, but the relation has no such column"
    )]
    FieldNotFound { dataset: String, field: String },
    #[error("dataset `{dataset}` field `{field}` is {field_type}, which cannot be a {role}")]
    RoleNotAdmitted {
        dataset: String,
        field: String,
        field_type: String,
        role: String,
    },
    #[error("dataset `{dataset}` identifies a row by `{field}`, which is not a column")]
    RowIdentityFieldNotFound { dataset: String, field: String },
    #[error("dataset `{dataset}` labels `{field}` with `{label_field}`, which is not a column")]
    LabelFieldNotFound {
        dataset: String,
        field: String,
        label_field: String,
    },
    #[error("dataset `{dataset}` declares no {0} field", role)]
    MissingRequiredRole { dataset: String, role: String },
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RolesFile {
    datasets: Vec<DatasetRoles>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatasetRoles {
    key: String,
    database: String,
    relation: String,
    /// What one row is about. Omitted means a person, the grain every dataset
    /// had before a tenant-grain one existed.
    #[serde(default)]
    entity_type: EntityType,
    #[serde(default)]
    row_identity: Vec<String>,
    #[serde(default)]
    fields: BTreeMap<String, FieldDeclaration>,
}

/// Either a bare role (`author_email: entity`) or a block with display roles.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FieldDeclaration {
    Role(FieldRole),
    Detailed(DetailedField),
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct DetailedField {
    #[serde(default)]
    role: Option<FieldRole>,
    #[serde(default)]
    display: Vec<DisplayRole>,
    #[serde(default)]
    label_field: Option<String>,
}

/// Every dataset must bind these, or the compiler cannot scope its reads.
const REQUIRED_ROLES: &[FieldRole] = &[FieldRole::Tenant, FieldRole::EventTime];

pub fn load(snapshot_json: &str, roles_yaml: &str) -> Result<FieldCatalog, CatalogError> {
    let snapshot: Vec<SnapshotRelation> =
        serde_json::from_str(snapshot_json).map_err(|e| CatalogError::Snapshot(e.to_string()))?;
    let roles: RolesFile =
        serde_yaml::from_str(roles_yaml).map_err(|e| CatalogError::Roles(e.to_string()))?;

    let mut seen = HashSet::new();
    let mut datasets = Vec::with_capacity(roles.datasets.len());
    for declared in roles.datasets {
        if !seen.insert(declared.key.clone()) {
            return Err(CatalogError::DuplicateDataset(declared.key));
        }
        datasets.push(join(&snapshot, declared)?);
    }
    Ok(FieldCatalog { datasets })
}

fn join(
    snapshot: &[SnapshotRelation],
    declared: DatasetRoles,
) -> Result<CatalogDataset, CatalogError> {
    let found = snapshot
        .iter()
        .find(|r| r.database == declared.database && r.relation == declared.relation)
        .ok_or_else(|| CatalogError::RelationNotFound {
            dataset: declared.key.clone(),
            database: declared.database.clone(),
            relation: declared.relation.clone(),
        })?;

    let column_names: HashSet<&str> = found.columns.iter().map(|c| c.name.as_str()).collect();
    for field in &declared.row_identity {
        if !column_names.contains(field.as_str()) {
            return Err(CatalogError::RowIdentityFieldNotFound {
                dataset: declared.key.clone(),
                field: field.clone(),
            });
        }
    }
    for (field, declaration) in &declared.fields {
        if !column_names.contains(field.as_str()) {
            return Err(CatalogError::FieldNotFound {
                dataset: declared.key.clone(),
                field: field.clone(),
            });
        }
        if let FieldDeclaration::Detailed(DetailedField {
            label_field: Some(label),
            ..
        }) = declaration
            && !column_names.contains(label.as_str())
        {
            return Err(CatalogError::LabelFieldNotFound {
                dataset: declared.key.clone(),
                field: field.clone(),
                label_field: label.clone(),
            });
        }
    }

    let mut fields = Vec::with_capacity(found.columns.len());
    for column in &found.columns {
        let field_type = FieldType::parse(&column.column_type);
        let (role, display, label_field) = match declared.fields.get(&column.name) {
            None => (None, Vec::new(), None),
            Some(FieldDeclaration::Role(role)) => (Some(*role), Vec::new(), None),
            Some(FieldDeclaration::Detailed(detailed)) => (
                detailed.role,
                detailed.display.clone(),
                detailed.label_field.clone(),
            ),
        };
        if let Some(role) = role
            && !field_type.admits(role)
        {
            return Err(CatalogError::RoleNotAdmitted {
                dataset: declared.key.clone(),
                field: column.name.clone(),
                field_type: column.column_type.clone(),
                role: format!("{role:?}").to_lowercase(),
            });
        }
        fields.push(CatalogField {
            name: column.name.clone(),
            field_type,
            role,
            display,
            label_field,
        });
    }

    for required in REQUIRED_ROLES {
        if !fields.iter().any(|f| f.role == Some(*required)) {
            return Err(CatalogError::MissingRequiredRole {
                dataset: declared.key.clone(),
                role: format!("{required:?}").to_lowercase(),
            });
        }
    }

    Ok(CatalogDataset {
        key: declared.key,
        database: declared.database,
        relation: declared.relation,
        entity_type: declared.entity_type,
        read_discipline: ReadDiscipline::for_engine(&found.engine),
        sorting_key: parse_sorting_key(&found.sorting_key),
        row_identity: declared.row_identity,
        fields,
    })
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
        "database": "silver",
        "relation": "class_git_commits",
        "engine": "ReplacingMergeTree",
        "sorting_key": "unique_key",
        "columns": [
          {"name": "tenant_id", "type": "Nullable(String)"},
          {"name": "author_email", "type": "String"},
          {"name": "date", "type": "Nullable(DateTime)"},
          {"name": "lines_added", "type": "Nullable(Int64)"},
          {"name": "branch", "type": "String"},
          {"name": "branch_label", "type": "String"},
          {"name": "message", "type": "String"},
          {"name": "payload", "type": "Map(String, String)"}
        ]
      }
    ]"#;

    fn roles(fields: &str) -> String {
        format!(
            "datasets:\n  - key: git_commits\n    database: silver\n    relation: class_git_commits\n    fields:\n{fields}"
        )
    }

    fn roles_identified_by(identity: &str) -> String {
        format!(
            "datasets:\n  - key: git_commits\n    database: silver\n    relation: class_git_commits\n    row_identity: [{identity}]\n    fields:\n{MINIMUM}"
        )
    }

    const MINIMUM: &str = "      tenant_id: tenant\n      date: event_time\n";

    #[test]
    fn joins_types_with_roles() {
        let catalog = load(
            SNAPSHOT,
            &roles(&format!(
                "{MINIMUM}      author_email: entity\n      lines_added: measurable\n      message:\n        display: [title]\n"
            )),
        )
        .expect("catalog loads");

        let dataset = catalog.dataset("git_commits").expect("dataset present");
        assert_eq!(dataset.read_discipline, ReadDiscipline::Collapsing);
        assert_eq!(dataset.sorting_key, ["unique_key"]);
        assert_eq!(dataset.fields.len(), 8);

        let entity = dataset.field("author_email").unwrap();
        assert_eq!(entity.role, Some(FieldRole::Entity));
        assert_eq!(entity.field_type.class, TypeClass::Text);

        let measurable = dataset.field("lines_added").unwrap();
        assert_eq!(measurable.role, Some(FieldRole::Measurable));
        assert!(measurable.field_type.nullable);

        let title = dataset.field("message").unwrap();
        assert_eq!(title.role, None);
        assert_eq!(title.display, [DisplayRole::Title]);

        let unroled = dataset.field("payload").unwrap();
        assert_eq!(unroled.role, None);
        assert!(unroled.display.is_empty());
    }

    #[test]
    fn label_binding_resolves_against_the_relation() {
        let catalog = load(
            SNAPSHOT,
            &roles(&format!(
                "{MINIMUM}      branch:\n        role: dimension\n        label_field: branch_label\n"
            )),
        )
        .expect("catalog loads");
        let branch = catalog
            .dataset("git_commits")
            .unwrap()
            .field("branch")
            .unwrap();
        assert_eq!(branch.role, Some(FieldRole::Dimension));
        assert_eq!(branch.label_field.as_deref(), Some("branch_label"));
    }

    #[test]
    fn a_declared_row_identity_resolves_against_the_relation() {
        let catalog =
            load(SNAPSHOT, &roles_identified_by("branch, message")).expect("catalog loads");

        assert_eq!(
            catalog.dataset("git_commits").unwrap().row_identity,
            ["branch", "message"]
        );
    }

    #[test]
    fn a_row_identity_column_the_warehouse_lacks_is_rejected() {
        let error =
            load(SNAPSHOT, &roles_identified_by("commit_sha")).expect_err("field is absent");

        assert_eq!(
            error,
            CatalogError::RowIdentityFieldNotFound {
                dataset: "git_commits".to_owned(),
                field: "commit_sha".to_owned(),
            }
        );
    }

    #[test]
    fn a_dataset_may_declare_no_row_identity() {
        let catalog = load(SNAPSHOT, &roles(MINIMUM)).expect("catalog loads");

        assert!(
            catalog
                .dataset("git_commits")
                .unwrap()
                .row_identity
                .is_empty()
        );
    }

    #[test]
    fn a_role_on_a_column_the_warehouse_lacks_is_rejected() {
        let error = load(
            SNAPSHOT,
            &roles(&format!("{MINIMUM}      commit_sha: dimension\n")),
        )
        .expect_err("field is absent");
        assert_eq!(
            error,
            CatalogError::FieldNotFound {
                dataset: "git_commits".to_owned(),
                field: "commit_sha".to_owned(),
            }
        );
    }

    #[test]
    fn a_role_the_type_cannot_hold_is_rejected() {
        let error = load(
            SNAPSHOT,
            &roles(&format!("{MINIMUM}      message: measurable\n")),
        )
        .expect_err("text is not measurable");
        assert!(matches!(error, CatalogError::RoleNotAdmitted { .. }));

        let error = load(
            SNAPSHOT,
            &roles(&format!("{MINIMUM}      payload: dimension\n")),
        )
        .expect_err("a map is not groupable");
        assert!(matches!(error, CatalogError::RoleNotAdmitted { .. }));
    }

    #[test]
    fn a_label_field_the_warehouse_lacks_is_rejected() {
        let error = load(
            SNAPSHOT,
            &roles(&format!(
                "{MINIMUM}      branch:\n        role: dimension\n        label_field: branch_title\n"
            )),
        )
        .expect_err("label column is absent");
        assert!(matches!(error, CatalogError::LabelFieldNotFound { .. }));
    }

    #[test]
    fn a_dataset_without_tenant_or_event_time_is_rejected() {
        let error = load(SNAPSHOT, &roles("      author_email: entity\n"))
            .expect_err("required roles missing");
        assert_eq!(
            error,
            CatalogError::MissingRequiredRole {
                dataset: "git_commits".to_owned(),
                role: "tenant".to_owned(),
            }
        );
    }

    #[test]
    fn an_unknown_relation_is_rejected() {
        let yaml = "datasets:\n  - key: git_tags\n    database: silver\n    relation: class_git_tags\n    fields: {}\n";
        let error = load(SNAPSHOT, yaml).expect_err("relation is absent");
        assert!(matches!(error, CatalogError::RelationNotFound { .. }));
    }

    #[test]
    fn a_duplicate_dataset_key_is_rejected() {
        let one = "  - key: git_commits\n    database: silver\n    relation: class_git_commits\n    fields:\n      tenant_id: tenant\n      date: event_time\n";
        let yaml = format!("datasets:\n{one}{one}");
        let error = load(SNAPSHOT, &yaml).expect_err("key is repeated");
        assert_eq!(
            error,
            CatalogError::DuplicateDataset("git_commits".to_owned())
        );
    }

    #[test]
    fn unknown_keys_in_a_declaration_are_rejected() {
        let yaml = &roles(&format!(
            "{MINIMUM}      branch:\n        role: dimension\n        labelfield: branch_label\n"
        ));
        assert!(matches!(load(SNAPSHOT, yaml), Err(CatalogError::Roles(_))));
    }
}
