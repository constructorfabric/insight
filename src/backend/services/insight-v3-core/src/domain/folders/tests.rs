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
fn an_id_reads_back_as_the_text_it_was_parsed_from() {
    let text = "0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

    let id = FolderId::parse(text).unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(id.to_string(), text);
}

#[test]
fn an_id_that_is_not_a_uuid_is_refused() {
    assert!(matches!(FolderId::parse("platform"), Err(FolderError::Id)));
}

#[test]
fn a_filter_reads_unfiled_or_a_folder_id() {
    let id = "0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

    assert!(matches!(
        FolderFilter::parse("unfiled"),
        Ok(FolderFilter::Unfiled)
    ));
    assert!(
        matches!(FolderFilter::parse(id), Ok(FolderFilter::In(parsed)) if parsed.to_string() == id)
    );
    assert!(matches!(
        FolderFilter::parse("Platform"),
        Err(FolderError::Id)
    ));
}
