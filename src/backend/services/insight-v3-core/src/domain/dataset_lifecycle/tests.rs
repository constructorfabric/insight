use chrono::{TimeZone as _, Utc};
use clickhouse::test::{Mock, handlers};
use serde::Serialize;
use serde_json::json;

use super::*;
use crate::domain::kinds::dataset::state::DatasetState;
use crate::store::datasets::memory::MemoryDatasets;
use crate::store::definitions::memory::MemoryDefinitions;
use crate::store::relations::Relations;

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
        "source": { "kind": "stream" },
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
    relations: Relations,
    definitions: MemoryDefinitions,
}

impl Fixture {
    fn new() -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let tables = DatasetTables::new(insight_clickhouse::Client::new(
            insight_clickhouse::Config::new(mock.url(), "insight_datasets"),
        ));
        let relations = Relations::new(insight_clickhouse::Client::new(
            insight_clickhouse::Config::new(mock.url(), "insight"),
        ));

        Self {
            datasets: MemoryDatasets::at(
                Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0)
                    .single()
                    .unwrap_or_else(|| panic!("the fixture time exists")),
            ),
            mock,
            tables,
            relations,
            definitions: MemoryDefinitions::new(),
        }
    }

    /// The datasets database answers that nothing holds the name.
    fn nothing_holds_the_name(&self) {
        self.mock.add(handlers::provide(Vec::<NoTable>::new()));
    }

    fn lifecycle(&self) -> DatasetLifecycle<'_> {
        DatasetLifecycle::new(
            &self.datasets,
            &self.tables,
            &self.relations,
            &self.definitions,
        )
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
        matches!(&refused, Err(DatasetChangeError::Invalid(violations))
            if violations.iter().any(|one| one.field == "fields[0].type")),
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
                "source": { "kind": "stream" },
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
        .await?
        .attempt();

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
        "source": { "kind": "stream" },
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

    async fn page(
        &self,
        needle: &str,
        page: crate::domain::definition::Page,
    ) -> Result<crate::domain::definition::NamePage, DatasetStoreError> {
        self.0.page(needle, page).await
    }

    async fn take_create(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<crate::domain::datasets::Taken, DatasetStoreError> {
        self.0.take_create(name, declaration).await
    }

    async fn take_remove(&self, name: &DefinitionName) -> Result<Attempt, DatasetStoreError> {
        self.0.take_remove(name).await
    }

    async fn replace(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<bool, DatasetStoreError> {
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

/// A store that took the operation and then stopped answering, which is what
/// a connection lost on the way back from a commit looks like.
#[derive(Debug)]
struct StopsAnswering(MemoryDatasets);

#[async_trait::async_trait]
impl Datasets for StopsAnswering {
    async fn get(
        &self,
        name: &DefinitionName,
    ) -> Result<Option<crate::domain::datasets::Dataset>, DatasetStoreError> {
        self.0.get(name).await
    }

    async fn list(&self) -> Result<Vec<String>, DatasetStoreError> {
        self.0.list().await
    }

    async fn page(
        &self,
        needle: &str,
        page: crate::domain::definition::Page,
    ) -> Result<crate::domain::definition::NamePage, DatasetStoreError> {
        self.0.page(needle, page).await
    }

    async fn take_create(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<crate::domain::datasets::Taken, DatasetStoreError> {
        self.0.take_create(name, declaration).await
    }

    async fn take_remove(&self, name: &DefinitionName) -> Result<Attempt, DatasetStoreError> {
        self.0.take_remove(name).await
    }

    async fn replace(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<bool, DatasetStoreError> {
        self.0.replace(name, declaration).await
    }

    async fn finish(
        &self,
        _name: &DefinitionName,
        _token: &OperationToken,
        _finish: Finish,
    ) -> Result<Owning, DatasetStoreError> {
        Err(DatasetStoreError::UnreadableRow("no answer".to_owned()))
    }
}

/// A store that did not answer has not said the write failed: the commit may
/// have landed and the acknowledgement been lost, leaving a dataset that
/// stands and names this table. A table nothing names can be swept up; one a
/// standing dataset names cannot be brought back.
#[tokio::test]
async fn a_publication_the_store_never_answered_for_keeps_its_table() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    let recording = fixture.mock.add(handlers::record_ddl());
    let datasets = StopsAnswering(MemoryDatasets::at(
        Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0)
            .single()
            .unwrap_or_else(|| panic!("the fixture time exists")),
    ));

    let failed = DatasetLifecycle::new(
        &datasets,
        &fixture.tables,
        &fixture.relations,
        &fixture.definitions,
    )
    .declare(&name("commits"), &declaration())
    .await;

    assert!(failed.is_err(), "{failed:?}");
    let issued = recording.query().await;
    assert!(
        !issued.contains("DROP TABLE"),
        "an unanswered publication must leave its table alone: {issued}"
    );

    Ok(())
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

    let refused = DatasetLifecycle::new(
        &datasets,
        &fixture.tables,
        &fixture.relations,
        &fixture.definitions,
    )
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

#[tokio::test]
async fn removing_a_dataset_takes_its_records_and_its_row_with_it() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    let made = fixture.mock.add(handlers::record_ddl());
    fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;
    fixture.mock.add(handlers::provide(vec![NoTable {
        sorting_key: "table_name, received_at, id".to_owned(),
    }]));
    let dropping = fixture.mock.add(handlers::record_ddl());

    let removed = fixture.lifecycle().remove(&name("commits")).await?;

    assert_eq!(removed, Removal::Removed);
    assert!(fixture.datasets.get(&name("commits")).await?.is_none());
    let dropped = dropping.query().await;
    assert!(dropped.contains("DROP TABLE"), "{dropped}");
    let table = made
        .query()
        .await
        .split("IF NOT EXISTS ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("the create names a table"))
        .trim_matches('`')
        .to_owned();
    assert!(
        dropped.contains(&table),
        "the table dropped must be the one the row named: {dropped}"
    );

    Ok(())
}

#[tokio::test]
async fn removing_a_dataset_that_was_never_declared_drops_nothing() -> R {
    let fixture = Fixture::new();

    let refused = fixture.lifecycle().remove(&name("commits")).await;

    assert!(
        matches!(refused, Err(DatasetChangeError::NotFound)),
        "{refused:?}"
    );

    Ok(())
}

#[tokio::test]
async fn a_removal_already_under_way_is_this_requests_own_outcome() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    fixture.mock.add(handlers::record_ddl());
    fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;
    fixture.datasets.take_remove(&name("commits")).await?;

    let second = fixture.lifecycle().remove(&name("commits")).await?;

    assert_eq!(second, Removal::AlreadyUnderWay);
    assert!(
        fixture.datasets.get(&name("commits")).await?.is_some(),
        "the row stays until the removal under way finishes"
    );

    Ok(())
}

#[tokio::test]
async fn a_dataset_being_created_is_not_removed_from_under_that_attempt() -> R {
    let fixture = Fixture::new();
    fixture
        .datasets
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();

    let refused = fixture.lifecycle().remove(&name("commits")).await;

    assert!(
        matches!(
            refused,
            Err(DatasetChangeError::Refused(Refused::Busy(
                Operation::Create
            )))
        ),
        "{refused:?}"
    );

    Ok(())
}

#[tokio::test]
async fn a_table_that_is_no_longer_ours_is_left_where_it_is() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    fixture.mock.add(handlers::record_ddl());
    fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;
    fixture.mock.add(handlers::provide(vec![NoTable {
        sorting_key: "event_date, project_id".to_owned(),
    }]));

    let refused = fixture.lifecycle().remove(&name("commits")).await;

    assert!(
        matches!(
            refused,
            Err(DatasetChangeError::Table(DatasetTableError::NotOurs(_)))
        ),
        "{refused:?}"
    );

    Ok(())
}

/// A metric reading the fixture dataset, stored as a reader of it.
async fn a_metric_reading_commits(fixture: &Fixture, named: &str, body: Value) -> R {
    let stored = DefinitionName::parse(named)?;
    fixture
        .definitions
        .put(DefinitionKind::Metric, &stored, &body)
        .await?;

    Ok(())
}

fn reading_commits() -> Value {
    json!({
        "dataset": "commits",
        "table": "commits",
        "fields": [{ "field": "lines", "type": "int", "agg": "sum", "as_name": "total" }],
        "group_by": [],
        "filters": []
    })
}

/// The fixture declaration with one field's path moved, and nothing else.
fn declaration_reading_elsewhere() -> Value {
    json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [
            { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
            { "name": "lines", "path": "changed.lines", "type": "int" }
        ]
    })
}

#[tokio::test]
async fn a_dataset_a_metric_still_reads_is_not_taken_away() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    fixture.mock.add(handlers::record_ddl());
    fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;
    a_metric_reading_commits(&fixture, "lines_per_day", reading_commits()).await?;

    let refused = fixture.lifecycle().remove(&name("commits")).await;

    let Err(DatasetChangeError::StillRead(readers)) = refused else {
        panic!("expected a refusal naming the readers, got {refused:?}");
    };
    assert_eq!(readers, vec!["lines_per_day".to_owned()]);
    assert!(
        fixture.datasets.get(&name("commits")).await?.is_some(),
        "the dataset stays while something reads it"
    );

    Ok(())
}

