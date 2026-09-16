use chrono::{TimeZone as _, Utc};
use clickhouse::test::{Mock, handlers};
use serde::Serialize;
use serde_json::json;

use super::*;
use crate::store::datasets::memory::MemoryDatasets;

type R = Result<(), Box<dyn std::error::Error>>;

/// What `system.tables` answers about a name nothing holds.
#[derive(Debug, Serialize, clickhouse::Row)]
struct NoTable {
    sorting_key: String,
}

fn name(value: &str) -> DefinitionName {
    DefinitionName::parse(value).unwrap_or_else(|error| panic!("the name parses: {error}"))
}

fn declaration() -> Value {
    json!({
        "title": "Commits",
        "fields": [
            { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
            { "name": "lines", "path": "lines", "type": "int" }
        ]
    })
}

struct Fixture {
    mock: Mock,
    datasets: MemoryDatasets,
    tables: DatasetTables,
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
        }
    }

    /// The datasets database answers that nothing holds the name.
    fn nothing_holds_the_name(&self) {
        self.mock.add(handlers::provide(Vec::<NoTable>::new()));
    }

    fn lifecycle(&self) -> DatasetLifecycle<'_> {
        DatasetLifecycle::new(&self.datasets, &self.tables)
    }
}

#[tokio::test]
async fn declaring_a_dataset_provisions_its_table_and_leaves_it_ready() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    let recording = fixture.mock.add(handlers::record_ddl());

    let stored = fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;

    assert_eq!(stored, declaration());
    let Some(dataset) = fixture.datasets.get(&name("commits")).await? else {
        panic!("the dataset stands");
    };
    assert_eq!(dataset.state, DatasetState::Ready);
    assert!(dataset.held.is_none(), "the operation is released");
    let Some(table) = dataset.physical_table else {
        panic!("the dataset knows where its records are");
    };
    assert!(
        recording.query().await.contains(&table),
        "the table the row names is the one that was made"
    );

    Ok(())
}

#[tokio::test]
async fn a_declaration_that_does_not_hold_is_refused_before_any_table_is_made() -> R {
    let fixture = Fixture::new();

    let refused = fixture
        .lifecycle()
        .declare(
            &name("commits"),
            &json!({ "title": "Commits", "fields": [{ "name": "day", "path": "day", "type": "moment" }] }),
        )
        .await;

    assert!(
        matches!(refused, Err(DatasetChangeError::Unreadable(_))),
        "{refused:?}"
    );
    assert!(
        fixture.datasets.get(&name("commits")).await?.is_none(),
        "nothing is claimed by a body that cannot be read"
    );

    Ok(())
}

#[tokio::test]
async fn a_declaration_that_is_wrong_twice_is_refused_with_both_reasons() -> R {
    let fixture = Fixture::new();

    let refused = fixture
        .lifecycle()
        .declare(
            &name("commits"),
            &json!({
                "title": "Commits",
                "fields": [
                    { "name": "day", "path": "day", "type": "datetime" },
                    { "name": "day", "path": "other", "type": "int" }
                ],
                "row_identity": ["nowhere"],
            }),
        )
        .await;

    let Err(DatasetChangeError::Invalid(violations)) = refused else {
        panic!("expected violations, got {refused:?}");
    };
    assert!(
        violations.len() > 1,
        "every problem is reported at once: {violations:?}"
    );
    assert!(fixture.datasets.get(&name("commits")).await?.is_none());

    Ok(())
}

#[tokio::test]
async fn a_dataset_another_attempt_is_making_is_not_declared_twice() -> R {
    let fixture = Fixture::new();
    fixture
        .datasets
        .take_create(&name("commits"), &declaration())
        .await?;

    let refused = fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await;

    assert!(
        matches!(refused, Err(DatasetChangeError::Refused(Refused::Busy(_)))),
        "{refused:?}"
    );

    Ok(())
}

#[tokio::test]
async fn replacing_the_declaration_of_a_dataset_that_stands_touches_no_table() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    fixture.mock.add(handlers::record_ddl());
    fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;
    let Some(before) = fixture.datasets.get(&name("commits")).await? else {
        panic!("the dataset stands");
    };

    let second = json!({
        "title": "Commits per day",
        "fields": [
            { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
            { "name": "lines", "path": "lines", "type": "int" }
        ]
    });
    fixture
        .lifecycle()
        .declare(&name("commits"), &second)
        .await?;

    let Some(after) = fixture.datasets.get(&name("commits")).await? else {
        panic!("the dataset still stands");
    };
    assert_eq!(after.declaration, second);
    assert_eq!(
        after.physical_table, before.physical_table,
        "a replacement leaves the records where they are"
    );
    assert_eq!(after.state, DatasetState::Ready);

    Ok(())
}

/// A store that hands out an operation and then reports the attempt stale,
/// which is what an attempt whose lease lapsed mid-flight meets.
#[derive(Debug)]
struct LosesTheDataset(MemoryDatasets);

#[async_trait::async_trait]
impl Datasets for LosesTheDataset {
    async fn get(
        &self,
        name: &DefinitionName,
    ) -> Result<Option<crate::domain::datasets::Dataset>, DatasetStoreError> {
        self.0.get(name).await
    }

    async fn list(&self) -> Result<Vec<String>, DatasetStoreError> {
        self.0.list().await
    }

    async fn take_create(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<Attempt, DatasetStoreError> {
        self.0.take_create(name, declaration).await
    }

    async fn take_remove(&self, name: &DefinitionName) -> Result<Attempt, DatasetStoreError> {
        self.0.take_remove(name).await
    }

    async fn replace(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<(), DatasetStoreError> {
        self.0.replace(name, declaration).await
    }

    async fn finish(
        &self,
        _name: &DefinitionName,
        _token: &OperationToken,
        _finish: Finish,
    ) -> Result<Owning, DatasetStoreError> {
        Ok(Owning::Lost)
    }
}

#[tokio::test]
async fn an_attempt_that_lost_the_dataset_takes_away_the_table_it_made() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    let recording = fixture.mock.add(handlers::record_ddl());
    let dropping = fixture.mock.add(handlers::record_ddl());
    let datasets = LosesTheDataset(MemoryDatasets::at(
        Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0)
            .single()
            .unwrap_or_else(|| panic!("the fixture time exists")),
    ));

    let refused = DatasetLifecycle::new(&datasets, &fixture.tables)
        .declare(&name("commits"), &declaration())
        .await;

    assert!(
        matches!(refused, Err(DatasetChangeError::Refused(Refused::Busy(_)))),
        "{refused:?}"
    );
    let made = recording.query().await;
    let dropped = dropping.query().await;
    assert!(dropped.contains("DROP TABLE"), "{dropped}");
    let table = made
        .split("IF NOT EXISTS ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("the create names a table: {made}"))
        .trim_matches('`');
    assert!(
        dropped.contains(table),
        "the table taken away must be the one this attempt made: {dropped}"
    );

    Ok(())
}
