use uuid::Uuid;

use super::*;
use crate::domain::definition::DefinitionKind;
use crate::domain::folders::FolderId;
use crate::store::definitions::visibility::assert_bound;

fn caller() -> Caller {
    Caller::administrator(PersonId::new(Uuid::from_u128(
        0x0192_a1b2_0000_7000_8000_0000_0000_0001,
    )))
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

fn query(needle: &str) -> DashboardQuery {
    DashboardQuery {
        needle: needle.to_owned(),
        ..DashboardQuery::default()
    }
}

#[test]
fn every_page_and_count_keeps_only_what_the_caller_sees_and_binds_every_mark() {
    let queries = [
        query(""),
        DashboardQuery {
            owned: Some(Owned::Me),
            ..query("")
        },
        DashboardQuery {
            owned: Some(Owned::Shared),
            folder: Some(FolderFilter::In(an_id())),
            ..query("")
        },
        DashboardQuery {
            folder: Some(FolderFilter::Unfiled),
            tags: Some(any_of(&["Alpha"])),
            ..query("deliv")
        },
    ];
    let (visible, _) = dashboard_visible(caller());

    for asked in &queries {
        let paged = page_statement(caller(), asked, a_page());
        let counted = count_statement(caller(), asked);

        for statement in [&paged, &counted] {
            assert!(
                statement.sql.contains(&visible),
                "{asked:?}: {}",
                statement.sql
            );
            assert_bound(statement);
        }
    }
}

#[test]
fn a_folder_filter_keeps_only_entries_of_a_folder_the_caller_sees() {
    let filtered = DashboardQuery {
        folder: Some(FolderFilter::In(an_id())),
        ..query("")
    };

    let statement = page_statement(caller(), &filtered, a_page());

    assert!(
        statement.sql.contains("filter_entries.folder_id = ?"),
        "{}",
        statement.sql
    );
    assert!(
        statement.sql.contains("filter_folder_grants"),
        "{}",
        statement.sql
    );
}

#[test]
fn standings_are_read_by_the_ids_of_one_page_at_once() {
    let ids = ["one", "two", "three"];

    for template in [GRANTS_REACHING, CARRIERS] {
        let statement = over_ids(template, caller(), &ids);

        assert!(statement.sql.contains("IN (?, ?, ?)"), "{}", statement.sql);
        assert_bound(&statement);
    }
    assert!(CARRIERS.contains("carrying.carries"), "{CARRIERS}");
}

#[test]
fn an_owner_column_reads_as_ownerless_a_person_or_a_refusal() {
    let person = "0192a1b2-0000-7000-8000-000000000001";

    assert!(matches!(owner_of(None), Ok(Owner::Unowned)));
    assert!(matches!(owner_of(Some(person)), Ok(Owner::Person(found)) if found == caller().person));
    assert!(matches!(
        owner_of(Some("someone")),
        Err(DefinitionStoreError::UnreadableRow("owner"))
    ));
    assert!(matches!(
        level_of("owner"),
        Err(DefinitionStoreError::UnreadableRow("level"))
    ));
}
