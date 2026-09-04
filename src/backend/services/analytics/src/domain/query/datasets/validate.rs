//! Checks a dataset declaration against the field catalog.
//!
//! INVARIANT: a declaration that passes here can never fail a compile for a
//! missing column.

use std::collections::HashSet;

use crate::domain::field_catalog::model::{
    CatalogColumn, CatalogRelation, FieldCatalog, ReadDiscipline, TypeClass,
};

use super::declaration::{Dataset, DatasetDocument, DatasetKey, Dimension, Measurable, TimeField};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeclarationError {
    #[error("dataset declaration does not parse: {0}")]
    Document(String),
    #[error("`{0}` is not a dataset key: lowercase snake_case starting with [a-z]")]
    BadKey(String),
    #[error("dataset `{0}` is declared twice")]
    DuplicateDataset(String),
    #[error("dataset `{dataset}` names relation `{database}.{relation}`, absent from the catalog")]
    RelationNotFound {
        dataset: String,
        database: String,
        relation: String,
    },
    #[error(
        "dataset `{dataset}` declares a `{declared}` read of `{relation}`, whose engine needs `{required}`"
    )]
    ReadDisciplineMismatch {
        dataset: String,
        relation: String,
        declared: &'static str,
        required: &'static str,
    },
    #[error(
        "dataset `{dataset}` names `{field}` as its {role}, and the relation has no such column"
    )]
    ColumnNotFound {
        dataset: String,
        field: String,
        role: &'static str,
    },
    #[error(
        "dataset `{dataset}` names `{field}` as its {role}, and a {found} column cannot be one"
    )]
    ColumnTypeRejected {
        dataset: String,
        field: String,
        role: &'static str,
        found: &'static str,
    },
    #[error("dataset `{dataset}` declares `{field}` as a {role} twice")]
    DuplicateField {
        dataset: String,
        field: String,
        role: &'static str,
    },
    #[error("dataset `{dataset}` declares {count} default time fields, and a scan buckets by one")]
    DefaultTimeFields { dataset: String, count: usize },
    #[error(
        "dataset `{dataset}` groups nullable dimension `{field}` without an absent_value, so its absent rows have no name"
    )]
    DimensionWithoutAbsentValue { dataset: String, field: String },
    #[error(
        "dataset `{dataset}` gives non-nullable dimension `{field}` an absent_value, which no row can take"
    )]
    DimensionWithUnreachableAbsentValue { dataset: String, field: String },
    #[error("dataset `{dataset}` declares an empty row identity, so no row is one fact")]
    EmptyRowIdentity { dataset: String },
}

pub fn validate(
    document: DatasetDocument,
    catalog: &FieldCatalog,
) -> Result<Dataset, DeclarationError> {
    let dataset = document.key.clone();
    let key = DatasetKey::parse(&document.key)
        .ok_or_else(|| DeclarationError::BadKey(document.key.clone()))?;

    let relation = catalog
        .relation(&document.database, &document.relation)
        .ok_or_else(|| DeclarationError::RelationNotFound {
            dataset: dataset.clone(),
            database: document.database.clone(),
            relation: document.relation.clone(),
        })?;

    check_read_discipline(&dataset, &document, relation)?;

    let tenant = column(&dataset, relation, &document.tenant_field, "tenant field")?;
    admit(
        &dataset,
        tenant,
        "tenant field",
        &[TypeClass::Text, TypeClass::Uuid],
    )?;

    let time_fields = validate_time_fields(&dataset, &document, relation)?;
    let dimensions = validate_dimensions(&dataset, &document, relation)?;
    let measurables = validate_measurables(&dataset, &document, relation)?;

    if document.row_identity.is_empty() {
        return Err(DeclarationError::EmptyRowIdentity { dataset });
    }
    for field in &document.row_identity {
        column(&dataset, relation, field, "row identity")?;
    }

    Ok(Dataset {
        key,
        database: document.database,
        relation: document.relation,
        read_discipline: document.read_discipline,
        tenant_field: document.tenant_field,
        time_fields,
        dimensions,
        measurables,
        row_identity: document.row_identity,
    })
}

