use clickhouse::test::{Mock, handlers};

use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn tables(mock: &Mock) -> DatasetTables {
    DatasetTables::new(insight_clickhouse::Client::new(
        insight_clickhouse::Config::new(mock.url(), "insight_datasets"),
    ))
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

/// The `Replicated` database engine replays the statement on every replica and
/// converts the engine on the way in, so a clustered install sends exactly what
/// a single-node one sends.
#[test]
fn a_clustered_install_provisions_the_same_statement() -> R {
    let clustered = tables_on(
        "insight_datasets",
        insight_clickhouse::Topology::new(true, "insight_cluster"),
    );
    let standalone = tables_on("insight_datasets", insight_clickhouse::Topology::Standalone);

    assert_eq!(
        provision_sql(&clustered, "ds_commits_1")?,
        provision_sql(&standalone, "ds_commits_1")?
    );

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
