use clickhouse::test::{Mock, handlers};

use super::*;

#[test]
fn physical_table_names_use_portable_identifiers() {
    assert!(TableName::parse("events_2026").is_ok());

    for value in ["", " events", "events.daily", "9events", "naïve"] {
        assert!(
            TableName::parse(value).is_err(),
            "should reject table name: {value:?}"
        );
    }
}

#[tokio::test]
async fn table_creation_uses_the_fixed_raw_data_schema() {
    let mock = Mock::new();
    let recording = mock.add(handlers::record_ddl());
    let client =
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));
    let store = TableStore::new(client);
    let table = TableName::parse("events_2026").unwrap_or_else(|error| panic!("name: {error}"));

    store
        .create(&table)
        .await
        .unwrap_or_else(|error| panic!("table should be created: {error}"));
    let query = recording.query().await;

    assert!(query.contains("CREATE TABLE IF NOT EXISTS `events_2026`"));
    assert!(query.contains("id UUID"));
    assert!(query.contains("table_name String"));
    assert!(query.contains("raw_data String"));
    assert!(query.contains("received_at DateTime64(3, 'UTC')"));
    assert!(query.contains("ORDER BY (table_name, received_at, id)"));
}

#[tokio::test]
async fn a_hung_listing_is_time_bounded() {
    let (address, server) = hanging_warehouse().await;
    let store = TableStore::with_read_timeout(
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            format!("http://{address}"),
            "insight",
        )),
        Duration::from_millis(25),
    );

    let result = tokio::time::timeout(Duration::from_secs(1), store.list()).await;
    server.abort();

    assert!(matches!(result, Ok(Err(TableStoreError::Timeout))));
}

#[tokio::test]
async fn a_hung_field_sample_is_time_bounded() {
    let (address, server) = hanging_warehouse().await;
    let store = TableStore::with_read_timeout(
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            format!("http://{address}"),
            "insight",
        )),
        Duration::from_millis(25),
    );
    let table = TableName::parse("events_2026").unwrap_or_else(|error| panic!("name: {error}"));

    let result = tokio::time::timeout(Duration::from_secs(1), store.sample_fields(&table)).await;
    server.abort();

    assert!(matches!(result, Ok(Err(TableStoreError::Timeout))));
}

#[test]
fn no_failure_of_a_read_claims_a_creation_failed() {
    assert!(!TableStoreError::Timeout.to_string().contains("creation"));
    assert!(
        !TableStoreError::ClickHouse(clickhouse::error::Error::Custom("x".to_owned()))
            .to_string()
            .contains("creation")
    );
}

/// Accepts the connection and then answers nothing at all.
async fn hanging_warehouse() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("test listener must bind: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("test listener must have an address: {error}"));
    let server = tokio::spawn(async move {
        let _connection = listener
            .accept()
            .await
            .unwrap_or_else(|error| panic!("test server must accept: {error}"));
        futures::future::pending::<()>().await;
    });

    (address, server)
}

#[test]
fn listing_selects_only_tables_shaped_like_an_ingest_table() {
    // The database holds tables from every other service too, so the
    // ingest schema's own ORDER BY is what separates ours from theirs.
    assert!(LIST_TABLES.contains("system.tables"));
    assert!(LIST_TABLES.contains("database = currentDatabase()"));
    assert!(LIST_TABLES.contains("table_name, received_at, id"));
}

#[tokio::test]
async fn listing_returns_the_table_names() {
    let mock = Mock::new();
    mock.add(handlers::provide(vec![
        "events".to_owned(),
        "raw_data".to_owned(),
    ]));
    let client =
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));
    let store = TableStore::new(client);

    let names = store
        .list()
        .await
        .unwrap_or_else(|error| panic!("tables should list: {error}"));

    assert_eq!(names, vec!["events".to_owned(), "raw_data".to_owned()]);
}

#[tokio::test]
async fn overlapping_rows_yield_one_entry_per_field_typed_from_its_value() {
    let mock = Mock::new();
    mock.add(handlers::provide(vec![
        r#"{"day":"2026-09-01","lines":59}"#.to_owned(),
        r#"{"day":"2026-09-02","author":"nda"}"#.to_owned(),
    ]));
    let client =
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));
    let store = TableStore::new(client);
    let table = TableName::parse("events_2026").unwrap_or_else(|error| panic!("name: {error}"));

    let fields = store
        .sample_fields(&table)
        .await
        .unwrap_or_else(|error| panic!("fields should be sampled: {error}"));

    assert_eq!(
        fields,
        vec![
            ("day".to_owned(), "string"),
            ("lines".to_owned(), "int"),
            ("author".to_owned(), "string"),
        ]
    );
}
