//! What a dataset declaration says, before and after the field catalog checks it.
//!
//! INVARIANT: a [`Dataset`] has every column resolved, so no layer above re-asks
//! whether one exists.

use serde::Deserialize;

use crate::domain::field_catalog::model::ReadDiscipline;

/// How a query names a dataset, parsed so no raw string reaches a lookup.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DatasetKey(String);

impl DatasetKey {
    pub fn parse(raw: &str) -> Option<Self> {
        let mut characters = raw.chars();
        match characters.next() {
            Some(first) if first.is_ascii_lowercase() => {}
            Some(_) | None => return None,
        }
        characters
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            .then(|| Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DatasetKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetDocument {
    pub key: String,
    pub database: String,
    pub relation: String,
    pub read_discipline: ReadDiscipline,
    pub tenant_field: String,
    pub time_fields: Vec<TimeFieldDocument>,
    #[serde(default)]
    pub dimensions: Vec<DimensionDocument>,
    #[serde(default)]
    pub measurables: Vec<MeasurableDocument>,
    pub row_identity: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeFieldDocument {
    pub field: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionDocument {
    pub field: String,
    #[serde(default)]
    pub label_field: Option<String>,
    /// Required of a nullable column and refused of any other.
    #[serde(default)]
    pub absent_value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurableDocument {
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dataset {
    pub key: DatasetKey,
    pub database: String,
    pub relation: String,
    pub read_discipline: ReadDiscipline,
    pub tenant_field: String,
    pub time_fields: Vec<TimeField>,
    pub dimensions: Vec<Dimension>,
    pub measurables: Vec<Measurable>,
    pub row_identity: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeField {
    pub field: String,
    pub nullable: bool,
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dimension {
    pub field: String,
    pub label_field: Option<String>,
    /// Present exactly when the column is nullable.
    pub absent_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurable {
    pub field: String,
}

impl Dataset {
    pub fn dimension(&self, field: &str) -> Option<&Dimension> {
        self.dimensions
            .iter()
            .find(|dimension| dimension.field == field)
    }

    pub fn measurable(&self, field: &str) -> Option<&Measurable> {
        self.measurables
            .iter()
            .find(|measurable| measurable.field == field)
    }

    pub fn time_field(&self, field: &str) -> Option<&TimeField> {
        self.time_fields
            .iter()
            .find(|candidate| candidate.field == field)
    }

    /// INVARIANT: validation admits exactly one default, so this never fails on
    /// a declaration the loader accepted.
    pub fn default_time_field(&self) -> Option<&TimeField> {
        self.time_fields.iter().find(|field| field.default)
    }

    pub fn dimension_names(&self) -> Vec<&str> {
        self.dimensions
            .iter()
            .map(|dimension| dimension.field.as_str())
            .collect()
    }

    pub fn measurable_names(&self) -> Vec<&str> {
        self.measurables
            .iter()
            .map(|measurable| measurable.field.as_str())
            .collect()
    }

    pub fn time_field_names(&self) -> Vec<&str> {
        self.time_fields
            .iter()
            .map(|field| field.field.as_str())
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_dataset_key_is_lowercase_snake_case() {
        assert!(DatasetKey::parse("git_commits").is_some());
        assert!(DatasetKey::parse("git2").is_some());
        for rejected in ["Git", "_git", "2git", "git-commits", "git.commits", ""] {
            assert!(DatasetKey::parse(rejected).is_none(), "{rejected}");
        }
    }
}
