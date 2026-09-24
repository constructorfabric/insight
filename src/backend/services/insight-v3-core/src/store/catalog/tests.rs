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
    held_by(database, table, name, kind, "MergeTree")
}

fn held_by(database: &str, table: &str, name: &str, kind: &str, engine: &str) -> ColumnFixture {
    ColumnFixture {
        database: database.to_owned(),
        table: table.to_owned(),
        name: name.to_owned(),
        r#type: kind.to_owned(),
        engine: engine.to_owned(),
    }
}

fn catalog_over(rows: Vec<ColumnFixture>) -> (Mock, Catalog) {
    let mock = Mock::new();
    mock.add(handlers::provide(rows));
    let client =
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));

    (mock, Catalog::new(client, "insight", "insight_datasets"))
}

async fn listing(catalog: &Catalog) -> Vec<TableEntry> {
    catalog
        .tables()
        .await
        .unwrap_or_else(|error| panic!("the catalogue should list: {error}"))
}

fn names(tables: &[TableEntry]) -> Vec<String> {
    tables
        .iter()
        .map(|listed| format!("{}.{}", listed.database, listed.table))
        .collect()
}

/// Every match, so a test that is not about the cap need not think about it.
async fn described_by(catalog: &Catalog, names: &[&str]) -> Described {
    let asked: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();

    catalog
        .describe(&asked, usize::MAX)
        .await
        .unwrap_or_else(|error| panic!("describe answers: {error}"))
}

fn described(tables: &[TableSchema]) -> Vec<String> {
    tables.iter().map(TableSchema::qualified).collect()
}

/// The statement itself is the filter: the mock answers whatever it is asked,
/// so the text is what proves the engine's databases and the datasets
/// database are never offered.
#[test]
fn the_listing_leaves_out_the_engines_databases_and_the_datasets_database() {
    assert!(LIST_COLUMNS.contains("NOT IN ('system', 'information_schema', 'INFORMATION_SCHEMA')"));
    assert!(LIST_COLUMNS.contains("c.database != ?"));
}

/// A materialised view keeps its rows in a table of its own, which is not a
/// relation anyone should name.
#[test]
fn a_materialised_views_own_storage_is_never_listed() {
    assert!(LIST_COLUMNS.contains("NOT startsWith(c.table, '.inner')"));
}

#[test]
fn a_layer_is_read_off_the_database_name() {
    let cases = [
        ("bronze_github", Layer::Bronze),
        ("bronze_gitlab", Layer::Bronze),
        ("silver", Layer::Silver),
        ("identity", Layer::Identity),
        ("insight", Layer::Gold),
        ("default", Layer::Other),
        ("staging", Layer::Other),
    ];

    for (database, expected) in cases {
        assert_eq!(classify(database, "insight"), expected, "for {database}");
    }
}

#[test]
fn the_gold_layer_follows_the_configured_database() {
    assert_eq!(classify("insight", "insight"), Layer::Gold);
    assert_eq!(classify("insight", "gold"), Layer::Other);
    assert_eq!(classify("gold", "gold"), Layer::Gold);
}

#[tokio::test]
async fn each_table_becomes_one_schema_carrying_its_columns_in_position_order() {
    let (_mock, catalog) = catalog_over(vec![
        column("bronze_github", "issues", "number", "Int64"),
        column("bronze_github", "issues", "title", "String"),
        column("silver", "fct_commit", "sha", "String"),
    ]);

    let tables = listing(&catalog).await;
    let one = described_by(&catalog, &["bronze_github.issues"]).await;

    assert_eq!(
        names(&tables),
        vec!["bronze_github.issues", "silver.fct_commit"]
    );
    let Some(issues) = one.tables.first() else {
        panic!("the first table is described")
    };
    assert_eq!(issues.layer, Layer::Bronze);
    assert_eq!(
        issues.columns,
        vec![
            Column {
                name: "number".to_owned(),
                kind: "Int64".to_owned()
            },
            Column {
                name: "title".to_owned(),
                kind: "String".to_owned()
            },
        ]
    );
}

#[tokio::test]
async fn a_bare_table_is_described_in_every_database_that_has_one() {
    let (_mock, catalog) = catalog_over(vec![
        column("bronze_github", "issues", "number", "Int64"),
        column("bronze_gitlab", "issues", "iid", "Int64"),
        column("silver", "fct_commit", "sha", "String"),
    ]);

    let bare = described_by(&catalog, &["issues"]).await;
    let qualified = described_by(&catalog, &["bronze_gitlab.issues"]).await;

    assert_eq!(
        described(&bare.tables),
        vec!["bronze_github.issues", "bronze_gitlab.issues"]
    );
    assert_eq!(bare.total, 2);
    assert_eq!(described(&qualified.tables), vec!["bronze_gitlab.issues"]);
}

