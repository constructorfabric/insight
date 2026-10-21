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

    let dashboards = DefinitionKind::Dashboard;

    assert!(matches!(
        FolderFilter::parse(dashboards, "unfiled"),
        Ok(FolderFilter::Unfiled)
    ));
    assert!(
        matches!(FolderFilter::parse(dashboards, id), Ok(FolderFilter::In(parsed)) if parsed.to_string() == id)
    );
    assert!(matches!(
        FolderFilter::parse(dashboards, "Platform"),
        Err(FolderError::Id)
    ));
}

#[test]
fn only_dashboards_are_filtered_by_folder() {
    assert!(matches!(
        FolderFilter::parse(DefinitionKind::Metric, "unfiled"),
        Err(FolderError::NotFiled("metrics"))
    ));
}

#[test]
fn a_folder_holds_a_dashboard_filed_among_its_owners_folders_and_no_other() {
    let caller = PersonId::new(uuid::Uuid::from_u128(1));
    let someone = PersonId::new(uuid::Uuid::from_u128(2));
    let id = FolderId::new();
    let mine = Owner::Person(caller);
    let cases = [
        (mine, Some(mine), "fits"),
        (mine, Some(Owner::Unowned), "wrong shelf"),
        (mine, Some(Owner::Person(someone)), "not found"),
        (mine, None, "not found"),
        (Owner::Unowned, Some(Owner::Unowned), "fits"),
        (Owner::Unowned, Some(mine), "wrong shelf"),
        (Owner::Unowned, Some(Owner::Person(someone)), "not found"),
        (Owner::Unowned, None, "not found"),
    ];

    for (folders_of, owner, expected) in cases {
        let answered = match fits(folders_of, caller, id, owner) {
            Ok(()) => "fits",
            Err(FolderError::WrongShelf(_)) => "wrong shelf",
            Err(FolderError::FolderNotFound(_)) => "not found",
            Err(other) => panic!("unexpected {other:?}"),
        };

        assert_eq!(
            answered, expected,
            "filed among {folders_of:?} in {owner:?}"
        );
    }
}
