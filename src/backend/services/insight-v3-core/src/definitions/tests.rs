use super::*;

#[test]
fn names_reject_anything_outside_the_identifier_charset() {
    assert!(DefinitionName::parse("commits_per_day").is_ok());
    assert!(DefinitionName::parse("").is_err());
    assert!(DefinitionName::parse("drop table").is_err());
    assert!(DefinitionName::parse("a`b").is_err());
    assert!(DefinitionName::parse(&"a".repeat(129)).is_err());
}

#[test]
fn each_kind_has_its_own_table() {
    assert_eq!(DefinitionKind::Metric.table(), "metrics");
    assert_eq!(DefinitionKind::Widget.table(), "widgets");
    assert_eq!(DefinitionKind::Dashboard.table(), "dashboards");
}

#[test]
fn the_pattern_the_model_is_given_states_the_rule_the_store_enforces() {
    assert!(
        DefinitionName::PATTERN.contains(&MAX_NAME_CHARS.to_string()),
        "{}",
        DefinitionName::PATTERN
    );
    assert!(DefinitionName::parse(&"a".repeat(MAX_NAME_CHARS)).is_ok());
    assert!(DefinitionName::parse(&"a".repeat(MAX_NAME_CHARS + 1)).is_err());
}
