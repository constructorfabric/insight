//! The raw input stream, read from a live `ClickHouse`.
//!
//! `STREAM_SQL` is checked by a unit test that greps its text, which cannot see
//! the two hazards the query is written around: `toString` of a `Nullable`
//! column stays `Nullable` and the strict decoder rejects it, and an alias
//! reusing a source column name can trip a cyclic-alias error. Both are the
//! database's verdict, so only a live read establishes them.
//!
//! INVARIANT: never `#[ignore]` these — the identity CI job runs `cargo test`
//! without `--include-ignored`, so an ignored case silently stops running.
//!
//! Every row a case writes carries a source type minted for that case, and each
//! case reads back only its own rows. The stream is deliberately unfiltered, so
//! this is what keeps the suite parallel-safe and lets it share an instance with
//! rows it did not write — it never deletes and never truncates.

use clickhouse::Row;
use insight_clickhouse::{Client, Config};
use serde::Deserialize;
use uuid::Uuid;

use crate::domain::seed::IdentityInputRow;
use crate::domain::seed_service::IdentityInputsReader;

use super::identity_inputs::ClickHouseIdentityInputsReader;

type TestResult = anyhow::Result<()>;

const URL_VAR: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";

/// The table the stream reads, as `connectors-ddl/identity.sql` declares it.
/// Created when absent so the suite runs against a bare instance; never dropped,
/// because the instance may be carrying rows this suite did not write.
const DDL: &str = "CREATE TABLE IF NOT EXISTS identity.identity_inputs (
        `unique_key` String,
        `insight_tenant_id` UUID,
        `insight_source_id` UUID,
        `insight_source_type` String,
        `source_account_id` Nullable(String),
        `value_type` String,
        `value` Nullable(String),
        `value_field_name` String,
        `operation_type` String,
        `_synced_at` DateTime64(3),
        `_version` Int64
    ) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key
    SETTINGS allow_nullable_key = 1";

/// The columns `STREAM_SQL` reads, with the type each has to carry for the
/// strict decoder to accept it. An instance may already hold a table of this
/// name in another shape — a stale dump, a hand-loaded scratch copy — and
/// reading that one would fail on the decoder rather than on anything this
/// suite is about, so the fixture skips instead.
const REQUIRED_TYPES: [(&str, &str); 7] = [
    ("insight_source_type", "String"),
    ("insight_source_id", "UUID"),
    ("source_account_id", "Nullable(String)"),
    ("value_type", "String"),
    ("value", "Nullable(String)"),
    ("operation_type", "String"),
    ("_synced_at", "DateTime64(3)"),
];

#[derive(Row, Deserialize)]
struct ColumnShape {
    name: String,
    #[serde(rename = "type")]
    ch_type: String,
}

/// `Ok(None)` when the table is not the shape this suite writes and reads.
async fn shape_matches(ch: &Client) -> anyhow::Result<bool> {
    let columns: Vec<ColumnShape> = ch
        .query(
            "SELECT name, type FROM system.columns \
             WHERE database = 'identity' AND table = 'identity_inputs'",
        )
        .fetch_all()
        .await?;

    for (name, want) in REQUIRED_TYPES {
        let Some(found) = columns.iter().find(|c| c.name == name) else {
            eprintln!("skip: identity.identity_inputs here has no {name} column");
            return Ok(false);
        };
        if found.ch_type != want {
            eprintln!(
                "skip: identity.identity_inputs here carries {name} as {}, not {want}",
                found.ch_type
            );
            return Ok(false);
        }
    }
    Ok(true)
}

struct Fixture {
    ch: Client,
    reader: ClickHouseIdentityInputsReader,
    tenant: Uuid,
    source_id: Uuid,
    /// Minted per case: the tag every row of this case carries, and the filter
    /// that hides every other case's rows from it.
    source_type: String,
}

/// Empty counts as unset: the CI matrix passes '' to entries without a
/// provisioned ClickHouse, and set-but-empty must skip exactly like absent.
async fn fixture_or_skip() -> anyhow::Result<Option<Fixture>> {
    let url = std::env::var(URL_VAR).unwrap_or_default();
    if url.is_empty() {
        eprintln!("skip: set {URL_VAR} to run");
        return Ok(None);
    }
    let user = std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER").unwrap_or_default();
    let password = std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD").unwrap_or_default();

    // The database has to be made from a connection that does not already name
    // it: the client sends its configured database with every query, so one
    // pointed at `identity` cannot be the one that creates `identity`.
    let connect = |database: &str| {
        let mut config = Config::new(url.clone(), database);
        if !user.is_empty() {
            config = config.with_auth(&user, &password);
        }
        Client::new(config)
    };
    connect("default")
        .query("CREATE DATABASE IF NOT EXISTS identity")
        .execute()
        .await?;
    let ch = connect("identity");
    ch.query(DDL).execute().await?;
    if !shape_matches(&ch).await? {
        return Ok(None);
    }

    Ok(Some(Fixture {
        ch,
        reader: ClickHouseIdentityInputsReader::connect(&url, "identity", &user, &password),
        tenant: Uuid::now_v7(),
        source_id: Uuid::now_v7(),
        source_type: format!("ci-{}", Uuid::now_v7().simple()),
    }))
}

