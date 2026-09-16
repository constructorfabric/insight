use clickhouse::test::{Mock, handlers};
use serde_json::json;
use tokio::net::TcpListener;

use super::*;

#[test]
fn physical_table_name_is_preserved() {
    let record = RawDataRecord::parse("synthetic_events", &json!({"value": 1}))
        .unwrap_or_else(|error| panic!("record must parse: {error}"));

    assert_eq!(record.table_name(), "synthetic_events");
}

#[test]
fn arbitrary_json_values_are_accepted() {
    for value in [
        serde_json::Value::Null,
        json!(true),
        json!(42),
        json!("text"),
        json!([1, {"nested": false}]),
        json!({"nested": [1, 2, 3]}),
    ] {
        assert!(
            RawDataRecord::parse("synthetic_events", &value).is_ok(),
            "every JSON shape must be accepted"
        );
    }
}

#[test]
fn unsafe_table_names_are_rejected() {
    for table in [
        "",
        "   ",
        "synthetic.events",
        &"x".repeat(crate::store::tables::MAX_TABLE_NAME_CHARS + 1),
    ] {
        assert!(
            RawDataRecord::parse(table, &json!(null)).is_err(),
            "must reject physical table name: {table:?}"
        );
    }
}

#[test]
fn clickhouse_timeout_is_normalized_to_store_timeout() {
    let error = StoreError::from(clickhouse::error::Error::TimedOut);

    assert!(matches!(error, StoreError::Timeout));
}

#[tokio::test]
async fn insert_writes_the_selected_table_row() {
    let mock = Mock::new();
    let recording = mock.add(handlers::record::<RawDataRow>());
    let client =
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));
    let store = RawDataStore::new(client);
    let record = RawDataRecord::parse("synthetic_events", &json!({"nested": [1, true, null]}))
        .unwrap_or_else(|error| panic!("record must parse: {error}"));

    store
        .insert(record)
        .await
        .unwrap_or_else(|error| panic!("insert must succeed: {error}"));
    let rows: Vec<RawDataRow> = recording.collect().await;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].table_name, "synthetic_events");
    assert_eq!(rows[0].raw_data, r#"{"nested":[1,true,null]}"#);
    assert_ne!(rows[0].id, uuid::Uuid::nil());
}

#[tokio::test]
async fn a_streams_first_write_creates_the_table_it_lands_in() {
    let mock = Mock::new();
    mock.add(handlers::exception(60));
    let ddl = mock.add(handlers::record_ddl());
    mock.add(handlers::record::<RawDataRow>());
    let store = RawDataStore::new(insight_clickhouse::Client::new(
        insight_clickhouse::Config::new(mock.url(), "insight"),
    ));
    let record = RawDataRecord::parse("first_write_stream", &json!({"a": 1}))
        .unwrap_or_else(|error| panic!("record must parse: {error}"));

    store
        .insert(record)
        .await
        .unwrap_or_else(|error| panic!("the write should land after the table exists: {error}"));
    let statement = ddl.query().await;

    assert!(
        statement.contains("CREATE TABLE IF NOT EXISTS `first_write_stream`"),
        "the stream's own table was not created: {statement}"
    );
}

#[test]
fn a_write_to_a_stream_with_no_table_is_the_callers_to_fix() {
    let refusal = StoreError::from(clickhouse::error::Error::BadResponse(
        "Code: 60. DB::Exception: Table insight.absent_stream does not exist. (UNKNOWN_TABLE)"
            .to_owned(),
    ));

    assert!(
        matches!(refusal, StoreError::NoTable),
        "a missing table answered 500 and leaked the database's own text: {refusal:?}"
    );
}

#[test]
fn any_other_bad_response_stays_a_server_error() {
    let refusal = StoreError::from(clickhouse::error::Error::BadResponse(
        "Code: 241. DB::Exception: Memory limit exceeded".to_owned(),
    ));

    assert!(matches!(refusal, StoreError::ClickHouse(_)));
}

#[tokio::test]
async fn hung_clickhouse_insert_is_time_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0")
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
    let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        format!("http://{address}"),
        "insight",
    ));
    let timeout = Duration::from_millis(25);
    let store = RawDataStore::with_timeouts(
        client,
        InsertTimeouts {
            send: timeout,
            end: timeout,
            total: timeout,
        },
    );
    let record = RawDataRecord::parse("synthetic_events", &json!(1))
        .unwrap_or_else(|error| panic!("record must parse: {error}"));

    let result = tokio::time::timeout(Duration::from_secs(1), store.insert(record)).await;
    server.abort();

    assert!(matches!(result, Ok(Err(StoreError::Timeout))));
}
