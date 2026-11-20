use serde::{Deserialize, Serialize};

pub(crate) const MAX_PATH_SEGMENTS: usize = 8;
pub(crate) const BUCKET_COLUMN: &str = "bucket";

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
    pub(crate) const ALL: [&'static str; 5] = ["string", "int", "float", "bool", "datetime"];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FieldRole {
    Dimension,
    Measurable,
    Time,
}

impl FieldRole {
    pub(crate) const ALL: [&'static str; 3] = ["dimension", "measurable", "time"];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PersonHandle {
    Email,
    Id,
}

impl PersonHandle {
    pub(crate) const ALL: [&'static str; 2] = ["email", "id"];
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) r#type: FieldType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) role: Option<FieldRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) absent_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) person: Option<PersonHandle>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) default_clock: bool,
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip predicate")]
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Declaration {
    pub(crate) title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    pub(crate) fields: Vec<Field>,
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

    pub(crate) fn default_clock(&self) -> Option<&Field> {
        self.fields.iter().find(|field| field.default_clock)
    }
}

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
    segments.push(current);

    segments
}

#[cfg(test)]
mod tests;