fn check_read_discipline(
    dataset: &str,
    document: &DatasetDocument,
    relation: &CatalogRelation,
) -> Result<(), DeclarationError> {
    if document.read_discipline == relation.read_discipline {
        return Ok(());
    }

    Err(DeclarationError::ReadDisciplineMismatch {
        dataset: dataset.to_owned(),
        relation: format!("{}.{}", document.database, document.relation),
        declared: discipline_name(document.read_discipline),
        required: discipline_name(relation.read_discipline),
    })
}

fn validate_time_fields(
    dataset: &str,
    document: &DatasetDocument,
    relation: &CatalogRelation,
) -> Result<Vec<TimeField>, DeclarationError> {
    let mut seen = HashSet::new();
    let mut time_fields = Vec::with_capacity(document.time_fields.len());
    for declared in &document.time_fields {
        if !seen.insert(declared.field.as_str()) {
            return Err(DeclarationError::DuplicateField {
                dataset: dataset.to_owned(),
                field: declared.field.clone(),
                role: "time field",
            });
        }

        let found = column(dataset, relation, &declared.field, "time field")?;
        admit(
            dataset,
            found,
            "time field",
            &[TypeClass::Date, TypeClass::Timestamp],
        )?;

        time_fields.push(TimeField {
            field: declared.field.clone(),
            nullable: found.field_type.nullable,
            default: declared.default,
        });
    }

    let defaults = time_fields.iter().filter(|field| field.default).count();
    if defaults != 1 {
        return Err(DeclarationError::DefaultTimeFields {
            dataset: dataset.to_owned(),
            count: defaults,
        });
    }

    Ok(time_fields)
}

fn validate_dimensions(
    dataset: &str,
    document: &DatasetDocument,
    relation: &CatalogRelation,
) -> Result<Vec<Dimension>, DeclarationError> {
    let mut seen = HashSet::new();
    let mut dimensions = Vec::with_capacity(document.dimensions.len());
    for declared in &document.dimensions {
        if !seen.insert(declared.field.as_str()) {
            return Err(DeclarationError::DuplicateField {
                dataset: dataset.to_owned(),
                field: declared.field.clone(),
                role: "dimension",
            });
        }

        let found = column(dataset, relation, &declared.field, "dimension")?;
        admit(
            dataset,
            found,
            "dimension",
            &[
                TypeClass::Text,
                TypeClass::Uuid,
                TypeClass::Boolean,
                TypeClass::Number,
            ],
        )?;

        match (found.field_type.nullable, &declared.absent_value) {
            (true, None) => {
                return Err(DeclarationError::DimensionWithoutAbsentValue {
                    dataset: dataset.to_owned(),
                    field: declared.field.clone(),
                });
            }
            (false, Some(_)) => {
                return Err(DeclarationError::DimensionWithUnreachableAbsentValue {
                    dataset: dataset.to_owned(),
                    field: declared.field.clone(),
                });
            }
            (true, Some(_)) | (false, None) => {}
        }

        if let Some(label) = &declared.label_field {
            let label_column = column(dataset, relation, label, "dimension label")?;
            admit(
                dataset,
                label_column,
                "dimension label",
                &[TypeClass::Text, TypeClass::Uuid, TypeClass::Number],
            )?;
        }

        dimensions.push(Dimension {
            field: declared.field.clone(),
            label_field: declared.label_field.clone(),
            absent_value: declared.absent_value.clone(),
        });
    }

    Ok(dimensions)
}

fn validate_measurables(
    dataset: &str,
    document: &DatasetDocument,
    relation: &CatalogRelation,
) -> Result<Vec<Measurable>, DeclarationError> {
    let mut seen = HashSet::new();
    let mut measurables = Vec::with_capacity(document.measurables.len());
    for declared in &document.measurables {
        if !seen.insert(declared.field.as_str()) {
            return Err(DeclarationError::DuplicateField {
                dataset: dataset.to_owned(),
                field: declared.field.clone(),
                role: "measurable",
            });
        }

        let found = column(dataset, relation, &declared.field, "measurable")?;
        admit(dataset, found, "measurable", &[TypeClass::Number])?;

        measurables.push(Measurable {
            field: declared.field.clone(),
        });
    }

    Ok(measurables)
}

