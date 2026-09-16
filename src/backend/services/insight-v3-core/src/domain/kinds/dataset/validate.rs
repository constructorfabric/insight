//! Whether a declaration can be stored, and everything wrong with it if not.
//!
//! Every problem is reported at once, each against the field of the submitted
//! body it belongs to, so that a declaration with several mistakes is corrected
//! in one pass rather than one refusal at a time.

use std::collections::HashSet;

use super::declaration::{BUCKET_COLUMN, Declaration, FieldType, MAX_PATH_SEGMENTS};
use crate::domain::definition::DefinitionName;

/// Catalogue paths a dataset may not take, because a static segment of the
/// portal already answers on them.
const RESERVED_NAMES: [&str; 4] = ["metrics", "widgets", "dashboards", "datasets"];

/// Why one part of a declaration cannot be stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    Missing,
    Unknown,
    Duplicate,
    Malformed,
    NotAdmissible,
}

impl Reason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "MISSING",
            Self::Unknown => "UNKNOWN",
            Self::Duplicate => "DUPLICATE",
            Self::Malformed => "MALFORMED",
            Self::NotAdmissible => "NOT_ADMISSIBLE",
        }
    }
}

/// One problem, addressed to the place in the body that carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Violation {
    /// Where the problem is, as the submitted body is shaped:
    /// `fields[2].type`, `row_identity[0]`, `name`.
    pub(crate) field: String,
    pub(crate) reason: Reason,
    pub(crate) detail: String,
}

impl Violation {
    fn new(field: impl Into<String>, reason: Reason, detail: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason,
            detail: detail.into(),
        }
    }

    pub(crate) fn reason_code(&self) -> &'static str {
        self.reason.as_str()
    }
}

/// Checks a declaration, reporting every problem rather than the first.
pub(crate) fn validate(name: &str, declaration: &Declaration) -> Vec<Violation> {
    let mut violations = Vec::new();

    check_name(name, &mut violations);
    check_title(declaration, &mut violations);
    check_fields(declaration, &mut violations);
    check_default_clock(declaration, &mut violations);
    check_row_identity(declaration, &mut violations);

    violations
}

fn check_name(name: &str, violations: &mut Vec<Violation>) {
    if DefinitionName::parse(name).is_err() {
        violations.push(Violation::new(
            "name",
            Reason::Malformed,
            "a name is letters, digits, underscore and dash, up to 128 characters",
        ));
        return;
    }

    if RESERVED_NAMES.contains(&name) {
        violations.push(Violation::new(
            "name",
            Reason::NotAdmissible,
            format!("`{name}` is a catalogue path; admissible: any other name"),
        ));
    }
}

fn check_title(declaration: &Declaration, violations: &mut Vec<Violation>) {
    if declaration.title.trim().is_empty() {
        violations.push(Violation::new(
            "title",
            Reason::Missing,
            "a dataset is listed by its title, so it needs one",
        ));
    }
}

fn check_fields(declaration: &Declaration, violations: &mut Vec<Violation>) {
    if declaration.fields.is_empty() {
        violations.push(Violation::new(
            "fields",
            Reason::Missing,
            "a dataset declares at least one field; nothing can be asked of one that declares none",
        ));
    }

    let mut seen: HashSet<&str> = HashSet::with_capacity(declaration.fields.len());

    for (index, field) in declaration.fields.iter().enumerate() {
        let at = |part: &str| format!("fields[{index}].{part}");

        if DefinitionName::parse(&field.name).is_err() {
            violations.push(Violation::new(
                at("name"),
                Reason::Malformed,
                "a field name is letters, digits, underscore and dash, up to 128 characters",
            ));
        } else if field.name == BUCKET_COLUMN {
            violations.push(Violation::new(
                at("name"),
                Reason::NotAdmissible,
                format!("`{BUCKET_COLUMN}` is the column a windowed run injects"),
            ));
        } else if !seen.insert(field.name.as_str()) {
            violations.push(Violation::new(
                at("name"),
                Reason::Duplicate,
                format!("`{}` is declared more than once", field.name),
            ));
        }

        check_path(&field.path, &at("path"), violations);

        if field.default_clock && field.r#type != FieldType::Datetime {
            violations.push(Violation::new(
                at("default_clock"),
                Reason::NotAdmissible,
                "the main date is a datetime field; a window cannot select by anything else",
            ));
        }

        if field.person.is_some() && field.r#type != FieldType::String {
            violations.push(Violation::new(
                at("person"),
                Reason::NotAdmissible,
                "a person is carried by a string field, because a handle is text",
            ));
        }

        if field.absent_value.is_some() && field.r#type != FieldType::String {
            violations.push(Violation::new(
                at("absent_value"),
                Reason::NotAdmissible,
                "a substitute stands in for a missing value where it is read, which is text",
            ));
        }
    }
}

fn check_path(path: &str, at: &str, violations: &mut Vec<Violation>) {
    if path.is_empty() {
        violations.push(Violation::new(
            at,
            Reason::Missing,
            "a field reads a key of the record, so it needs a path",
        ));
        return;
    }

    if path.ends_with('\\') && !path.ends_with("\\\\") {
        violations.push(Violation::new(
            at,
            Reason::Malformed,
            "a path ends mid-escape; a literal backslash is written twice",
        ));
        return;
    }

    let segments = super::declaration::path_segments(path);

    if segments.iter().any(String::is_empty) {
        violations.push(Violation::new(
            at,
            Reason::Malformed,
            "a path segment is empty; a literal dot in a key is escaped with a backslash",
        ));
    }

    if segments.len() > MAX_PATH_SEGMENTS {
        violations.push(Violation::new(
            at,
            Reason::NotAdmissible,
            format!("a path reads at most {MAX_PATH_SEGMENTS} segments deep"),
        ));
    }
}

fn check_default_clock(declaration: &Declaration, violations: &mut Vec<Violation>) {
    let clocks: Vec<&str> = declaration
        .fields
        .iter()
        .filter(|field| field.default_clock)
        .map(|field| field.name.as_str())
        .collect();

    if clocks.len() > 1 {
        violations.push(Violation::new(
            "fields",
            Reason::Duplicate,
            format!(
                "a record has one main date; these claim it: {}",
                clocks.join(", ")
            ),
        ));
    }
}

fn check_row_identity(declaration: &Declaration, violations: &mut Vec<Violation>) {
    let mut seen: HashSet<&str> = HashSet::with_capacity(declaration.row_identity.len());

    for (index, name) in declaration.row_identity.iter().enumerate() {
        let at = format!("row_identity[{index}]");

        if declaration.field(name).is_none() {
            violations.push(Violation::new(
                at,
                Reason::Unknown,
                format!(
                    "`{name}` is not a declared field; declared: {}",
                    declared(declaration)
                ),
            ));
            continue;
        }

        if !seen.insert(name.as_str()) {
            violations.push(Violation::new(
                at,
                Reason::Duplicate,
                format!("`{name}` is named twice"),
            ));
        }
    }
}

fn declared(declaration: &Declaration) -> String {
    if declaration.fields.is_empty() {
        return "none".to_owned();
    }

    declaration
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
