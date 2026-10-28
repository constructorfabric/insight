use clickhouse::test::{Mock, handlers};

use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn tables(mock: &Mock) -> DatasetTables {
    DatasetTables::new(insight_clickhouse::Client::new(
        insight_clickhouse::Config::new(mock.url(), "insight_datasets"),
    ))
}

/// What `system.databases` answers a provision's first question with: a plain
/// database, which is what a standalone topology expects.
fn plain_database(mock: &Mock) {
    mock.add(handlers::provide(vec![String::from("Atomic")]));
}

#[tokio::test]
async fn a_name_nothing_holds_is_free_to_provision() -> R {
    let mock = Mock::new();
    mock.add(handlers::provide(Vec::<ShapeRow>::new()));

    assert_eq!(tables(&mock).shape_of("ds_commits_1").await?, Shape::Absent);

    Ok(())
}

#[tokio::test]
async fn a_table_of_the_ingest_shape_is_one_this_service_could_have_made() -> R {
    let mock = Mock::new();
    mock.add(handlers::provide(vec![ShapeRow {
        sorting_key: INGEST_SORTING_KEY.to_owned(),
    }]));

    assert_eq!(tables(&mock).shape_of("ds_commits_1").await?, Shape::Ingest);

    Ok(())
}

#[tokio::test]
async fn a_table_of_any_other_shape_belongs_to_something_else() -> R {
    let mock = Mock::new();
    mock.add(handlers::provide(vec![ShapeRow {
        sorting_key: "event_date, project_id".to_owned(),
    }]));

    assert_eq!(
        tables(&mock).shape_of("ds_commits_1").await?,
        Shape::Foreign
    );

    Ok(())
}

#[tokio::test]
async fn a_name_held_by_a_foreign_table_is_refused_rather_than_created_over() {
    let mock = Mock::new();
    plain_database(&mock);
    mock.add(handlers::provide(vec![ShapeRow {
        sorting_key: "event_date, project_id".to_owned(),
    }]));

    let refused = tables(&mock).provision("ds_commits_1").await;

    assert!(
        matches!(&refused, Err(DatasetTableError::NotOurs(named)) if named == "ds_commits_1"),
        "{refused:?}"
    );
}

#[tokio::test]
async fn provisioning_makes_a_table_of_the_ingest_shape() -> R {
    let mock = Mock::new();
    plain_database(&mock);
    mock.add(handlers::provide(Vec::<ShapeRow>::new()));
    let recording = mock.add(handlers::record_ddl());

    tables(&mock).provision("ds_commits_1").await?;

    let ddl = recording.query().await;
    assert!(ddl.contains("CREATE TABLE IF NOT EXISTS"), "{ddl}");
    assert!(ddl.contains("ds_commits_1"), "{ddl}");
    assert!(
        ddl.contains("ORDER BY (table_name, received_at, id)"),
        "{ddl}"
    );

    Ok(())
}

#[tokio::test]
async fn a_table_already_of_the_right_shape_is_this_attempt_repeating_itself() -> R {
    let mock = Mock::new();
    plain_database(&mock);
    mock.add(handlers::provide(vec![ShapeRow {
        sorting_key: INGEST_SORTING_KEY.to_owned(),
    }]));
    let recording = mock.add(handlers::record_ddl());

    tables(&mock).provision("ds_commits_1").await?;

    assert!(
        recording.query().await.contains("IF NOT EXISTS"),
        "an existing table of the right shape must not be created twice"
    );

    Ok(())
}