#[tokio::test]
async fn a_replacement_that_leaves_a_reader_unanswerable_is_refused_naming_it() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    fixture.mock.add(handlers::record_ddl());
    fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;
    a_metric_reading_commits(&fixture, "lines_per_day", reading_commits()).await?;

    let without_lines = json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [{ "name": "day", "path": "day", "type": "datetime", "default_clock": true }]
    });
    let refused = fixture
        .lifecycle()
        .declare(&name("commits"), &without_lines)
        .await;

    let Err(DatasetChangeError::WouldBreak(broken)) = refused else {
        panic!("expected a refusal naming the metric, got {refused:?}");
    };
    assert!(
        broken.iter().any(|one| one.metric == "lines_per_day"),
        "{broken:?}"
    );
    let Some(held) = fixture.datasets.get(&name("commits")).await? else {
        panic!("the dataset stands");
    };
    assert_eq!(held.declaration, declaration(), "nothing was written");

    Ok(())
}

#[tokio::test]
async fn a_replacement_that_would_move_a_readers_numbers_is_refused_though_it_stays_valid() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    fixture.mock.add(handlers::record_ddl());
    fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;
    a_metric_reading_commits(&fixture, "lines_per_day", reading_commits()).await?;

    let refused = fixture
        .lifecycle()
        .declare(&name("commits"), &declaration_reading_elsewhere())
        .await;

    let Err(DatasetChangeError::WouldBreak(broken)) = refused else {
        panic!("expected a refusal, got {refused:?}");
    };
    assert!(
        broken
            .iter()
            .any(|one| one.why.contains("somewhere else in the record")),
        "{broken:?}"
    );

    Ok(())
}

