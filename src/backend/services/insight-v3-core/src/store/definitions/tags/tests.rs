use super::*;

fn person() -> PersonId {
    PersonId::new(Uuid::from_u128(0x0192_a1b2_0000_7000_8000_0000_0000_0001))
}

#[test]
fn a_tag_is_found_and_made_within_one_owners_scope() {
    assert!(
        FIND_TAG.contains("owner_scope = ? AND name = ?"),
        "{FIND_TAG}"
    );
    assert!(INSERT_TAG.contains("owner_person_id"), "{INSERT_TAG}");
    assert!(
        COUNT_TAGS.ends_with("WHERE owner_scope = ?"),
        "{COUNT_TAGS}"
    );
}

#[test]
fn the_ownerless_scope_is_the_empty_one_and_an_owners_is_their_id() {
    assert_eq!(scope_value(Owner::Unowned).to_string(), "''");
    assert_eq!(
        scope_value(Owner::Person(person())).to_string(),
        "'0192a1b2-0000-7000-8000-000000000001'"
    );
}

#[test]
fn listing_tags_counts_only_visible_dashboards_and_merges_scopes_by_name() {
    let statement = list_statement(person());

    assert!(statement.sql.contains(VISIBLE), "{}", statement.sql);
    assert!(
        statement.sql.contains("GROUP BY tags.name"),
        "{}",
        statement.sql
    );
    assert!(
        statement.sql.contains("COUNT(DISTINCT dashboards.id)"),
        "{}",
        statement.sql
    );
    assert!(!statement.sql.contains("owner_scope"), "{}", statement.sql);
}
