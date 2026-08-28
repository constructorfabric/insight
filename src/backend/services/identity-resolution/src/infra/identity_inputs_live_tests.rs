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
//! case reads back only its own rows — the stream is deliberately unfiltered, so
//! that tag is what keeps the suite parallel-safe.
//!
//! SAFETY: the suite writes into the table the persons-seed consumes, so it
//! refuses to run at all unless every row present is one it wrote, and it clears
//! its own leavings before the first case rather than after the last — a panic,
//! a kill or a failed assertion must not be able to leave a fake observation
//! behind for a real seed to read.

use std::sync::LazyLock;

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
/// because a deployment's own table is not this suite's to remove.
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

/// The prefix every source type this suite mints carries, so its own rows can be
/// told from anybody else's.
///
/// INVARIANT: `TAG_FAMILY` must match every tag this suite has ever written, not
/// just the current one — leavings from an older tag are still this suite's to
/// clear, and treating them as foreign would wedge the instance for good.
const TAG_PREFIX: &str = "ci-identity-inputs-";
const TAG_FAMILY: &str = "ci-%";

/// Runs once per process, before any case is handed a fixture, so it cannot
/// delete rows a concurrent case is still using.
static LEAVINGS_CLEARED: LazyLock<tokio::sync::OnceCell<()>> =
    LazyLock::new(tokio::sync::OnceCell::new);

async fn clear_leavings(ch: &Client) -> anyhow::Result<()> {
    ch.query(
        "ALTER TABLE identity.identity_inputs DELETE \
         WHERE insight_source_type LIKE ? SETTINGS mutations_sync = 2",
    )
    .bind(TAG_FAMILY)
    .execute()
    .await?;
    Ok(())
}

#[derive(Row, Deserialize)]
struct RowCount {
    n: u64,
}

#[derive(Row, Deserialize)]
struct ColumnShape {
    name: String,
    #[serde(rename = "type")]
    ch_type: String,
}

/// Fails rather than skips: the environment variable being set says the caller
/// asked for this suite, so a table it must not write into is an error to
/// report, not an absence to pass over quietly.
async fn refuse_a_table_holding_other_rows(ch: &Client) -> anyhow::Result<()> {
    let counted: Vec<RowCount> = ch
        .query(
            "SELECT count() AS n FROM identity.identity_inputs \
             WHERE insight_source_type NOT LIKE ?",
        )
        .bind(TAG_FAMILY)
        .fetch_all()
        .await?;
    let foreign = counted.first().map_or(0, |c| c.n);
    anyhow::ensure!(
        foreign == 0,
        "identity.identity_inputs on this instance holds {foreign} rows this suite did not \
         write; it seeds real persons, so point {URL_VAR} at an instance that does not carry it"
    );
    Ok(())
}

/// `Ok(false)` when the table is not the shape this suite writes and reads.
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
    LEAVINGS_CLEARED
        .get_or_try_init(|| clear_leavings(&ch))
        .await?;
    refuse_a_table_holding_other_rows(&ch).await?;

    Ok(Some(Fixture {
        ch,
        reader: ClickHouseIdentityInputsReader::connect(&url, "identity", &user, &password),
        tenant: Uuid::now_v7(),
        source_id: Uuid::now_v7(),
        source_type: format!("{TAG_PREFIX}{}", Uuid::now_v7().simple()),
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

    /// Drop everything this case wrote, so a green run leaves the table as it
    /// found it. Not the safety net — that is `clear_leavings`, which runs
    /// before the first case and therefore survives a panic here.
    async fn forget(&self) -> anyhow::Result<()> {
        self.ch
            .query(
                "ALTER TABLE identity.identity_inputs DELETE \
                 WHERE insight_source_type = ? SETTINGS mutations_sync = 2",
            )
            .bind(&self.source_type)
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
    f.forget().await
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
    // The control: an emptiness this case reads must be the filter's doing, not
    // a read that returns nothing whatever it is asked.
    f.write(
        "acct-stated",
        "email",
        Some("stated@inputs.test"),
        "UPSERT",
        "2026-01-02 03:04:05.000",
    )
    .await?;

    let accounts: Vec<String> = f
        .read_own()
        .await?
        .into_iter()
        .map(|r| r.source_account_id)
        .collect();

    assert_eq!(
        accounts,
        vec!["acct-stated"],
        "an UPSERT carrying nothing states nothing, whether it is blank or NULL"
    );
    f.forget().await
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
    f.forget().await
}

#[tokio::test]
async fn an_accounts_observations_come_back_latest_first() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // `build_profiles` takes the FIRST row per account as the one in force, so
    // the order the stream returns is a contract, not a presentation detail.
    // INVARIANT: the newer value sorts AFTER the older one alphabetically. The
    // query falls back to `value_type, value` ASC, so equal-looking addresses
    // would let that tiebreaker reproduce the expected order on its own and the
    // case could not tell it from ordering by time.
    f.write(
        "acct-moved",
        "email",
        Some("aaa.former@inputs.test"),
        "UPSERT",
        "2026-01-01 00:00:00.000",
    )
    .await?;
    f.write(
        "acct-moved",
        "email",
        Some("zzz.current@inputs.test"),
        "UPSERT",
        "2026-06-01 00:00:00.000",
    )
    .await?;

    let values: Vec<String> = f.read_own().await?.into_iter().map(|r| r.value).collect();

    assert_eq!(
        values,
        vec!["zzz.current@inputs.test", "aaa.former@inputs.test"],
        "the latest observation must lead the account's rows"
    );
    f.forget().await
}