/// The database is create-databases.sh's to make, so a provision against a
/// missing one, or one of the wrong engine, stops before any shape is read.
#[tokio::test]
async fn a_provision_refuses_a_datasets_database_of_the_wrong_engine() {
    let mock = Mock::new();
    mock.add(handlers::provide(vec![String::from("Replicated")]));

    let refused = tables(&mock).provision("ds_commits_1").await;

    assert!(
        matches!(&refused, Err(DatasetTableError::DatabaseTopology(remedy)) if remedy.contains("clickhouse_cluster_mode is off")),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_provision_refuses_a_datasets_database_nothing_created() {
    let mock = Mock::new();
    mock.add(handlers::provide(Vec::<String>::new()));

    let refused = tables(&mock).provision("ds_commits_1").await;

    assert!(
        matches!(&refused, Err(DatasetTableError::DatabaseTopology(remedy)) if remedy.contains("create-databases.sh")),
        "{refused:?}"
    );
}

#[tokio::test]
async fn dropping_names_the_table_and_tolerates_its_absence() -> R {
    let mock = Mock::new();
    let recording = mock.add(handlers::record_ddl());

    tables(&mock).drop_table("ds_commits_1").await?;

    let ddl = recording.query().await;
    assert!(ddl.contains("DROP TABLE IF EXISTS"), "{ddl}");
    assert!(ddl.contains("ds_commits_1"), "{ddl}");

    Ok(())
}

/// The warehouse's own "stopped waiting" is a timeout here too: the statement
/// is still queued, so the caller must treat it as one whose fate is unknown.
#[tokio::test]
async fn a_statement_the_warehouse_stopped_waiting_on_is_a_timeout() {
    let mock = Mock::new();
    mock.add(handlers::exception(159));

    let unfinished = tables(&mock).drop_table("ds_commits_1").await;

    assert!(
        matches!(unfinished, Err(DatasetTableError::Timeout)),
        "{unfinished:?}"
    );
}

/// A warehouse that takes every connection and never says a word back.
async fn silent_warehouse() -> Result<String, std::io::Error> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((connection, _)) = listener.accept().await {
            held.push(connection);
        }
    });

    Ok(format!("http://{address}"))
}

/// A warehouse that never answers is cut off by this service's own bound,
/// and reported the same way: the statement may still land.
#[tokio::test]
async fn a_statement_the_warehouse_never_answers_ends_at_this_services_bound() -> R {
    let tables = DatasetTables::new(insight_clickhouse::Client::new(
        insight_clickhouse::Config::new(silent_warehouse().await?, "insight_datasets"),
    ))
    .with_ddl_timeout(Duration::from_millis(10));

    let cut_off = tables.drop_table("ds_commits_1").await;

    assert!(
        matches!(cut_off, Err(DatasetTableError::Timeout)),
        "{cut_off:?}"
    );

    Ok(())
}

/// A client that dials nothing: these pin the statement `provision` would
/// send, which is decided before any of it reaches a server.
fn tables_on(database: &str, topology: insight_clickhouse::Topology) -> DatasetTables {
    DatasetTables::new(insight_clickhouse::Client::new(
        insight_clickhouse::Config::new("http://ch:8123", database).with_topology(topology),
    ))
}

fn provision_sql(tables: &DatasetTables, table: &str) -> Result<String, DatasetTableError> {
    Ok(tables.client.ddl_sql(&tables.ingest_spec(table))?)
}

/// A table is recognised as this service's by the key `system.tables` reads
/// back, so the key the create writes has to render into exactly that text.
#[test]
fn the_sort_key_the_create_writes_is_the_one_a_reader_recognises() -> R {
    let tables = tables_on("insight_datasets", insight_clickhouse::Topology::Standalone);

    let sql = provision_sql(&tables, "ds_commits_1")?;

    assert!(
        sql.ends_with(&format!("ORDER BY ({INGEST_SORTING_KEY})")),
        "a key no reader matches would make every table of ours look foreign: {sql}"
    );

    Ok(())
}