impl Fixture {
    /// One observation. `value` is `None` for the NULL the column allows;
    /// `synced_at` is stated per row because the order the stream returns is
    /// what several cases are about.
    async fn write(
        &self,
        account: &str,
        value_type: &str,
        value: Option<&str>,
        operation: &str,
        synced_at: &str,
    ) -> anyhow::Result<()> {
        let value_sql = value.map_or_else(|| "NULL".to_owned(), |v| format!("'{v}'"));
        self.ch
            .query(&format!(
                "INSERT INTO identity.identity_inputs (unique_key, insight_tenant_id,
                     insight_source_id, insight_source_type, source_account_id, value_type,
                     value, value_field_name, operation_type, _synced_at, _version)
                 VALUES ('{key}', toUUID('{tenant}'), toUUID('{source_id}'), '{source_type}',
                     '{account}', '{value_type}', {value_sql}, 'fixture.field', '{operation}',
                     toDateTime64('{synced_at}', 3), 1)",
                key = Uuid::now_v7(),
                tenant = self.tenant,
                source_id = self.source_id,
                source_type = self.source_type,
            ))
            .execute()
            .await?;
        Ok(())
    }

    /// What the stream returns for THIS case — every other case's rows, and any
    /// the instance already carried, are somebody else's source type.
    async fn read_own(&self) -> anyhow::Result<Vec<IdentityInputRow>> {
        let rows = self.reader.stream(self.tenant).await?;
        Ok(rows
            .into_iter()
            .filter(|r| r.source_type == self.source_type)
            .collect())
    }
}

#[tokio::test]
async fn an_observation_survives_the_round_trip_through_the_real_column_types() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    f.write(
        "acct-1",
        "email",
        Some("person@inputs.test"),
        "UPSERT",
        "2026-01-02 03:04:05.678",
    )
    .await?;

    let rows = f.read_own().await?;

    assert_eq!(rows.len(), 1, "the case wrote exactly one readable row");
    let row = &rows[0];
    assert_eq!(row.source_id, f.source_id, "the id must survive toString");
    assert_eq!(row.source_account_id, "acct-1");
    assert_eq!(row.value_type, "email");
    assert_eq!(row.value, "person@inputs.test");
    assert_eq!(
        row.synced_at.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
        "2026-01-02 03:04:05.678",
        "the timestamp must survive toString and the reparse"
    );
    assert!(!row.is_delete, "an UPSERT is not a closure signal");
    Ok(())
}

#[tokio::test]
async fn an_upsert_stating_no_value_is_not_an_observation() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    f.write(
        "acct-empty",
        "email",
        Some(""),
        "UPSERT",
        "2026-01-02 03:04:05.000",
    )
    .await?;
    f.write(
        "acct-null",
        "email",
        None,
        "UPSERT",
        "2026-01-02 03:04:05.000",
    )
    .await?;

    assert!(
        f.read_own().await?.is_empty(),
        "an UPSERT carrying nothing states nothing, whether it is blank or NULL"
    );
    Ok(())
}

#[tokio::test]
async fn a_closure_signal_is_read_even_though_it_carries_no_value() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // DELETE rows arrive with an empty value by the write contract, so the
    // non-empty filter must not reach them — value-filtering DELETEs would drop
    // every tombstone and leave the seed unable to close an account.
    f.write(
        "acct-gone",
        "email",
        Some(""),
        "DELETE",
        "2026-01-02 03:04:05.000",
    )
    .await?;

    let rows = f.read_own().await?;

    assert_eq!(rows.len(), 1, "the tombstone must survive the value filter");
    assert!(
        rows[0].is_delete,
        "a DELETE row must be flagged as a closure"
    );
    Ok(())
}

#[tokio::test]
async fn an_accounts_observations_come_back_latest_first() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // `build_profiles` takes the FIRST row per account as the one in force, so
    // the order the stream returns is a contract, not a presentation detail.
    f.write(
        "acct-moved",
        "email",
        Some("former@inputs.test"),
        "UPSERT",
        "2026-01-01 00:00:00.000",
    )
    .await?;
    f.write(
        "acct-moved",
        "email",
        Some("current@inputs.test"),
        "UPSERT",
        "2026-06-01 00:00:00.000",
    )
    .await?;

    let values: Vec<String> = f.read_own().await?.into_iter().map(|r| r.value).collect();

    assert_eq!(
        values,
        vec!["current@inputs.test", "former@inputs.test"],
        "the latest observation must lead the account's rows"
    );
    Ok(())
}
