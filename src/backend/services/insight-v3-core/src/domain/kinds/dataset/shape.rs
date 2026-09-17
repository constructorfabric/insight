//! Whether a submitted declaration is even shaped like one.
//!
//! Reading the body into a [`Declaration`] stops at the first thing it cannot
//! read, and answers with a place in the text rather than a place in the
//! document. A reader correcting a form needs every problem at once, against
//! the field that carries it, so the shape is checked here first and the
//! deserialiser only ever sees a body it can read.

use serde_json::Value;

use super::declaration::{FieldRole, FieldType, PersonHandle};
use crate::domain::violation::{Reason, Violation};

/// What a declaration may say at the top.
const DECLARATION_KEYS: [&str; 4] = ["title", "description", "fields", "row_identity"];
/// What one field may say.
const FIELD_KEYS: [&str; 8] = [
    "name",
    "path",
    "type",
    "role",
    "description",
    "absent_value",
    "person",
    "default_clock",
];

/// Every way this body is not a declaration, each against the place in it that
/// is wrong.
pub(crate) fn check(body: &Value) -> Vec<Violation> {
    let mut violations = Vec::new();

    let Some(declaration) = body.as_object() else {
        return vec![Violation::new(
            "body",
            Reason::Malformed,
            "a declaration is an object",
        )];
    };

    unknown_keys(declaration.keys(), &DECLARATION_KEYS, "", &mut violations);
    text(declaration.get("title"), "title", true, &mut violations);
    text(
        declaration.get("description"),
        "description",
        false,
        &mut violations,
    );
    check_fields(declaration.get("fields"), &mut violations);
    check_row_identity(declaration.get("row_identity"), &mut violations);

    violations
}

fn check_fields(fields: Option<&Value>, violations: &mut Vec<Violation>) {
    let Some(fields) = fields else {
        violations.push(Violation::new(
            "fields",
            Reason::Missing,
            "a dataset declares the fields its records hold",
        ));
        return;
    };
    let Some(fields) = fields.as_array() else {
        violations.push(Violation::new(
            "fields",
            Reason::Malformed,
            "fields is a list",
        ));
        return;
    };

    for (index, field) in fields.iter().enumerate() {
        let at = format!("fields[{index}]");
        let Some(field) = field.as_object() else {
            violations.push(Violation::new(
                at,
                Reason::Malformed,
                "a field is an object",
            ));
            continue;
        };

        unknown_keys(field.keys(), &FIELD_KEYS, &at, violations);
        text(field.get("name"), &format!("{at}.name"), true, violations);
        text(field.get("path"), &format!("{at}.path"), true, violations);
        text(
            field.get("description"),
            &format!("{at}.description"),
            false,
            violations,
        );
        text(
            field.get("absent_value"),
            &format!("{at}.absent_value"),
            false,
            violations,
        );
        word(
            field.get("type"),
            &format!("{at}.type"),
            &FieldType::ALL,
            true,
            violations,
        );
        word(
            field.get("role"),
            &format!("{at}.role"),
            &FieldRole::ALL,
            false,
            violations,
        );
        word(
            field.get("person"),
            &format!("{at}.person"),
            &PersonHandle::ALL,
            false,
            violations,
        );
        flag(
            field.get("default_clock"),
            &format!("{at}.default_clock"),
            violations,
        );
    }
}

fn check_row_identity(identity: Option<&Value>, violations: &mut Vec<Violation>) {
    let Some(identity) = identity else {
        return;
    };
    let Some(identity) = identity.as_array() else {
        violations.push(Violation::new(
            "row_identity",
            Reason::Malformed,
            "row_identity is a list of field names",
        ));
        return;
    };

    for (index, named) in identity.iter().enumerate() {
        if !named.is_string() {
            violations.push(Violation::new(
                format!("row_identity[{index}]"),
                Reason::Malformed,
                "row_identity names a field",
            ));
        }
    }
}

/// A key nothing reads is a mistake worth naming: it is usually the right
/// value under the wrong name.
fn unknown_keys<'a>(
    given: impl Iterator<Item = &'a String>,
    admissible: &[&str],
    at: &str,
    violations: &mut Vec<Violation>,
) {
    for key in given {
        if admissible.contains(&key.as_str()) {
            continue;
        }

        let path = if at.is_empty() {
            key.clone()
        } else {
            format!("{at}.{key}")
        };
        violations.push(Violation::new(
            path,
            Reason::Unknown,
            format!("nothing reads `{key}`; admissible: {}", listed(admissible)),
        ));
    }
}

fn text(value: Option<&Value>, at: &str, required: bool, violations: &mut Vec<Violation>) {
    match value {
        None | Some(Value::Null) if required => violations.push(Violation::new(
            at,
            Reason::Missing,
            format!("`{at}` is text this declaration has to carry"),
        )),
        Some(value) if !value.is_string() && !value.is_null() => violations.push(Violation::new(
            at,
            Reason::Malformed,
            format!("`{at}` is text"),
        )),
        _ => {}
    }
}

fn word(
    value: Option<&Value>,
    at: &str,
    admissible: &[&str],
    required: bool,
    violations: &mut Vec<Violation>,
) {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        if required {
            violations.push(Violation::new(
                at,
                Reason::Missing,
                format!("`{at}` is one of {}", listed(admissible)),
            ));
        }
        return;
    };

    let said = value.as_str();
    if !said.is_some_and(|said| admissible.contains(&said)) {
        violations.push(Violation::new(
            at,
            Reason::NotAdmissible,
            format!("admissible: {}", listed(admissible)),
        ));
    }
}

fn flag(value: Option<&Value>, at: &str, violations: &mut Vec<Violation>) {
    if value.is_some_and(|value| !value.is_boolean() && !value.is_null()) {
        violations.push(Violation::new(
            at,
            Reason::Malformed,
            format!("`{at}` is true or false"),
        ));
    }
}

fn listed(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| format!("`{word}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