/// A `Replicated` database replays the create on every replica but the rows
/// follow the engine, so a clustered install provisions a `ReplicatedMergeTree`.
#[test]
fn a_clustered_install_provisions_a_replicated_table() -> R {
    let clustered = tables_on(
        "insight_datasets",
        insight_clickhouse::Topology::new(true, "insight_cluster"),
    );
    let standalone = tables_on("insight_datasets", insight_clickhouse::Topology::Standalone);

    let clustered = provision_sql(&clustered, "ds_commits_1")?;
    let standalone = provision_sql(&standalone, "ds_commits_1")?;
    assert!(
        clustered.contains(") ENGINE = ReplicatedMergeTree ORDER BY ("),
        "{clustered}"
    );
    assert!(
        standalone.contains(") ENGINE = MergeTree ORDER BY ("),
        "{standalone}"
    );
    assert!(!clustered.contains("ON CLUSTER"), "{clustered}");

    Ok(())
}

#[test]
fn the_create_names_the_datasets_database_and_no_other() {
    let spec = tables_on("insight_datasets", insight_clickhouse::Topology::Standalone)
        .ingest_spec("ds_commits_1");
    let elsewhere = tables_on("silver", insight_clickhouse::Topology::Standalone);

    assert!(
        elsewhere.client.ddl_sql(&spec).is_err(),
        "a dataset's table belongs in the datasets database and nowhere else"
    );
}

/// A warehouse that records what it was asked, on a clustered install.
async fn clustered_capture()
-> Result<(insight_clickhouse::test_support::Capture, DatasetTables), std::io::Error> {
    let capture = insight_clickhouse::test_support::Capture::start().await?;
    let tables = DatasetTables::new(insight_clickhouse::Client::new(
        insight_clickhouse::Config::new(capture.url(), "insight_datasets")
            .with_topology(insight_clickhouse::Topology::new(true, "insight_cluster")),
    ));
    Ok((capture, tables))
}

const QUORUM_PAIR: [&str; 3] = [
    "insert_quorum",
    "insert_quorum_parallel",
    "select_sequential_consistency",
];

/// A record is never read back by the request that stored it, and the
/// preview follows no write of its own, so neither takes the quorum pair: a
/// sequential quorum would refuse the next client's insert into the same
/// table, and a quorum-pinned read would throw on a lagging replica.
#[tokio::test]
async fn record_inserts_and_preview_reads_carry_no_quorum_pair() -> R {
    let (capture, tables) = clustered_capture().await?;
    let page = Page {
        limit: 10,
        offset: 0,
        order: "received_at DESC",
    };

    tables.insert("ds_commits_1", "commits", "{}").await?;
    tables.page("ds_commits_1", page).await?;
    tables.count("ds_commits_1").await?;

    let received = capture.received();
    assert_eq!(received.len(), 3, "{received:?}");
    for request in &received {
        for setting in QUORUM_PAIR {
            assert!(
                request.pair(setting).is_none(),
                "{setting} on a call that never reads its own write: {request:?}"
            );
        }
    }

    Ok(())
}

/// A create or drop waits for the active replicas, inside this store's own
/// bound: the warehouse reports a statement that did not finish before the
/// store gives up on it, so a create that landed is not discarded.
#[tokio::test]
async fn a_statement_waits_for_the_active_replicas_within_the_bound() -> R {
    let (capture, tables) = clustered_capture().await?;

    tables.drop_table("ds_commits_1").await?;

    let [dropped] = &capture.received()[..] else {
        panic!("one statement is sent");
    };
    assert_eq!(
        dropped.pair("distributed_ddl_output_mode"),
        Some("throw_only_active"),
        "{dropped:?}"
    );
    assert_eq!(
        dropped.pair("distributed_ddl_task_timeout"),
        Some(
            insight_clickhouse::DDL_TASK_TIMEOUT
                .as_secs()
                .to_string()
                .as_str()
        ),
        "{dropped:?}"
    );
    assert!(
        tables.ddl_timeout > insight_clickhouse::DDL_TASK_TIMEOUT,
        "the store outlives the warehouse's wait: {:?} <= {:?}",
        tables.ddl_timeout,
        insight_clickhouse::DDL_TASK_TIMEOUT
    );

    Ok(())
}