#[tokio::test]
async fn a_table_is_found_by_its_database_and_name_alone() {
    let (_mock, catalog) = catalog_over(vec![
        column("bronze_github", "issues", "number", "Int64"),
        column("bronze_gitlab", "issues", "iid", "Int64"),
    ]);

    let found = catalog
        .find("bronze_gitlab", "issues")
        .await
        .unwrap_or_else(|error| panic!("find answers: {error}"));
    let absent = catalog
        .find("bronze_gitlab", "commits")
        .await
        .unwrap_or_else(|error| panic!("find answers: {error}"));

    assert_eq!(
        found.map(|schema| schema.qualified()),
        Some("bronze_gitlab.issues".to_owned())
    );
    assert_eq!(absent, None);
}

/// The mock holds one answer. A second listing that asked again would find
/// nothing to answer with and fail, so a second success proves the cache.
#[tokio::test]
async fn a_second_listing_inside_the_ttl_asks_clickhouse_nothing() {
    let (_mock, catalog) = catalog_over(vec![column("silver", "fct_commit", "sha", "String")]);

    let first = listing(&catalog).await;
    let second = listing(&catalog).await;

    assert_eq!(first, second);
}

#[tokio::test]
async fn a_schema_carries_the_engine_as_the_warehouse_spells_it() {
    let (_mock, catalog) = catalog_over(vec![held_by(
        "silver",
        "class_usage",
        "day",
        "Date",
        "ReplicatedReplacingMergeTree",
    )]);

    let tables = described_by(&catalog, &["class_usage"]).await.tables;

    assert_eq!(
        tables.first().map(|schema| schema.engine.as_str()),
        Some("ReplicatedReplacingMergeTree")
    );
}

#[tokio::test]
async fn a_fixed_catalogue_answers_what_it_was_given_and_asks_nobody() {
    let catalog = Catalog::fixed(vec![TableSchema {
        database: "silver".to_owned(),
        table: "fct_commit".to_owned(),
        layer: Layer::Silver,
        engine: "MergeTree".to_owned(),
        columns: Vec::new(),
    }]);

    assert_eq!(names(&listing(&catalog).await), vec!["silver.fct_commit"]);
}

/// A bare name can mean a table in every database the warehouse holds, and
/// each carries every column it has, so the cap has to bite before the copy.
#[tokio::test]
async fn a_description_copies_no_more_tables_than_it_was_asked_to() {
    let (_mock, catalog) = catalog_over(
        (0..5)
            .map(|n| column(&format!("bronze_{n}"), "issues", "number", "Int64"))
            .collect(),
    );

    let described = catalog
        .describe(&["issues".to_owned()], 2)
        .await
        .unwrap_or_else(|error| panic!("describe answers: {error}"));

    assert_eq!(described.tables.len(), 2);
    assert_eq!(described.total, 5);
}

/// The cap decides how much is copied out, never whether the warehouse holds a
/// name. A caller told that an existing table is absent builds on the wrong one.
#[tokio::test]
async fn a_name_the_cap_left_out_is_not_reported_as_one_the_warehouse_lacks() {
    let mut rows: Vec<ColumnFixture> = (0..5)
        .map(|n| column(&format!("bronze_{n}"), "issues", "number", "Int64"))
        .collect();
    rows.push(column("silver", "fct_commit", "sha", "String"));

    let (_mock, catalog) = catalog_over(rows);

    let described = catalog
        .describe(&["issues".to_owned(), "silver.fct_commit".to_owned()], 2)
        .await
        .unwrap_or_else(|error| panic!("describe answers: {error}"));

    assert_eq!(described.tables.len(), 2);
    assert!(
        described.unknown.is_empty(),
        "nothing is unknown here, yet: {:?}",
        described.unknown
    );
}

#[tokio::test]
async fn a_name_no_table_answers_to_is_reported_as_unknown() {
    let (_mock, catalog) = catalog_over(vec![column("silver", "fct_commit", "sha", "String")]);

    let described = described_by(&catalog, &["silver.fct_commit", "nowhere.nothing"]).await;

    assert_eq!(described.unknown, vec!["nowhere.nothing"]);
}
