use uuid::Uuid;

use super::*;
use crate::domain::access::PersonId;

fn person() -> PersonId {
    PersonId::new(Uuid::from_u128(7))
}

fn callers() -> [Caller; 2] {
    [Caller::administrator(person()), Caller::member(person())]
}

fn marks(clause: &Clause) -> (usize, usize) {
    (clause.0.matches('?').count(), clause.1.len())
}

#[test]
fn every_clause_binds_a_value_to_each_mark() {
    for caller in callers() {
        let clauses = [
            folder_visible("folders", caller),
            dashboard_visible(caller),
            dashboard_unfiled(caller),
            dashboard_in("filter.folder_id", caller),
        ];

        for clause in &clauses {
            let (marked, bound) = marks(clause);
            assert_eq!(marked, bound, "{caller:?}: {}", clause.0);
        }
    }
}

#[test]
fn only_an_administrator_sees_ownerless_rows_or_owns_a_dashboard() {
    let (admin, _) = dashboard_visible(Caller::administrator(person()));
    let (member, _) = dashboard_visible(Caller::member(person()));

    assert!(
        admin.contains("owner_person_id IS NULL AND TRUE"),
        "{admin}"
    );
    assert!(
        member.contains("owner_person_id IS NULL AND FALSE"),
        "{member}"
    );
    assert!(member.contains("owner_person_id = ? AND FALSE"), "{member}");
}

#[test]
fn a_dashboard_is_seen_through_grants_and_carrying_entries_of_folders_seen() {
    let (clause, _) = dashboard_visible(Caller::member(person()));

    for part in [
        "visible_grants.grantee IN (?, 'everyone')",
        "visible_entries.carries",
        "visible_folder_grants.grantee IN (?, 'everyone')",
        "visible_folder.owner_person_id IS NOT NULL",
    ] {
        assert!(clause.contains(part), "{part} is missing: {clause}");
    }
}

#[test]
fn a_folder_is_seen_by_its_owner_in_any_role_and_through_its_grants() {
    let (clause, _) = folder_visible("folders", Caller::member(person()));

    for part in [
        "folders.owner_person_id = ?",
        "folders_grants.grantee IN (?, 'everyone')",
        "folders.owner_person_id IS NULL AND FALSE",
    ] {
        assert!(clause.contains(part), "{part} is missing: {clause}");
    }
}
