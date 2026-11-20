use std::fmt::Write as _;

use super::declaration::{Declaration, Field, FieldRole, FieldType};

const NOTHING_DECLARED: &str = "No datasets have been declared yet. An administrator declares one before there is anything to query.";

pub(crate) fn describe_all(declared: &[(String, Declaration)]) -> String {
    if declared.is_empty() {
        return NOTHING_DECLARED.to_owned();
    }

    declared
        .iter()
        .map(|(name, declaration)| describe(name, declaration))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn describe(name: &str, declaration: &Declaration) -> String {
    let mut written = String::new();

    let _ = writeln!(written, "{name}: {}", declaration.title);
    if let Some(description) = &declaration.description {
        let _ = writeln!(written, "  {description}");
    }

    for field in &declaration.fields {
        let _ = writeln!(written, "  - {}", field_line(field));
    }

    if !declaration.row_identity.is_empty() {
        let _ = writeln!(
            written,
            "  Two records are the same record when they agree on: {}",
            declaration.row_identity.join(", ")
        );
    }

    written
}

fn field_line(field: &Field) -> String {
    let mut written = format!("{} ({})", field.name, readable(field.r#type));

    if let Some(role) = field.role {
        let _ = write!(written, ", {}", purpose(role));
    }
    if field.default_clock {
        written.push_str(", the record's main date");
    }
    if let Some(substitute) = &field.absent_value {
        let _ = write!(written, ", shown as `{substitute}` where it is missing");
    }
    if let Some(description) = &field.description {
        let _ = write!(written, " - {description}");
    }

    written
}

fn readable(field_type: FieldType) -> &'static str {
    match field_type {
        FieldType::String => "text",
        FieldType::Int => "whole number",
        FieldType::Float => "number",
        FieldType::Bool => "yes or no",
        FieldType::Datetime => "date and time",
    }
}

fn purpose(role: FieldRole) -> &'static str {
    match role {
        FieldRole::Dimension => "something to group or filter by",
        FieldRole::Measurable => "something to measure",
        FieldRole::Time => "a moment in time",
    }
}

#[cfg(test)]
mod tests;
