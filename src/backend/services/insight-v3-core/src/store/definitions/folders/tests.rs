use uuid::Uuid;

use super::*;
use crate::store::definitions::visibility::assert_bound;

fn callers() -> [Caller; 2] {
    let person = PersonId::new(Uuid::from_u128(7));

    [Caller::administrator(person), Caller::member(person)]
}

fn an_id() -> FolderId {
    FolderId::parse("0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b")
        .unwrap_or_else(|error| panic!("the id must parse: {error}"))
}

fn delivery() -> DefinitionId {
    DefinitionId::parse("delivery").unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn every_folder_query_binds_a_value_to_each_mark() {
    for caller in callers() {
        let statements = [
            list_statement(caller),
            unfiled_statement(caller),
            holding_statement(&delivery(), caller),
            add_entry_statement(an_id(), &delivery(), caller, Entry::Carrying),
            add_entry_statement(an_id(), &delivery(), caller, Entry::Filing),
            remove_entry_statement(an_id(), &delivery(), caller),
        ];

        for statement in &statements {
            assert_bound(statement);
        }
    }
}

#[test]
fn the_folder_list_keeps_the_folders_the_caller_sees_counting_the_dashboards_they_see() {
    for caller in callers() {
        let statement = list_statement(caller);

        assert!(
            statement.sql.contains(&folder_visible("folders", caller).0),
            "{}",
            statement.sql
        );
        assert!(
            statement.sql.contains(&dashboard_visible(caller).0),
            "{}",
            statement.sql
        );
    }
}

#[test]
fn an_entry_lands_only_on_a_dashboard_the_caller_sees_and_never_stops_carrying_here() {
    for caller in callers() {
        let statement = add_entry_statement(an_id(), &delivery(), caller, Entry::Filing);

        assert!(
            statement.sql.contains(&dashboard_visible(caller).0),
            "{}",
            statement.sql
        );
        assert!(
            statement
                .sql
                .contains("carries = folder_entries.carries OR VALUES(carries)"),
            "{}",
            statement.sql
        );
    }
}

#[test]
fn an_entry_goes_only_while_the_caller_sees_its_dashboard() {
    for caller in callers() {
        let statement = remove_entry_statement(an_id(), &delivery(), caller);

        assert!(
            statement.sql.contains(&dashboard_visible(caller).0),
            "{}",
            statement.sql
        );
    }
}

#[test]
fn an_entry_carries_only_while_its_placer_owns_the_dashboard_or_holds_an_editor_grant() {
    let statement = add_entry_statement(an_id(), &delivery(), callers()[0], Entry::Carrying);

    for part in [
        "? AND (dashboards.owner_person_id IS NULL",
        "OR dashboards.owner_person_id = ?",
        "qualifying.level = 'editor'",
        "qualifying.grantee IN (?, 'everyone')",
        "placed_by = IF(VALUES(carries), VALUES(placed_by), folder_entries.placed_by)",
    ] {
        assert!(statement.sql.contains(part), "{part}: {}", statement.sql);
    }
}

#[test]
fn the_folder_cap_counts_one_owners_folders_under_the_folders_lock() {
    assert!(
        COUNT_FOLDERS.ends_with("WHERE owner_person_id = ?"),
        "{COUNT_FOLDERS}"
    );
    assert!(LOCK_FOLDERS.ends_with("FOR UPDATE"), "{LOCK_FOLDERS}");
}
