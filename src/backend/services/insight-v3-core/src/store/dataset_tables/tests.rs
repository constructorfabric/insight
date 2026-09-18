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
