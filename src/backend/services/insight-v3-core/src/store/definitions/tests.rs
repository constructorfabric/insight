use serde_json::json;

use super::*;

fn bound(statement: &Statement) -> Vec<String> {
    statement
        .values
        .as_ref()
        .map(|values| values.0.iter().map(ToString::to_string).collect())
        .unwrap_or_default()
}

fn an_id() -> DefinitionId {
    DefinitionId::parse("0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b")
        .unwrap_or_else(|error| panic!("the id must parse: {error}"))
}

fn a_label() -> DefinitionLabel {
    DefinitionLabel::parse("Commits per day")
        .unwrap_or_else(|error| panic!("the label must parse: {error}"))
}

#[test]
fn a_create_inserts_and_never_upserts() {
    let statement = sql(INSERT_NEW, DefinitionKind::Dashboard);

    assert!(
        statement.starts_with("INSERT INTO dashboards (id, name, body, updated_at)"),
        "{statement}"
    );
    assert!(!statement.contains("ON DUPLICATE KEY"), "{statement}");
}

#[test]
fn a_replace_updates_by_id_and_never_inserts() {
    let statement = sql(REPLACE_ONE, DefinitionKind::Widget);

    assert!(
        statement.starts_with("UPDATE widgets SET name = ?"),
        "{statement}"
    );
    assert!(statement.ends_with("WHERE id = ?"), "{statement}");
}

#[test]
fn names_are_read_in_one_statement_binding_every_id() {
    let statement = names_statement(LibraryKind::Metric, &["first", "second"]);

    assert_eq!(
        statement.sql,
        "SELECT id, name FROM metrics WHERE id IN (?, ?)"
    );
    assert_eq!(bound(&statement), ["'first'", "'second'"]);
}

#[test]
fn a_catalogue_page_reads_each_body_and_its_last_change() {
    let page = Page::parse(Some(2), Some(4)).unwrap_or_else(|error| panic!("{error}"));
    let statement = page_statement(PAGE_ENTRIES, LibraryKind::Widget, "%day%", page);

    assert!(
        statement
            .sql
            .starts_with("SELECT id, name, body, updated_at FROM widgets"),
        "{}",
        statement.sql
    );
    assert_eq!(bound(&statement), ["'%day%'", "'%day%'", "2", "4"]);
}

#[test]
fn a_delete_is_by_id_in_the_kind_table() {
    assert_eq!(
        sql(DELETE_ONE, DefinitionKind::Dashboard),
        "DELETE FROM dashboards WHERE id = ?"
    );
}

#[test]
fn reads_are_by_id_and_never_interpolate_it() {
    let one = sql(SELECT_BODY, DefinitionKind::Metric);
    let all = sql(SELECT_IDS, DefinitionKind::Widget);

    assert_eq!(one, "SELECT body FROM metrics WHERE id = ?");
    assert_eq!(all, "SELECT id FROM widgets ORDER BY id");
}

#[test]
fn a_page_searches_name_and_body_and_orders_by_name_then_id() {
    let statement = sql(PAGE_LISTED, DefinitionKind::Dashboard);

    assert!(
        statement.contains("SELECT id, name FROM dashboards"),
        "{statement}"
    );
    assert!(
        statement.contains("name LIKE ? OR body LIKE ?"),
        "{statement}"
    );
    assert!(statement.contains("ORDER BY name, id"), "{statement}");
}

#[test]
fn a_create_binds_the_id_the_label_and_the_body() {
    let statement = MariaDefinitions::insert_statement(
        DefinitionKind::Metric,
        &an_id(),
        &a_label(),
        &json!({ "table": "events" }),
    )
    .unwrap_or_else(|error| panic!("the statement builds: {error}"));

    let values = bound(&statement);
    assert_eq!(values.len(), 3);
    assert_eq!(values[0], "'0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b'");
    assert_eq!(values[1], "'Commits per day'");
    assert!(values[2].contains("events"), "{:?}", values[2]);
}

#[test]
fn a_replace_binds_the_label_and_the_body_then_the_id() {
    let statement = MariaDefinitions::replace_statement(
        DefinitionKind::Metric,
        &an_id(),
        &a_label(),
        &json!({ "table": "events" }),
    )
    .unwrap_or_else(|error| panic!("the statement builds: {error}"));

    let values = bound(&statement);
    assert_eq!(values.len(), 3);
    assert_eq!(values[0], "'Commits per day'");
    assert!(values[1].contains("events"), "{:?}", values[1]);
    assert_eq!(values[2], "'0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b'");
}
