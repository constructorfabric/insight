use super::*;

#[test]
fn ids_reject_anything_outside_the_identifier_charset() {
    assert!(DefinitionId::parse("commits_per_day").is_ok());
    assert!(DefinitionId::parse("").is_err());
    assert!(DefinitionId::parse("drop table").is_err());
    assert!(DefinitionId::parse("a`b").is_err());
    assert!(DefinitionId::parse(&"a".repeat(129)).is_err());
}

#[test]
fn every_kind_there_is_answers_for_itself_everywhere() {
    for kind in DefinitionKind::ALL {
        match kind {
            DefinitionKind::Metric | DefinitionKind::Widget | DefinitionKind::Dashboard => (),
        }
    }

    assert_eq!(
        DefinitionKind::ALL.len(),
        3,
        "a kind added to the enum must be added to ALL"
    );
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
        DefinitionId::PATTERN.contains(&MAX_ID_CHARS.to_string()),
        "{}",
        DefinitionId::PATTERN
    );
    assert!(DefinitionId::parse(&"a".repeat(MAX_ID_CHARS)).is_ok());
    assert!(DefinitionId::parse(&"a".repeat(MAX_ID_CHARS + 1)).is_err());
}

#[test]
fn a_generated_id_is_fresh_and_satisfies_the_id_rule() {
    let first = DefinitionId::generate();
    let second = DefinitionId::generate();

    assert_ne!(first, second);
    for generated in [&first, &second] {
        assert!(
            DefinitionId::parse(generated.as_str()).is_ok(),
            "should parse: {generated:?}"
        );
    }
}

#[test]
fn a_label_is_trimmed_and_kept_as_typed() {
    let cases = [
        ("Commits per day", "Commits per day"),
        ("  Team board  ", "Team board"),
        ("Отчёт / 2026", "Отчёт / 2026"),
        ("a", "a"),
    ];

    for (raw, expected) in cases {
        let parsed = DefinitionLabel::parse(raw)
            .unwrap_or_else(|error| panic!("should accept {raw:?}: {error}"));
        assert_eq!(parsed.as_str(), expected, "for {raw:?}");
    }

    let longest = "é".repeat(MAX_LABEL_CHARS);
    assert!(DefinitionLabel::parse(&longest).is_ok());
}

#[test]
fn a_label_rejects_empty_overlong_and_control_characters() {
    let overlong = "a".repeat(MAX_LABEL_CHARS + 1);
    let cases = [
        "",
        "   ",
        "\t\n",
        overlong.as_str(),
        "line\nbreak",
        "bell\u{7}",
        "nul\0",
    ];

    for raw in cases {
        assert!(
            matches!(DefinitionLabel::parse(raw), Err(DefinitionError::Label)),
            "should reject: {raw:?}"
        );
    }
}

#[test]
fn every_id_reads_as_a_label_unchanged() {
    for raw in [
        "commits_per_day",
        "a",
        "0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b",
    ] {
        let id = DefinitionId::parse(raw).unwrap_or_else(|error| panic!("{raw}: {error}"));
        let parsed = DefinitionLabel::parse(raw)
            .unwrap_or_else(|error| panic!("an id is a valid label: {raw}: {error}"));

        assert_eq!(DefinitionLabel::of_id(&id), parsed, "for {raw}");
    }
    let longest =
        DefinitionId::parse(&"a".repeat(MAX_ID_CHARS)).unwrap_or_else(|error| panic!("{error}"));
    assert!(DefinitionLabel::parse(longest.as_str()).is_ok());
}
