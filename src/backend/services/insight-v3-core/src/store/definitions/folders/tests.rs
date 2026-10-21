use uuid::Uuid;

use super::*;
use crate::domain::definition::DefinitionKind;
use crate::store::definitions::{REPLACE_ONE, sql};

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

fn delivery() -> DefinitionId {
    DefinitionId::parse("delivery").unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn a_body_write_names_no_folder_so_a_rewrite_keeps_it() {
    let replace = sql(REPLACE_ONE, DefinitionKind::Dashboard);

    assert!(!replace.contains("folder_id"), "{replace}");
}

#[test]
fn the_folder_list_is_the_callers_and_the_ownerless_counting_what_they_see() {
    let statement = list_statement(caller());

    assert!(
        statement
            .sql
            .contains("WHERE folders.owner_person_id IS NULL OR folders.owner_person_id = ?"),
        "{}",
        statement.sql
    );
    assert!(statement.sql.contains(VISIBLE), "{}", statement.sql);
    assert!(
        statement
            .sql
            .contains("dashboard_placements.folder_id = folders.id"),
        "{}",
        statement.sql
    );
    assert_eq!(bound(&statement), [CALLER, CALLER, CALLER, CALLER]);
}

#[test]
fn the_unfiled_count_keeps_what_the_caller_sees_in_none_of_their_folders() {
    let statement = unfiled_statement(caller());

    assert!(statement.sql.contains(VISIBLE), "{}", statement.sql);
    assert!(statement.sql.contains(UNFILED), "{}", statement.sql);
    assert_eq!(bound(&statement), [CALLER, CALLER, CALLER]);
}

#[test]
fn filing_writes_the_ownerless_folder_or_the_persons_own_placement() {
    let folder = "'0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b'";
    let cases = [
        (
            Owner::Unowned,
            Some(an_id()),
            "UPDATE dashboards SET folder_id",
            vec![folder, "'delivery'"],
        ),
        (
            Owner::Unowned,
            None,
            "UPDATE dashboards SET folder_id",
            vec!["NULL", "'delivery'"],
        ),
        (
            Owner::Person(caller()),
            Some(an_id()),
            "INSERT INTO dashboard_placements",
            vec![CALLER, folder, "'delivery'", CALLER, CALLER, folder],
        ),
        (
            Owner::Person(caller()),
            None,
            "DELETE FROM dashboard_placements",
            vec![CALLER, "'delivery'"],
        ),
    ];

    for (folders_of, folder, starts, expected) in cases {
        let statement = file_statement(&delivery(), folders_of, folder);

        assert!(
            statement.sql.starts_with(starts),
            "{folders_of:?} {folder:?}: {}",
            statement.sql
        );
        assert_eq!(bound(&statement), expected, "{folders_of:?} {folder:?}");
    }
}

#[test]
fn only_the_callers_own_or_an_ownerless_folder_is_renamed_or_removed() {
    for template in [RENAME_FOLDER, DELETE_FOLDER] {
        let scoped = reachable(template);

        assert!(
            scoped.ends_with("(owner_person_id IS NULL OR owner_person_id = ?)"),
            "{scoped}"
        );
    }
}

#[test]
fn the_folder_cap_counts_one_persons_folders() {
    assert!(
        COUNT_FOLDERS.ends_with("WHERE owner_person_id = ?"),
        "{COUNT_FOLDERS}"
    );
}
