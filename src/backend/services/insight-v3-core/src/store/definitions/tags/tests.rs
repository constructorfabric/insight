use super::*;
use crate::domain::definition::{DefinitionKind, Page};
use crate::domain::folders::FolderId;

fn bound(statement: &Statement) -> Vec<String> {
    statement
        .values
        .as_ref()
        .map(|values| values.0.iter().map(ToString::to_string).collect())
        .unwrap_or_default()
}

fn an_id() -> FolderId {
    FolderId::parse("0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b")
        .unwrap_or_else(|error| panic!("the id must parse: {error}"))
}

fn a_page() -> Page {
    Page::parse(Some(20), Some(40)).unwrap_or_else(|error| panic!("the page must parse: {error}"))
}

fn any_of(values: &[&str]) -> TagFilter {
    let owned: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();

    TagFilter::parse(DefinitionKind::Dashboard, &owned)
        .unwrap_or_else(|error| panic!("the filter must parse: {error}"))
        .unwrap_or_else(|| panic!("a filter naming tags is one"))
}

#[test]
fn a_page_by_tag_binds_the_search_then_each_tag_then_the_page() {
    let statement = page_statement("deliv", a_page(), None, &any_of(&["Alpha", "Beta"]));

    assert!(
        statement.sql.contains("name LIKE ? OR body LIKE ?"),
        "{}",
        statement.sql
    );
    assert!(
        statement.sql.contains("tags.name IN (?, ?)"),
        "{}",
        statement.sql
    );
    assert!(!statement.sql.contains("folder_id"), "{}", statement.sql);
    assert_eq!(
        bound(&statement),
        ["'%deliv%'", "'%deliv%'", "'Alpha'", "'Beta'", "20", "40"]
    );
}

#[test]
fn a_page_by_tag_in_one_folder_binds_the_folder_before_the_tags() {
    let statement = page_statement(
        "",
        a_page(),
        Some(FolderFilter::In(an_id())),
        &any_of(&["Alpha"]),
    );

    assert!(statement.sql.contains("folder_id = ?"), "{}", statement.sql);
    assert_eq!(
        bound(&statement),
        [
            "'%%'",
            "'%%'",
            "'0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b'",
            "'Alpha'",
            "20",
            "40",
        ]
    );
}

#[test]
fn a_count_by_tag_among_the_unfiled_asks_for_no_folder_and_no_page() {
    let counted = count_statement("", Some(FolderFilter::Unfiled), &any_of(&["Alpha"]));

    assert!(counted.sql.contains("folder_id IS NULL"), "{}", counted.sql);
    assert!(counted.sql.contains("tags.name IN (?)"), "{}", counted.sql);
    assert_eq!(bound(&counted), ["'%%'", "'%%'", "'Alpha'"]);
}
