use chrono::{DateTime, TimeZone as _, Utc};
use clickhouse::test::{Mock, handlers};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::domain::dataset_lifecycle::DatasetLifecycle;
use crate::store::datasets::memory::MemoryDatasets;
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, Serialize, clickhouse::Row)]
struct NoTable {
    sorting_key: String,
}

/// One record as it reached the dataset's table.
#[derive(Debug, Deserialize, clickhouse::Row)]
struct LandedRecord {
    #[serde(with = "clickhouse::serde::uuid")]
    id: Uuid,
    table_name: String,
    raw_data: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    received_at: DateTime<Utc>,
}

fn name(value: &str) -> DefinitionName {
    DefinitionName::parse(value).unwrap_or_else(|error| panic!("the name parses: {error}"))
}

fn declaration() -> Value {
    json!({
        "title": "Commits",
        "fields": [{ "name": "day", "path": "day", "type": "datetime", "default_clock": true }]
    })
}

struct Fixture {
    mock: Mock,
    datasets: MemoryDatasets,
    tables: DatasetTables,
    definitions: MemoryDefinitions,
}

impl Fixture {
    fn new() -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let tables = DatasetTables::new(insight_clickhouse::Client::new(
            insight_clickhouse::Config::new(mock.url(), "insight_datasets"),
        ));

        Self {
            datasets: MemoryDatasets::at(
                Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0)
                    .single()
                    .unwrap_or_else(|| panic!("the fixture time exists")),
            ),
            mock,
            tables,
            definitions: MemoryDefinitions::new(),
        }
    }

    /// Declares a dataset that stands, ready to take records.
    async fn a_ready_dataset(&self) -> R {
        self.mock.add(handlers::provide(Vec::<NoTable>::new()));
        self.mock.add(handlers::record_ddl());
        DatasetLifecycle::new(&self.datasets, &self.tables, &self.definitions)
            .declare(&name("commits"), &declaration())
            .await?;

        Ok(())
    }

    fn ingest(&self) -> DatasetIngest<'_> {
        DatasetIngest::new(&self.datasets, &self.tables)
    }
}

#[tokio::test]
async fn a_record_lands_whole_in_the_table_the_declaration_names() -> R {
    let fixture = Fixture::new();
    fixture.a_ready_dataset().await?;
    let landed = fixture.mock.add(handlers::record::<LandedRecord>());
    let sent = json!({ "day": "2026-09-16", "extra": 1 });

    fixture.ingest().receive(&name("commits"), &sent).await?;

    let rows: Vec<LandedRecord> = landed.collect().await;
    let [record] = rows.as_slice() else {
        panic!("one record landed, got {rows:?}");
    };
    assert_eq!(record.table_name, "commits");
    assert_eq!(
        serde_json::from_str::<Value>(&record.raw_data)?,
        sent,
        "the record is stored whole, keys the declaration never named and all"
    );
    assert!(!record.id.is_nil());
    assert!(record.received_at.timestamp() > 0);

    Ok(())
}

#[tokio::test]
async fn a_dataset_nobody_declared_takes_no_records() -> R {
    let fixture = Fixture::new();

    let refused = fixture
        .ingest()
        .receive(&name("commits"), &json!({ "day": "2026-09-16" }))
        .await;

    assert!(matches!(refused, Err(IngestError::NotReady)), "{refused:?}");

    Ok(())
}

#[tokio::test]
async fn a_dataset_still_being_made_takes_no_records() -> R {
    let fixture = Fixture::new();
    fixture
        .datasets
        .take_create(&name("commits"), &declaration())
        .await?;

    let refused = fixture
        .ingest()
        .receive(&name("commits"), &json!({ "day": "2026-09-16" }))
        .await;

    assert!(matches!(refused, Err(IngestError::NotReady)), "{refused:?}");

    Ok(())
}

#[tokio::test]
async fn a_record_that_reached_no_table_is_refused_rather_than_accepted() -> R {
    let fixture = Fixture::new();
    fixture.a_ready_dataset().await?;
    // What a removal winning the race against this write looks like.
    fixture.mock.add(handlers::exception(60));

    let refused = fixture
        .ingest()
        .receive(&name("commits"), &json!({ "day": "2026-09-16" }))
        .await;

    assert!(matches!(refused, Err(IngestError::NotReady)), "{refused:?}");

    Ok(())
}

#[tokio::test]
async fn a_dataset_being_removed_takes_no_more_records() -> R {
    let fixture = Fixture::new();
    fixture.a_ready_dataset().await?;
    fixture.datasets.take_remove(&name("commits")).await?;

    let refused = fixture
        .ingest()
        .receive(&name("commits"), &json!({ "day": "2026-09-16" }))
        .await;

    assert!(matches!(refused, Err(IngestError::NotReady)), "{refused:?}");

    Ok(())
}
