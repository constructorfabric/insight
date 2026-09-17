//! What a dataset says about the records it holds.
//!
//! A declaration is the one place the shape of a record is written down. The
//! compiler reads it to know where a value lives, the catalogue renders it, and
//! the assistant is told it instead of guessing from a sample.

use serde::{Deserialize, Serialize};

/// The longest key path a field may read, in segments.
pub(crate) const MAX_PATH_SEGMENTS: usize = 8;
/// The column a windowed run injects, which no field may claim.
pub(crate) const BUCKET_COLUMN: &str = "bucket";

/// What a value is read as, and with it what a metric may do with the field.
///
/// The type decides admissibility; [`FieldRole`] only describes. A numeric
/// identifier is therefore groupable, which it would not be if the role
/// decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FieldType {
    String,
    Int,
    Float,
    Bool,
    Datetime,
}

impl FieldType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::String => "a string",
            Self::Int => "a whole number",
            Self::Float => "a number",
            Self::Bool => "a flag",
            Self::Datetime => "a datetime",
        }
    }

    pub(crate) fn is_numeric(self) -> bool {
        match self {
            Self::Int | Self::Float => true,
            Self::String | Self::Bool | Self::Datetime => false,
        }
    }
}

/// What a field is for, as a reader is told it.
///
/// Advisory: it guides the catalogue and the assistant's choice of field, and
/// never widens or narrows what a metric may ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FieldRole {
    Dimension,
    Measurable,
    Time,
}

/// Which handle a field carries when it holds a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PersonHandle {
    Email,
    Id,
}

/// One field of a dataset, as declared.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Field {
    pub(crate) name: String,
    /// Where the value sits in a record, as dot-separated segments.
    pub(crate) path: String,
    pub(crate) r#type: FieldType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) role: Option<FieldRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    /// What a reader is shown where the value is empty. Presentation only: row
    /// identity reads the value before this stands in, so two records merely
    /// missing the key are never collapsed through one substitute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) absent_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) person: Option<PersonHandle>,
    /// Whether this field is the record's main date, the one a window selects
    /// by when a metric names none of its own.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) default_clock: bool,
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip predicate")]
fn is_false(value: &bool) -> bool {
    !*value
}

/// A dataset, as it is declared and stored.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Declaration {
    pub(crate) title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    pub(crate) fields: Vec<Field>,
    /// The fields that make two records the same record. Empty means every
    /// record stands on its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) row_identity: Vec<String>,
}

impl Declaration {
    pub(crate) fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.name == name)
    }

    pub(crate) fn field_names(&self) -> Vec<&str> {
        self.fields
            .iter()
            .map(|field| field.name.as_str())
            .collect()
    }

    /// The field a window selects by when a metric names none.
    pub(crate) fn default_clock(&self) -> Option<&Field> {
        self.fields.iter().find(|field| field.default_clock)
    }
}

/// One field's key path, split into the segments that address it.
///
/// A segment may hold a literal dot, escaped with a backslash, because a
/// payload is free to use one in a key.
pub(crate) fn path_segments(path: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut escaped = false;

    for character in path.chars() {
        match (escaped, character) {
            (true, other) => {
                current.push(other);
                escaped = false;
            }
            (false, '\\') => escaped = true,
            (false, '.') => segments.push(std::mem::take(&mut current)),
            (false, other) => current.push(other),
        }
    }
    // A trailing backslash escapes nothing; the validator refuses the path, and
    // dropping it here keeps this function total.
    segments.push(current);

    segments
}

#[cfg(test)]
mod tests;
