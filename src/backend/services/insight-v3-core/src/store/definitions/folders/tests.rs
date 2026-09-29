use super::*;
use crate::domain::definition::{DefinitionKind, Page};
use crate::domain::folders::FolderId;
use crate::store::definitions::{UPSERT, sql};

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

#[test]
fn a_body_write_names_no_folder_so_a_rewrite_keeps_it() {
    let upsert = sql(UPSERT, DefinitionKind::Dashboard);

    assert!(!upsert.contains("folder_id"), "{upsert}");
}

#[test]
fn a_page_of_one_folder_binds_the_folder_and_keeps_the_search() {
    let statement = page_statement("deliv", a_page(), FolderFilter::In(an_id()));

    assert!(statement.sql.contains("folder_id = ?"), "{}", statement.sql);
    assert!(
        statement.sql.contains("name LIKE ? OR body LIKE ?"),
        "{}",
        statement.sql
    );
    assert_eq!(
        bound(&statement),
        [
            "'%deliv%'",
            "'%deliv%'",
            "'0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b'",
            "20",
            "40",
        ]
    );
}

#[test]
fn a_page_of_the_unfiled_asks_for_no_folder_at_all() {
    let statement = page_statement("", a_page(), FolderFilter::Unfiled);
    let counted = count_statement("", FolderFilter::Unfiled);

    for query in [&statement.sql, &counted.sql] {
        assert!(query.contains("folder_id IS NULL"), "{query}");
    }
    assert_eq!(bound(&statement), ["'%%'", "'%%'", "20", "40"]);
}

#[test]
fn filing_binds_the_folder_then_the_dashboard() {
    let name = crate::domain::definition::DefinitionName::parse("delivery")
        .unwrap_or_else(|error| panic!("{error}"));

    let filed = file_statement(&name, Some(an_id()));
    let unfiled = file_statement(&name, None);

    assert_eq!(
        bound(&filed),
        ["'0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b'", "'delivery'"]
    );
    assert_eq!(bound(&unfiled), ["NULL", "'delivery'"]);
}
