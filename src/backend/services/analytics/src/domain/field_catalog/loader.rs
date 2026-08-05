//! Build the field catalog by joining the authored roles (`roles.yaml`) with
//! the ClickHouse type snapshot (`types.snapshot.json`). The join is where the
//! two halves are reconciled: a role that names a column the warehouse does not
//! have, or a column whose type the layer cannot model, is a hard error — so the
//! embedded catalog is the validation universe, guaranteed internally consistent
//! at build time (the offline tests below), and kept honest against the live
//! warehouse by the drift test (`live_tests`).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use thiserror::Error;

use crate::domain::field_catalog::model::{
    Dataset, Field, FieldCatalog, FieldRole, FieldType, ReadDiscipline,
};

const ROLES_YAML: &str = include_str!("roles.yaml");
const TYPES_SNAPSHOT_JSON: &str = include_str!("types.snapshot.json");

static CATALOG: OnceLock<FieldCatalog> = OnceLock::new();

/// The embedded, validated field catalog. Panics on a malformed or inconsistent
/// embedded artifact — a build defect the offline tests already catch, never a
/// runtime condition.
pub fn field_catalog() -> &'static FieldCatalog {
    CATALOG.get_or_init(|| {
        #[expect(
            clippy::expect_used,
            reason = "roles.yaml + types.snapshot.json are embedded at compile time and pinned by the offline consistency tests; a failure here is a build defect, not a runtime condition"
        )]
        build(ROLES_YAML, TYPES_SNAPSHOT_JSON).expect("embedded field catalog must be consistent")
    })
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("invalid roles.yaml: {0}")]
    Roles(String),
    #[error("invalid types.snapshot.json: {0}")]
    Snapshot(String),
    #[error("dataset {dataset}: relation {relation} is absent from the type snapshot")]
    UnknownRelation { dataset: String, relation: String },
    #[error("dataset {dataset}: field {field} is not a column of {relation}")]
    UnknownColumn {
        dataset: String,
        relation: String,
        field: String,
    },
    #[error(
        "dataset {dataset}: field {field} has ClickHouse type {ch_type}, which the semantic layer does not model"
    )]
    UnmodeledType {
        dataset: String,
        field: String,
        ch_type: String,
    },
    #[error("dataset {dataset}: expected exactly one {role} field, found {count}")]
    RoleCardinality {
        dataset: String,
        role: &'static str,
        count: usize,
    },
    #[error("dataset {dataset}: no event_time field")]
    NoEventTime { dataset: String },
    #[error("duplicate dataset key {0}")]
    DuplicateDataset(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RolesDoc {
    datasets: Vec<DatasetRoles>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatasetRoles {
    key: String,
    database: String,
    table: String,
    read_discipline: ReadDiscipline,
    fields: Vec<FieldRoleEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldRoleEntry {
    name: String,
    role: FieldRole,
}

/// Relation (`database.table`) -> column -> ClickHouse type.
type Snapshot = BTreeMap<String, BTreeMap<String, String>>;

fn build(roles_yaml: &str, snapshot_json: &str) -> Result<FieldCatalog, CatalogError> {
    let roles: RolesDoc =
        serde_yaml::from_str(roles_yaml).map_err(|e| CatalogError::Roles(e.to_string()))?;
    let snapshot: Snapshot =
        serde_json::from_str(snapshot_json).map_err(|e| CatalogError::Snapshot(e.to_string()))?;

    let mut datasets = Vec::with_capacity(roles.datasets.len());
    let mut seen = std::collections::BTreeSet::new();

    for dataset in roles.datasets {
        if !seen.insert(dataset.key.clone()) {
            return Err(CatalogError::DuplicateDataset(dataset.key));
        }
        datasets.push(build_dataset(&dataset, &snapshot)?);
    }

    Ok(FieldCatalog { datasets })
}

fn build_dataset(roles: &DatasetRoles, snapshot: &Snapshot) -> Result<Dataset, CatalogError> {
    let relation = format!("{}.{}", roles.database, roles.table);
    let columns = snapshot
        .get(&relation)
        .ok_or_else(|| CatalogError::UnknownRelation {
            dataset: roles.key.clone(),
            relation: relation.clone(),
        })?;

    let mut fields = Vec::with_capacity(roles.fields.len());
    for entry in &roles.fields {
        let ch_type = columns
            .get(&entry.name)
            .ok_or_else(|| CatalogError::UnknownColumn {
                dataset: roles.key.clone(),
                relation: relation.clone(),
                field: entry.name.clone(),
            })?;
        let (ty, nullable) =
            FieldType::normalize(ch_type).ok_or_else(|| CatalogError::UnmodeledType {
                dataset: roles.key.clone(),
                field: entry.name.clone(),
                ch_type: ch_type.clone(),
            })?;
        fields.push(Field {
            name: entry.name.clone(),
            role: entry.role,
            ty,
            nullable,
        });
    }

    let dataset = Dataset {
        key: roles.key.clone(),
        database: roles.database.clone(),
        table: roles.table.clone(),
        read_discipline: roles.read_discipline,
        fields,
    };

    assert_role_invariants(&dataset)?;
    Ok(dataset)
}

/// A dataset must expose exactly one tenant field, exactly one entity field, and
/// at least one event-time field — the minimum for the compiler to scope,
/// attribute, and bucket every measure over it.
fn assert_role_invariants(dataset: &Dataset) -> Result<(), CatalogError> {
    let count = |role| dataset.fields_with_role(role).count();

    for (role, name) in [(FieldRole::Tenant, "tenant"), (FieldRole::Entity, "entity")] {
        let n = count(role);
        if n != 1 {
            return Err(CatalogError::RoleCardinality {
                dataset: dataset.key.clone(),
                role: name,
                count: n,
            });
        }
    }

    if count(FieldRole::EventTime) == 0 {
        return Err(CatalogError::NoEventTime {
            dataset: dataset.key.clone(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dataset<'a>(catalog: &'a FieldCatalog, key: &str) -> &'a Dataset {
        catalog
            .datasets
            .iter()
            .find(|d| d.key == key)
            .unwrap_or_else(|| panic!("dataset {key} missing"))
    }

    fn field<'a>(dataset: &'a Dataset, name: &str) -> &'a Field {
        dataset
            .fields
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("field {name} missing"))
    }

    #[test]
    fn embedded_catalog_builds_and_is_nonempty() {
        let catalog = field_catalog();
        assert!(!catalog.datasets.is_empty(), "catalog is empty");
        dataset(catalog, "git_commits");
        dataset(catalog, "git_pull_requests");
    }

    #[test]
    fn every_authored_field_resolved_to_a_type() {
        // build() only returns Ok if every field joined to a modeled type; this
        // spells out the resulting shape for one dataset.
        let catalog = field_catalog();
        let commits = dataset(catalog, "git_commits");
        assert_eq!(commits.read_discipline, ReadDiscipline::Final);

        let author = field(commits, "author_email");
        assert_eq!(author.role, FieldRole::Entity);
        assert_eq!(author.ty, FieldType::String);

        let date = field(commits, "date");
        assert_eq!(date.role, FieldRole::EventTime);
        assert_eq!(date.ty, FieldType::DateTime);
        assert!(date.nullable, "date is Nullable(DateTime) in the snapshot");

        let lines = field(commits, "lines_added");
        assert_eq!(lines.role, FieldRole::Value);
        assert_eq!(lines.ty, FieldType::Int);
    }

    #[test]
    fn every_dataset_has_one_tenant_one_entity_and_a_time() {
        for dataset in &field_catalog().datasets {
            assert_eq!(dataset.fields_with_role(FieldRole::Tenant).count(), 1);
            assert_eq!(dataset.fields_with_role(FieldRole::Entity).count(), 1);
            assert!(dataset.fields_with_role(FieldRole::EventTime).count() >= 1);
        }
    }

    #[test]
    fn type_normalization_peels_wrappers() {
        let cases = [
            ("String", Some((FieldType::String, false))),
            ("Nullable(String)", Some((FieldType::String, true))),
            ("Int64", Some((FieldType::Int, false))),
            ("Nullable(Int64)", Some((FieldType::Int, true))),
            ("UInt8", Some((FieldType::UInt, false))),
            ("DateTime", Some((FieldType::DateTime, false))),
            ("DateTime64(3)", Some((FieldType::DateTime, false))),
            ("Nullable(DateTime)", Some((FieldType::DateTime, true))),
            ("LowCardinality(String)", Some((FieldType::String, false))),
            (
                "LowCardinality(Nullable(String))",
                Some((FieldType::String, true)),
            ),
            ("Float64", Some((FieldType::Float, false))),
            ("Array(String)", None),
            ("Map(String, String)", None),
        ];
        for (input, expected) in cases {
            assert_eq!(FieldType::normalize(input), expected, "normalize({input})");
        }
    }

    const MIN_SNAPSHOT: &str =
        r#"{"silver.t":{"tenant_id":"Nullable(String)","e":"String","ts":"DateTime"}}"#;

    fn roles(fields: &str) -> String {
        format!(
            "datasets:\n  - key: d\n    database: silver\n    table: t\n    \
             read_discipline: final\n    fields:\n{fields}"
        )
    }

    #[test]
    fn rejects_field_absent_from_snapshot() {
        let doc = roles(
            "      - {{ name: tenant_id, role: tenant }}\n      - {{ name: e, role: entity }}\n      \
             - {{ name: ts, role: event_time }}\n      - {{ name: ghost, role: value }}\n",
        )
        .replace("{{", "{")
        .replace("}}", "}");
        let Err(err) = build(&doc, MIN_SNAPSHOT) else {
            panic!("ghost column must be rejected");
        };
        assert!(matches!(err, CatalogError::UnknownColumn { field, .. } if field == "ghost"));
    }

    #[test]
    fn rejects_unmodeled_type() {
        let snapshot = r#"{"silver.t":{"tenant_id":"Nullable(String)","e":"String","ts":"DateTime","weird":"Array(UInt8)"}}"#;
        let doc = roles(
            "      - { name: tenant_id, role: tenant }\n      - { name: e, role: entity }\n      \
             - { name: ts, role: event_time }\n      - { name: weird, role: value }\n",
        );
        let Err(err) = build(&doc, snapshot) else {
            panic!("Array type must be rejected");
        };
        assert!(matches!(err, CatalogError::UnmodeledType { field, .. } if field == "weird"));
    }

    #[test]
    fn rejects_dataset_without_entity() {
        let doc = roles(
            "      - { name: tenant_id, role: tenant }\n      - { name: ts, role: event_time }\n",
        );
        let Err(err) = build(&doc, MIN_SNAPSHOT) else {
            panic!("missing entity must be rejected");
        };
        assert!(matches!(
            err,
            CatalogError::RoleCardinality {
                role: "entity",
                count: 0,
                ..
            }
        ));
    }

    #[test]
    fn rejects_relation_absent_from_snapshot() {
        let doc = roles(
            "      - { name: tenant_id, role: tenant }\n      - { name: e, role: entity }\n      \
             - { name: ts, role: event_time }\n",
        )
        .replace("table: t", "table: missing");
        let Err(err) = build(&doc, MIN_SNAPSHOT) else {
            panic!("missing relation must be rejected");
        };
        assert!(matches!(err, CatalogError::UnknownRelation { .. }));
    }
}