#[tokio::test]
async fn a_replacement_that_moves_the_date_a_reader_windows_by_is_refused() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    fixture.mock.add(handlers::record_ddl());
    let with_two_dates = json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [
            { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
            { "name": "merged", "path": "merged", "type": "datetime" },
            { "name": "lines", "path": "lines", "type": "int" }
        ]
    });
    fixture
        .lifecycle()
        .declare(&name("commits"), &with_two_dates)
        .await?;
    a_metric_reading_commits(&fixture, "lines_per_day", reading_commits()).await?;

    let moved_clock = json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [
            { "name": "day", "path": "day", "type": "datetime" },
            { "name": "merged", "path": "merged", "type": "datetime", "default_clock": true },
            { "name": "lines", "path": "lines", "type": "int" }
        ]
    });
    let refused = fixture
        .lifecycle()
        .declare(&name("commits"), &moved_clock)
        .await;

    let Err(DatasetChangeError::WouldBreak(broken)) = refused else {
        panic!("expected a refusal, got {refused:?}");
    };
    assert!(
        broken
            .iter()
            .any(|one| one.why.contains("window selects by")),
        "{broken:?}"
    );

    Ok(())
}

#[tokio::test]
async fn a_replacement_nothing_reads_is_free_to_land() -> R {
    let fixture = Fixture::new();
    fixture.nothing_holds_the_name();
    fixture.mock.add(handlers::record_ddl());
    fixture
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;

    fixture
        .lifecycle()
        .declare(&name("commits"), &declaration_reading_elsewhere())
        .await?;

    let Some(held) = fixture.datasets.get(&name("commits")).await? else {
        panic!("the dataset stands");
    };
    assert_eq!(held.declaration, declaration_reading_elsewhere());

    Ok(())
}

/// A column of a relation, as `system.columns` answers for one.
#[derive(Debug, Serialize, clickhouse::Row)]
struct ColumnRow {
    name: String,
    r#type: String,
}

/// A relation's engine, as `system.tables` answers for one.
#[derive(Debug, Serialize, clickhouse::Row)]
struct EngineRow {
    engine: String,
}

impl EngineRow {
    /// An engine that does not keep superseded rows, so a read counts each
    /// row once and the relation may be bound.
    fn plain() -> Self {
        Self {
            engine: "MergeTree".to_owned(),
        }
    }
}

impl ColumnRow {
    fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            r#type: "String".to_owned(),
        }
    }
}

