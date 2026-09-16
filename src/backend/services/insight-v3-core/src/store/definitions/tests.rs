use serde_json::json;

use super::*;

#[test]
fn a_write_upserts_on_the_name_so_a_change_is_one_statement() {
    let statement = sql(UPSERT, DefinitionKind::Dashboard);

    assert!(statement.contains("INSERT INTO dashboards"), "{statement}");
    assert!(
        statement.contains("ON DUPLICATE KEY UPDATE body = VALUES(body)"),
        "{statement}"
    );
}

#[test]
fn a_delete_is_by_name_in_the_kind_table() {
    assert_eq!(
        sql(DELETE_ONE, DefinitionKind::Dashboard),
        "DELETE FROM dashboards WHERE name = ?"
    );
}

#[test]
fn reads_are_by_name_and_never_interpolate_it() {
    let one = sql(SELECT_BODY, DefinitionKind::Metric);
    let all = sql(SELECT_NAMES, DefinitionKind::Widget);

    assert_eq!(one, "SELECT body FROM metrics WHERE name = ?");
    assert_eq!(all, "SELECT name FROM widgets ORDER BY name");
}

#[test]
fn the_bound_values_are_the_name_and_the_body() {
    let name = DefinitionName::parse("commits_per_day")
        .unwrap_or_else(|error| panic!("name must parse: {error}"));

    let statement =
        MariaDefinitions::upsert(DefinitionKind::Metric, &name, &json!({ "table": "events" }))
            .unwrap_or_else(|error| panic!("the statement builds: {error}"));

    let values = statement
        .values
        .as_ref()
        .map(|values| values.0.clone())
        .unwrap_or_default();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].to_string(), "'commits_per_day'");
    assert!(values[1].to_string().contains("events"), "{:?}", values[1]);
}