fn column<'a>(
    dataset: &str,
    relation: &'a CatalogRelation,
    field: &str,
    role: &'static str,
) -> Result<&'a CatalogColumn, DeclarationError> {
    relation
        .column(field)
        .ok_or_else(|| DeclarationError::ColumnNotFound {
            dataset: dataset.to_owned(),
            field: field.to_owned(),
            role,
        })
}

fn admit(
    dataset: &str,
    found: &CatalogColumn,
    role: &'static str,
    admissible: &[TypeClass],
) -> Result<(), DeclarationError> {
    if admissible.contains(&found.field_type.class) {
        return Ok(());
    }

    Err(DeclarationError::ColumnTypeRejected {
        dataset: dataset.to_owned(),
        field: found.name.clone(),
        role,
        found: class_name(found.field_type.class),
    })
}

fn class_name(class: TypeClass) -> &'static str {
    match class {
        TypeClass::Text => "text",
        TypeClass::Number => "numeric",
        TypeClass::Boolean => "boolean",
        TypeClass::Date => "date",
        TypeClass::Timestamp => "timestamp",
        TypeClass::Uuid => "uuid",
        TypeClass::Composite => "composite",
    }
}

fn discipline_name(discipline: ReadDiscipline) -> &'static str {
    match discipline {
        ReadDiscipline::Plain => "plain",
        ReadDiscipline::Final => "final",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::domain::field_catalog::loader;

    const SNAPSHOT: &str = r#"[
      {
        "database": "insight",
        "relation": "git_commits",
        "engine": "MergeTree",
        "sorting_key": "tenant_id",
        "columns": [
          {"name": "tenant_id", "type": "Nullable(String)"},
          {"name": "authored_at", "type": "DateTime"},
          {"name": "commit_hash", "type": "String"},
          {"name": "message", "type": "String"},
          {"name": "repository", "type": "String"},
          {"name": "repository_label", "type": "String"},
          {"name": "source_id", "type": "Nullable(String)"},
          {"name": "lines_added", "type": "Nullable(Int64)"},
          {"name": "payload", "type": "Map(String, String)"}
        ]
      },
      {
        "database": "silver",
        "relation": "class_git_commits",
        "engine": "ReplacingMergeTree",
        "sorting_key": "unique_key",
        "columns": [
          {"name": "insight_tenant_id", "type": "String"},
          {"name": "authored_at", "type": "DateTime"}
        ]
      }
    ]"#;

    const MINIMAL: &str = "
key: git_commits
database: insight
relation: git_commits
read_discipline: plain
tenant_field: tenant_id
time_fields:
  - field: authored_at
    default: true