fn over_a_relation(fields: &serde_json::Value) -> serde_json::Value {
    json!({
        "title": "Collaboration observations",
        "source": {
            "kind": "relation",
            "database": "insight",
            "table": "collab_metric_observations"
        },
        "fields": fields
    })
}

/// A declaration over a stream describes records that have not arrived; one
/// over a relation describes something that exists now, and is held to it.
#[tokio::test]
async fn a_dataset_over_a_relation_is_checked_against_what_the_warehouse_holds() -> R {
    let fixture = Fixture::new();
    fixture.mock.add(handlers::provide(vec![
        ColumnRow::named("metric_date"),
        ColumnRow::named("value"),
    ]));
    fixture
        .mock
        .add(handlers::provide(vec![EngineRow::plain()]));

    let declared = fixture
        .lifecycle()
        .declare(
            &name("collab"),
            &over_a_relation(&json!([
                { "name": "day", "column": "metric_date", "type": "datetime" }
            ])),
        )
        .await;

    assert!(declared.is_ok(), "{declared:?}");
    Ok(())
}

/// A column the relation does not have compiles into every metric over the
/// dataset and then fails on every run, which is far from where the mistake
/// was made.
#[tokio::test]
async fn a_column_the_relation_does_not_have_is_refused_when_it_is_declared() -> R {
    let fixture = Fixture::new();
    fixture
        .mock
        .add(handlers::provide(vec![ColumnRow::named("metric_date")]));
    fixture
        .mock
        .add(handlers::provide(vec![EngineRow::plain()]));

    let refused = fixture
        .lifecycle()
        .declare(
            &name("collab"),
            &over_a_relation(&json!([
                { "name": "day", "column": "metric_date", "type": "datetime" },
                { "name": "who", "column": "nonsense", "type": "string" }
            ])),
        )
        .await;

    let Err(DatasetChangeError::Invalid(violations)) = refused else {
        panic!("should be refused as invalid: {refused:?}")
    };
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].field, "fields[1].column");

    Ok(())
}

#[tokio::test]
async fn a_dataset_over_a_relation_the_warehouse_does_not_have_is_refused() -> R {
    let fixture = Fixture::new();
    fixture.mock.add(handlers::provide(Vec::<ColumnRow>::new()));

    let refused = fixture
        .lifecycle()
        .declare(
            &name("collab"),
            &over_a_relation(&json!([
                { "name": "day", "column": "metric_date", "type": "datetime" }
            ])),
        )
        .await;

    let Err(DatasetChangeError::Invalid(violations)) = refused else {
        panic!("should be refused as invalid: {refused:?}")
    };
    assert_eq!(violations[0].field, "source.table");

    Ok(())
}

/// The connection that may create or drop a table is bound to the datasets
/// database. A dataset over a relation must not provision anything at all:
/// the relation is the warehouse's, made and taken away by the warehouse.
///
/// Nothing answers a DDL here, so a statement issued against the datasets
/// database has no handler and the declare fails. Succeeding is the proof
/// that none was issued.
#[tokio::test]
async fn declaring_a_dataset_over_a_relation_provisions_no_table() -> R {
    let fixture = Fixture::new();
    fixture
        .mock
        .add(handlers::provide(vec![ColumnRow::named("metric_date")]));
    fixture
        .mock
        .add(handlers::provide(vec![EngineRow::plain()]));

    let declared = fixture
        .lifecycle()
        .declare(
            &name("collab"),
            &over_a_relation(&json!([
                { "name": "day", "column": "metric_date", "type": "datetime" }
            ])),
        )
        .await;

    assert!(declared.is_ok(), "no table is provisioned: {declared:?}");
    Ok(())
}

/// A relation whose engine this cannot vouch for might be counted more than
/// once by a plain read, and `FINAL` is refused outright by the engines that
/// do not need it. Answering a number that is quietly too high is the one
/// outcome worth refusing a declaration over.
#[tokio::test]
async fn a_relation_this_cannot_vouch_for_is_refused_with_the_way_round_it() -> R {
    let fixture = Fixture::new();
    fixture
        .mock
        .add(handlers::provide(vec![ColumnRow::named("metric_date")]));
    fixture.mock.add(handlers::provide(vec![EngineRow {
        engine: "ReplacingMergeTree".to_owned(),
    }]));

    let refused = fixture
        .lifecycle()
        .declare(
            &name("collab"),
            &over_a_relation(&json!([
                { "name": "day", "column": "metric_date", "type": "datetime" }
            ])),
        )
        .await;

    let Err(DatasetChangeError::Invalid(violations)) = refused else {
        panic!("should be refused as invalid: {refused:?}")
    };
    assert_eq!(violations[0].field, "source.table");
    assert!(
        violations[0].detail.contains("view"),
        "should say the way round it: {}",
        violations[0].detail
    );

    Ok(())
}

