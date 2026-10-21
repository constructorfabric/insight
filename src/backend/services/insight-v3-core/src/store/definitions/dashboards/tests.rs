use uuid::Uuid;

use super::*;
use crate::domain::definition::DefinitionKind;
use crate::domain::folders::FolderId;

const CALLER: &str = "'0192a1b2-0000-7000-8000-000000000001'";

fn caller() -> PersonId {
    PersonId::new(Uuid::from_u128(0x0192_a1b2_0000_7000_8000_0000_0000_0001))
}

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

fn query(needle: &str) -> DashboardQuery {
    DashboardQuery {
        needle: needle.to_owned(),
        ..DashboardQuery::default()
    }
}

#[test]
fn the_visibility_rule_keeps_the_owner_the_shared_and_the_ownerless() {
    for part in [
        "dashboards.owner_person_id IS NULL",
        "dashboards.owner_person_id = ?",
        "dashboard_shares.dashboard = dashboards.id AND dashboard_shares.person = ?",
    ] {
        assert!(VISIBLE.contains(part), "{part} is missing: {VISIBLE}");
    }
    assert_eq!(VISIBLE.matches('?').count(), visible_values(caller()).len());
}

#[test]
fn every_page_and_count_keeps_only_what_the_caller_sees() {
    let queries = [
        query(""),
        DashboardQuery {
            owned: Some(Owned::Me),
            ..query("")
        },
        DashboardQuery {
            folder: Some(FolderFilter::Unfiled),
            tags: Some(any_of(&["Alpha"])),
            ..query("deliv")
        },
    ];

    for asked in &queries {
        let paged = page_statement(caller(), asked, a_page());
        let counted = count_statement(caller(), asked);

        for statement in [&paged, &counted] {
            assert!(
                statement.sql.contains(VISIBLE),
                "{asked:?}: {}",
                statement.sql
            );
            assert_eq!(
                statement.sql.matches('?').count(),
                bound(statement).len(),
                "every mark is bound for {asked:?}: {}",
                statement.sql
            );
        }
    }
}

#[test]
fn a_page_binds_the_caller_then_the_search_then_the_page() {
    let statement = page_statement(caller(), &query("deliv"), a_page());

    assert!(
        statement
            .sql
            .contains("dashboards.name LIKE ? OR dashboards.body LIKE ?"),
        "{}",
        statement.sql
    );
    assert!(
        statement
            .sql
            .contains("ORDER BY dashboards.name, dashboards.id"),
        "{}",
        statement.sql
    );
    assert_eq!(
        bound(&statement),
        [CALLER, CALLER, CALLER, "'%deliv%'", "'%deliv%'", "20", "40"]
    );
}

#[test]
fn a_page_by_tag_matches_the_name_in_any_scope() {
    let asked = DashboardQuery {
        tags: Some(any_of(&["Alpha", "Beta"])),
        ..query("")
    };

    let statement = page_statement(caller(), &asked, a_page());

    assert!(
        statement.sql.contains("tags.name IN (?, ?)"),
        "{}",
        statement.sql
    );
    assert!(!statement.sql.contains("owner_scope"), "{}", statement.sql);
    assert_eq!(bound(&statement)[5..], ["'Alpha'", "'Beta'", "20", "40"]);
}

#[test]
fn a_folder_keeps_an_ownerless_dashboard_by_its_folder_and_an_owned_one_by_the_callers_filing() {
    let (clause, values) = folder_clause(FolderFilter::In(an_id()), caller());

    assert!(
        clause.contains("dashboards.owner_person_id IS NULL AND dashboards.folder_id = ?"),
        "{clause}"
    );
    assert!(
        clause.contains("dashboard_placements.person = ?"),
        "{clause}"
    );
    let shown: Vec<String> = values.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        [
            "'0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b'",
            CALLER,
            "'0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b'",
        ]
    );
}

#[test]
fn unfiled_is_in_no_folder_of_the_callers_or_an_ownerless_one_in_none() {
    let (clause, values) = folder_clause(FolderFilter::Unfiled, caller());

    assert!(
        clause.contains("dashboards.owner_person_id IS NULL AND dashboards.folder_id IS NULL"),
        "{clause}"
    );
    assert!(clause.contains("NOT EXISTS"), "{clause}");
    assert_eq!(values.len(), 1);
}

#[test]
fn owned_by_me_and_shared_with_me_are_told_apart_by_the_owner() {
    let mine = page_statement(
        caller(),
        &DashboardQuery {
            owned: Some(Owned::Me),
            ..query("")
        },
        a_page(),
    );
    let shared = count_statement(
        caller(),
        &DashboardQuery {
            owned: Some(Owned::Shared),
            ..query("")
        },
    );

    assert!(
        mine.sql.contains("AND dashboards.owner_person_id = ?"),
        "{}",
        mine.sql
    );
    assert!(
        shared
            .sql
            .contains("dashboards.owner_person_id <> ? AND EXISTS"),
        "{}",
        shared.sql
    );
}

#[test]
fn an_owner_column_reads_as_ownerless_a_person_or_a_refusal() {
    let person = "0192a1b2-0000-7000-8000-000000000001";

    assert!(matches!(owner_of(None), Ok(Owner::Unowned)));
    assert!(matches!(owner_of(Some(person)), Ok(Owner::Person(found)) if found == caller()));
    assert!(matches!(
        owner_of(Some("someone")),
        Err(DefinitionStoreError::UnreadableRow("owner"))
    ));
}
