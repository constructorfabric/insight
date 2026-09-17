use clickhouse::test::{Mock, handlers};

use super::*;

#[derive(clickhouse::Row, serde::Serialize)]
struct ColumnFixture {
    database: String,
    table: String,
    name: String,
    r#type: String,
    engine: String,
}

fn column(database: &str, table: &str, name: &str, kind: &str) -> ColumnFixture {
    ColumnFixture {
        database: database.to_owned(),
        table: table.to_owned(),
        name: name.to_owned(),
        r#type: kind.to_owned(),
        engine: "MergeTree".to_owned(),
    }
}

fn columns(names: &[&str]) -> Vec<(String, String)> {
    names
        .iter()
        .map(|name| ((*name).to_owned(), "String".to_owned()))
        .collect()
}

fn catalog_over(rows: Vec<ColumnFixture>) -> (Mock, Catalog) {
    let mock = Mock::new();
    mock.add(handlers::provide(rows));
    let client =
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));

    (mock, Catalog::new(client, "insight".to_owned()))
}

async fn listing(catalog: &Catalog) -> Vec<TableSchema> {
    catalog
        .tables()
        .await
        .unwrap_or_else(|error| panic!("the catalogue should list: {error}"))
}

#[test]
fn the_listing_leaves_out_the_engines_own_databases() {
    assert!(LIST_COLUMNS.contains("system.columns"));

    for database in [
        "'system'",
        "'information_schema'",
        "'INFORMATION_SCHEMA'",
        "'default'",
    ] {
        assert!(
            LIST_COLUMNS.contains(database),
            "should exclude {database}: {LIST_COLUMNS}"
        );
    }

    assert!(LIST_COLUMNS.contains("ORDER BY c.database, c.table, c.position"));
}

#[test]
fn a_layer_is_read_off_the_database_name() {
    let cases = [
        ("bronze_github", Layer::Bronze),
        ("bronze_whatever", Layer::Bronze),
        ("bronze", Layer::Other),
        ("silver", Layer::Silver),
        ("identity", Layer::Identity),
        ("insight", Layer::Gold),
        ("staging", Layer::Other),
    ];

    for (database, expected) in cases {
        assert_eq!(
            classify(database, "insight", &columns(&["day"])),
            expected,
            "should classify {database}"
        );
    }
}

#[test]
fn the_gold_layer_follows_the_configured_database() {
    let day = columns(&["day"]);

    assert_eq!(classify("warehouse", "warehouse", &day), Layer::Gold);
    assert_eq!(classify("insight", "warehouse", &day), Layer::Other);
}

#[test]
fn a_table_carrying_the_ingest_schema_is_not_its_databases_layer() {
    let ingest = columns(&["id", "table_name", "raw_data", "received_at"]);
    let partial = columns(&["id", "table_name", "raw_data"]);

    assert_eq!(classify("insight", "insight", &ingest), Layer::Ingest);
    assert_eq!(classify("insight", "insight", &partial), Layer::Gold);
}

#[tokio::test]
async fn each_table_becomes_one_schema_carrying_its_columns_in_position_order() {
    let (_mock, catalog) = catalog_over(vec![
        column("silver", "git_commits", "sha", "String"),
        column("silver", "git_commits", "lines_changed", "UInt32"),
        column("silver", "git_reviews", "reviewer", "String"),
    ]);

    let tables = listing(&catalog).await;

    assert_eq!(
        tables,
        vec![
            TableSchema {
                database: "silver".to_owned(),
                table: "git_commits".to_owned(),
                layer: Layer::Silver,
                engine: TableEngine::MergeTree,
                columns: vec![
                    ("sha".to_owned(), "String".to_owned()),
                    ("lines_changed".to_owned(), "UInt32".to_owned()),
                ],
            },
            TableSchema {
                database: "silver".to_owned(),
                table: "git_reviews".to_owned(),
                layer: Layer::Silver,
                engine: TableEngine::MergeTree,
                columns: vec![("reviewer".to_owned(), "String".to_owned())],
            },
        ]
    );
}

#[tokio::test]
async fn a_second_listing_inside_the_ttl_asks_clickhouse_nothing() {
    // The mock answers one request, so a second query fails the call below.
    let (_mock, catalog) = catalog_over(vec![column("silver", "git_commits", "sha", "String")]);

    let first = listing(&catalog).await;
    let second = catalog
        .tables()
        .await
        .unwrap_or_else(|error| panic!("the cached listing should not query again: {error}"));

    assert_eq!(first, second);
}

#[tokio::test]
async fn a_schema_carries_the_engine_loaded_with_its_columns() {
    let mut replacing = column("silver", "events", "id", "UInt64");
    replacing.engine = "ReplicatedReplacingMergeTree".to_owned();
    let (_mock, catalog) = catalog_over(vec![replacing]);

    let tables = listing(&catalog).await;

    assert_eq!(tables[0].engine, TableEngine::ReplacingMergeTree);
    assert!(tables[0].engine.requires_final());
    assert!(!TableEngine::MergeTree.requires_final());
}