/// A dataset over a relation declared and standing, for a test that then
/// does something to it.
async fn a_ready_relation_dataset(fixture: &Fixture) -> R {
    fixture
        .mock
        .add(handlers::provide(vec![ColumnRow::named("metric_date")]));
    fixture
        .mock
        .add(handlers::provide(vec![EngineRow::plain()]));
    fixture
        .lifecycle()
        .declare(
            &name("collab"),
            &over_a_relation(&json!([
                { "name": "day", "column": "metric_date", "type": "datetime" }
            ])),
        )
        .await?;

    Ok(())
}

/// Turning a stream into a relation strands the records already sent, with
/// the table still recorded against a dataset that no longer reads it.
/// Turning a relation into a stream leaves a dataset whose row says ready
/// and which has no table to read, for good.
#[tokio::test]
async fn what_a_dataset_is_over_cannot_be_changed_by_replacing_its_declaration() -> R {
    let into_a_relation = Fixture::new();
    into_a_relation.nothing_holds_the_name();
    into_a_relation.mock.add(handlers::record_ddl());
    into_a_relation
        .lifecycle()
        .declare(&name("commits"), &declaration())
        .await?;

    into_a_relation
        .mock
        .add(handlers::provide(vec![ColumnRow::named("metric_date")]));
    into_a_relation
        .mock
        .add(handlers::provide(vec![EngineRow::plain()]));
    let refused = into_a_relation
        .lifecycle()
        .declare(
            &name("commits"),
            &over_a_relation(&json!([
                { "name": "day", "column": "metric_date", "type": "datetime" }
            ])),
        )
        .await;
    let Err(DatasetChangeError::Invalid(violations)) = refused else {
        panic!("a stream may not become a relation: {refused:?}")
    };
    assert_eq!(violations[0].field, "source.kind");

    let into_a_stream = Fixture::new();
    a_ready_relation_dataset(&into_a_stream).await?;

    let refused = into_a_stream
        .lifecycle()
        .declare(&name("collab"), &declaration())
        .await;
    let Err(DatasetChangeError::Invalid(violations)) = refused else {
        panic!("a relation may not become a stream: {refused:?}")
    };
    assert_eq!(violations[0].field, "source.kind");

    Ok(())
}

/// The relation is the warehouse's: this service did not make it and may not
/// take it away. Nothing answers a DDL here, so a statement issued against
/// the datasets database has no handler and the removal fails. Succeeding is
/// the proof none was issued.
#[tokio::test]
async fn removing_a_dataset_over_a_relation_drops_nothing() -> R {
    let fixture = Fixture::new();
    a_ready_relation_dataset(&fixture).await?;

    let removed = fixture.lifecycle().remove(&name("collab")).await?;

    assert_eq!(removed, Removal::Removed);
    Ok(())
}

/// An attempt that takes a name over from one that had already provisioned a
/// table leaves that name in the row. Ingest decides by the table rather
/// than by the declaration, so a dataset over a relation that kept one would
/// take records into a table nothing reads.
#[tokio::test]
async fn a_relation_taking_over_a_name_forgets_the_table_the_lost_attempt_made() -> R {
    let fixture = Fixture::new();
    let first = fixture
        .datasets
        .take_create(&name("collab"), &declaration())
        .await?
        .attempt();
    fixture
        .datasets
        .finish(
            &name("collab"),
            &first.token,
            Finish::Provisioned("ds_collab_first".to_owned()),
        )
        .await?;

    // The lease lapses and a second attempt declares the same name, over a
    // relation this time.
    fixture.datasets.set_now(
        Utc.with_ymd_and_hms(2026, 9, 16, 14, 0, 0)
            .single()
            .unwrap_or_else(|| panic!("the fixture time exists")),
    );
    a_ready_relation_dataset(&fixture).await?;

    let held = fixture
        .datasets
        .get(&name("collab"))
        .await?
        .unwrap_or_else(|| panic!("the dataset stands"));
    assert_eq!(held.state, DatasetState::Ready);
    assert_eq!(
        held.physical_table, None,
        "a dataset over a relation records no table of ours"
    );

    Ok(())
}
