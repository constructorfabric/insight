use super::*;

#[test]
fn a_name_is_stored_without_the_spaces_around_it() {
    let parsed = FolderName::parse("  Platform  ")
        .unwrap_or_else(|error| panic!("a padded name must parse: {error}"));

    assert_eq!(parsed.as_str(), "Platform");
}

#[test]
fn a_name_of_nothing_but_spaces_is_refused() {
    for blank in ["", "   ", "\t"] {
        assert!(
            matches!(FolderName::parse(blank), Err(FolderError::Name)),
            "{blank:?}"
        );
    }
}

#[test]
fn a_name_may_run_to_sixty_four_characters_and_no_further() {
    assert!(FolderName::parse(&"é".repeat(64)).is_ok());
    assert!(matches!(
        FolderName::parse(&"é".repeat(65)),
        Err(FolderError::Name)
    ));
}

#[test]
fn two_names_differing_only_in_case_are_the_same_name() {
    let upper = FolderName::parse("Platform").unwrap_or_else(|error| panic!("{error}"));
    let lower = FolderName::parse("platform").unwrap_or_else(|error| panic!("{error}"));

    assert!(upper.same_as(&lower));
    assert_ne!(upper.as_str(), lower.as_str());
}

#[test]
fn an_id_reads_back_as_the_text_it_was_parsed_from() {
    let text = "0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

    let id = FolderId::parse(text).unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(id.to_string(), text);
}

#[test]
fn an_id_that_is_not_a_uuid_is_refused() {
    assert!(matches!(FolderId::parse("platform"), Err(FolderError::Id)));
}
