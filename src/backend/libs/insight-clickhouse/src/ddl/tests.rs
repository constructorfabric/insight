use super::*;

fn standalone(ddl: &Ddl) -> String {
    ddl.render(Topology::Standalone)
}

fn replicated(ddl: &Ddl) -> String {
    ddl.render(Topology::Replicated)
}

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
        standalone(&staging()),
        "CREATE TABLE `identity`.`identity_persons_staging_abc` (\n\
         \x20   id UInt64,\n\
         \x20   value Nullable(String)\n\
         ) ENGINE = MergeTree ORDER BY id"
    );
}

/// A `Replicated` database replays the DDL on every replica but replicates no
/// rows: only a `Replicated*` engine does, so the topology decides the family.
#[test]
fn the_engine_family_follows_the_topology() {
    assert!(
        replicated(&staging()).ends_with(") ENGINE = ReplicatedMergeTree ORDER BY id"),
        "{}",
        replicated(&staging())
    );
    assert!(
        replicated(&records()).contains(") ENGINE = ReplicatedMergeTree ORDER BY ("),
        "{}",
        replicated(&records())
    );
    assert!(
        standalone(&staging()).ends_with(") ENGINE = MergeTree ORDER BY id"),
        "{}",
        standalone(&staging())
    );
}

/// Argument-less: the server fills the Keeper path and replica name, and a
/// path spelled here would collide the moment two tables shared it.
#[test]
fn a_replicated_engine_names_no_keeper_path() {
    let sql = replicated(&staging());

    assert!(
        sql.contains("ENGINE = ReplicatedMergeTree ORDER BY"),
        "{sql}"
    );
    assert!(!sql.contains("ReplicatedMergeTree("), "{sql}");
}

/// Inside a `Replicated` database ClickHouse rejects `ON CLUSTER` on table
/// DDL, and a standalone node has no cluster to name.
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
        for topology in [Topology::Standalone, Topology::Replicated] {
            let sql = statement.render(topology);
            assert!(!sql.contains("ON CLUSTER"), "{sql}");
        }
    }
}

/// Only a create carries an engine, so only a create changes with the topology.
#[test]
fn a_drop_a_swap_and_a_rename_read_the_same_on_either_topology() {
    let statements = [
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
        assert_eq!(standalone(&statement), replicated(&statement));
    }
}

#[test]
fn a_drop_tolerates_a_name_nothing_holds() {
    assert_eq!(
        standalone(&Ddl::drop_table("insight_datasets", "ds_orders_abc")),
        "DROP TABLE IF EXISTS `insight_datasets`.`ds_orders_abc`"
    );
}

#[test]
fn a_swap_names_both_sides_in_the_one_database() {
    assert_eq!(
        standalone(&Ddl::exchange_tables(
            "identity",
            "identity_persons_staging_abc",
            "identity_persons"
        )),
        "EXCHANGE TABLES `identity`.`identity_persons_staging_abc` \
         AND `identity`.`identity_persons`"
    );
}

#[test]
fn a_rename_publishes_into_a_name_nothing_holds_yet() {
    assert_eq!(
        standalone(&Ddl::rename_table(
            "identity",
            "identity_persons_staging_abc",
            "identity_persons"
        )),
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
        standalone(&records()).starts_with("CREATE TABLE IF NOT EXISTS "),
        "{}",
        standalone(&records())
    );
}

#[test]
fn a_create_that_mints_its_own_name_fails_on_a_collision() {
    let sql = standalone(&staging());

    assert!(
        !sql.contains("IF NOT EXISTS"),
        "a name this run minted and something already holds is a bug, not a repeat: {sql}"
    );
}

/// A key ClickHouse reads back from `system.tables` carries no quotes, so a
/// reader recognising a table it made compares against the bare form.
#[test]
fn a_compound_sort_key_is_parenthesised_and_a_single_one_is_not() {
    assert!(standalone(&records()).ends_with("ORDER BY (table_name, received_at, id)"));
    assert!(standalone(&staging()).ends_with("ORDER BY id"));
}

#[test]
fn the_declared_column_order_is_the_rendered_order() {
    let sql = standalone(&records());

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
    .render(Topology::Standalone);

    assert!(sql.contains("`ds_quarterly-orders_abc`"), "{sql}");
}