row_identity: [commit_hash]
";

    fn catalog() -> FieldCatalog {
        loader::load(SNAPSHOT).expect("the fixture snapshot parses")
    }

    fn declare(yaml: &str) -> Result<Dataset, DeclarationError> {
        let document: DatasetDocument =
            serde_yaml::from_str(yaml).expect("the fixture document parses");
        validate(document, &catalog())
    }

    fn with(extra: &str) -> String {
        format!("{MINIMAL}{extra}")
    }

    #[test]
    fn a_declaration_the_catalog_agrees_with_resolves() {
        let dataset = declare(&with(
            "dimensions:
  - field: repository
    label_field: repository_label
  - field: source_id
    absent_value: __unknown__
measurables:
  - field: lines_added
",
        ))
        .expect("the declaration is admissible");

        assert_eq!(dataset.key.as_str(), "git_commits");
        assert_eq!(dataset.read_discipline, ReadDiscipline::Plain);
        assert_eq!(
            dataset
                .default_time_field()
                .map(|field| field.field.as_str()),
            Some("authored_at")
        );
        assert_eq!(
            dataset
                .dimension("source_id")
                .and_then(|d| d.absent_value.as_deref()),
            Some("__unknown__")
        );
        assert_eq!(
            dataset
                .dimension("repository")
                .and_then(|d| d.label_field.as_deref()),
            Some("repository_label")
        );
    }

    #[test]
    fn a_column_the_warehouse_lacks_is_refused() {
        let error = declare(&with("dimensions:\n  - field: branch\n")).expect_err("no such column");
        assert_eq!(
            error,
            DeclarationError::ColumnNotFound {
                dataset: "git_commits".to_owned(),
                field: "branch".to_owned(),
                role: "dimension",
            }
        );
    }

    #[test]
    fn a_column_whose_type_cannot_hold_its_role_is_refused() {
        let cases = [
            ("measurables:\n  - field: message\n", "measurable"),
            ("dimensions:\n  - field: payload\n", "dimension"),
        ];
        for (extra, role) in cases {
            let error = declare(&with(extra)).expect_err("the type cannot hold the role");
            assert!(
                matches!(&error, DeclarationError::ColumnTypeRejected { role: found, .. } if *found == role),
                "{extra}: {error}"
            );
        }
    }

    #[test]
    fn a_nullable_dimension_without_a_name_for_its_absent_rows_is_refused() {
        let error =
            declare(&with("dimensions:\n  - field: source_id\n")).expect_err("no absent_value");
        assert_eq!(
            error,
            DeclarationError::DimensionWithoutAbsentValue {
                dataset: "git_commits".to_owned(),
                field: "source_id".to_owned(),
            }
        );
    }

    #[test]
    fn a_non_nullable_dimension_naming_absent_rows_is_refused() {
        let error = declare(&with(
            "dimensions:\n  - field: repository\n    absent_value: __unknown__\n",
        ))
        .expect_err("no row can be absent");
        assert_eq!(
            error,
            DeclarationError::DimensionWithUnreachableAbsentValue {
                dataset: "git_commits".to_owned(),
                field: "repository".to_owned(),
            }
        );
    }

    #[test]
    fn a_declaration_must_name_exactly_one_default_time_field() {
        let none = declare(
            "
key: git_commits
database: insight
relation: git_commits
read_discipline: plain
tenant_field: tenant_id
time_fields:
  - field: authored_at
row_identity: [commit_hash]
",
        )
        .expect_err("no default");
        assert_eq!(
            none,
            DeclarationError::DefaultTimeFields {
                dataset: "git_commits".to_owned(),
                count: 0,
            }
        );
    }

    #[test]
    fn a_plain_read_of_a_replacing_relation_is_refused() {
        let error = declare(
            "
key: git_commits
database: silver
relation: class_git_commits
read_discipline: plain
tenant_field: insight_tenant_id
time_fields:
  - field: authored_at
    default: true
row_identity: [insight_tenant_id]
",
        )
        .expect_err("a replacing engine must be read collapsed");
        assert!(matches!(
            error,
            DeclarationError::ReadDisciplineMismatch {
                declared: "plain",
                required: "final",
                ..
            }
        ));
    }

    #[test]
    fn a_relation_the_catalog_lacks_is_refused() {
        let error = declare(&MINIMAL.replace("relation: git_commits", "relation: git_tags"))
            .expect_err("no such relation");
        assert!(matches!(error, DeclarationError::RelationNotFound { .. }));
    }

    #[test]
    fn a_repeated_field_in_one_role_is_refused() {
        let error = declare(&with(
            "dimensions:\n  - field: repository\n  - field: repository\n",
        ))
        .expect_err("declared twice");
        assert_eq!(
            error,
            DeclarationError::DuplicateField {
                dataset: "git_commits".to_owned(),
                field: "repository".to_owned(),
                role: "dimension",
            }
        );
    }

    #[test]
    fn a_declaration_without_a_row_identity_is_refused() {
        let error = declare(&MINIMAL.replace("row_identity: [commit_hash]", "row_identity: []"))
            .expect_err("no fact grain");
        assert_eq!(
            error,
            DeclarationError::EmptyRowIdentity {
                dataset: "git_commits".to_owned(),
            }
        );
    }

    #[test]
    fn an_unknown_key_in_a_declaration_is_refused() {
        let document: Result<DatasetDocument, _> = serde_yaml::from_str(&with(
            "dimensions:\n  - field: repository\n    labelfield: x\n",
        ));
        assert!(document.is_err());
    }
}
