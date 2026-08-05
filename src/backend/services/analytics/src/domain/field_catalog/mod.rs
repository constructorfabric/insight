//! The field catalog: the semantic layer's typed, role-annotated view of every
//! product dataset — the validation universe measures and expressions are
//! checked against. Built once from two committed halves: authored roles
//! (`roles.yaml`) joined with a ClickHouse type snapshot (`types.snapshot.json`).

#[cfg(test)]
mod live_tests;
mod loader;
pub mod model;

pub use loader::field_catalog;

/// Render the catalog as a compact, human-readable listing for the
/// `field-catalog` subcommand.
pub fn render() -> String {
    use std::fmt::Write as _;

    let mut out = String::from("# Field catalog\n");
    for dataset in &field_catalog().datasets {
        let _ = write!(
            out,
            "\n## {} ({}, read: {:?})\n",
            dataset.key,
            dataset.relation(),
            dataset.read_discipline
        );
        for field in &dataset.fields {
            let null = if field.nullable { " nullable" } else { "" };
            let _ = writeln!(
                out,
                "  - {:<20} {:<10?} {:?}{}",
                field.name, field.role, field.ty, null
            );
        }
    }
    out
}
