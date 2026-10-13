use super::*;

fn staging() -> Ddl {
    Ddl::create_table(TableSpec::merge_tree(
        "identity",
        "identity_persons_staging_abc",
        &[("id", "UInt64"), ("value", "Nullable(String)")],
        &["id"],
    ))
}

fn records() -> Ddl {
    Ddl::create_table(
        TableSpec::merge_tree(
            "insight_datasets",
            "ds_orders_abc",
            &[
                ("id", "UUID"),
                ("table_name", "String"),
                ("received_at", "DateTime64(3, 'UTC')"),
            ],
            &["table_name", "received_at", "id"],
        )
        .keep_existing(),
    )
}

#[test]
fn a_create_names_its_columns_its_engine_and_its_key() {
    assert_eq!(
        staging().render(),
        "CREATE TABLE `identity`.`identity_persons_staging_abc` (\n\
         \x20   id UInt64,\n\
         \x20   value Nullable(String)\n\
         ) ENGINE = MergeTree ORDER BY id"
    );
}

/// The `Replicated` database engine logs every DDL inside it in Keeper and
/// converts `MergeTree` on the way in, so a clause or a prefix here would be a
/// second answer to a question the database has already answered.
#[test]
fn no_statement_qualifies_itself_with_a_cluster() {
    let statements = [
        staging(),
        records(),
        Ddl::drop_table("identity", "identity_persons_staging_abc"),
        Ddl::exchange_tables(
            "identity",
            "identity_persons_staging_abc",
            "identity_persons",
        ),
        Ddl::rename_table(
            "identity",
            "identity_persons_staging_abc",
            "identity_persons",
        ),
    ];

    for statement in statements {
        let sql = statement.render();
        assert!(!sql.contains("ON CLUSTER"), "{sql}");
        assert!(!sql.contains("Replicated"), "{sql}");
    }
}

#[test]
fn a_drop_tolerates_a_name_nothing_holds() {
    assert_eq!(
        Ddl::drop_table("insight_datasets", "ds_orders_abc").render(),
        "DROP TABLE IF EXISTS `insight_datasets`.`ds_orders_abc`"
    );
}

#[test]
fn a_swap_names_both_sides_in_the_one_database() {
    assert_eq!(
        Ddl::exchange_tables(
            "identity",
            "identity_persons_staging_abc",
            "identity_persons"
        )
        .render(),
        "EXCHANGE TABLES `identity`.`identity_persons_staging_abc` \
         AND `identity`.`identity_persons`"
    );
}

#[test]
fn a_rename_publishes_into_a_name_nothing_holds_yet() {
    assert_eq!(
        Ddl::rename_table(
            "identity",
            "identity_persons_staging_abc",
            "identity_persons"
        )
        .render(),
        "RENAME TABLE `identity`.`identity_persons_staging_abc` TO `identity`.`identity_persons`"
    );
}

#[test]
fn every_statement_acts_in_the_database_it_names() {
    let statements = [
        Ddl::drop_table("silver", "t"),
        Ddl::exchange_tables("silver", "a", "b"),
        Ddl::rename_table("silver", "a", "b"),
    ];

    for statement in statements {
        assert_eq!(statement.database(), "silver", "{statement:?}");
    }
}

#[test]
fn a_repeated_create_leaves_a_table_already_there() {
    assert!(
        records()
            .render()
            .starts_with("CREATE TABLE IF NOT EXISTS "),
        "{}",
        records().render()
    );
}

#[test]
fn a_create_that_mints_its_own_name_fails_on_a_collision() {
    let sql = staging().render();

    assert!(
        !sql.contains("IF NOT EXISTS"),
        "a name this run minted and something already holds is a bug, not a repeat: {sql}"
    );
}

/// A key ClickHouse reads back from `system.tables` carries no quotes, so a
/// reader recognising a table it made compares against the bare form.
#[test]
fn a_compound_sort_key_is_parenthesised_and_a_single_one_is_not() {
    assert!(
        records()
            .render()
            .ends_with("ORDER BY (table_name, received_at, id)")
    );
    assert!(staging().render().ends_with("ORDER BY id"));
}

#[test]
fn the_declared_column_order_is_the_rendered_order() {
    let sql = records().render();

    let positions: Vec<usize> = ["id UUID", "table_name String", "received_at "]
        .iter()
        .filter_map(|column| sql.find(column))
        .collect();
    assert_eq!(positions.len(), 3, "every column must be rendered: {sql}");
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "the declared order is the table's physical column order: {sql}"
    );
}

#[test]
fn a_dataset_name_carrying_a_dash_stays_one_name() {
    let sql = Ddl::create_table(TableSpec::merge_tree(
        "insight_datasets",
        "ds_quarterly-orders_abc",
        &[("id", "UUID")],
        &["id"],
    ))
    .render();

    assert!(sql.contains("`ds_quarterly-orders_abc`"), "{sql}");
}
