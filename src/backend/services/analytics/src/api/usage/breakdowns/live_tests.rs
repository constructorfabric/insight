#![expect(
    clippy::expect_used,
    reason = "a live-server setup failure should fail the test loudly"
)]

use chrono::Utc;
use uuid::Uuid;

use super::super::WindowBinds;
use super::super::search::PersonSearch;
use super::super::sort::Order;
use super::{UsagePerson, people_query};

const URL_VAR: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";

const USAGE_MIGRATION: &str = include_str!(
    "../../../../../../../ingestion/scripts/migrations/20260816000000_usage-events.sql"
);

const PERSONS_MACRO: &str =
    include_str!("../../../../../../../ingestion/dbt/macros/create_identity_persons.sql");

const ROSTER: &str = "bamboohr";

type R = Result<(), Box<dyn std::error::Error>>;

fn client_or_skip() -> Option<insight_clickhouse::Client> {
    let url = std::env::var(URL_VAR).unwrap_or_default();
    if url.is_empty() {
        eprintln!("skipping: {URL_VAR} not set");
        return None;
    }

    let mut config = insight_clickhouse::Config::new(url, "default");
    if let (Ok(user), Ok(password)) = (
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER"),
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD"),
    ) && !user.is_empty()
    {
        config = config.with_auth(user, password);
    }

    Some(insight_clickhouse::Client::new(config))
}

fn persons_ddl() -> String {
    let start = PERSONS_MACRO
        .find("CREATE TABLE")
        .expect("the macro creates the persons journal");
    let end = PERSONS_MACRO
        .find("ENGINE")
        .expect("the macro names an engine");

    format!(
        "{} ENGINE = MergeTree ORDER BY id",
        &PERSONS_MACRO[start..end]
    )
}

async fn create_relations(ch: &insight_clickhouse::Client) {
    for statement in [
        "CREATE DATABASE IF NOT EXISTS identity".to_owned(),
        "CREATE DATABASE IF NOT EXISTS product_usage".to_owned(),
        persons_ddl(),
    ] {
        ch.query(&statement)
            .execute()
            .await
            .unwrap_or_else(|error| panic!("setup failed: {error}\n{statement}"));
    }

    let migration: String = USAGE_MIGRATION
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    for statement in migration.split(';').filter(|s| !s.trim().is_empty()) {
        ch.query(statement)
            .execute()
            .await
            .unwrap_or_else(|error| panic!("migration failed: {error}\n{statement}"));
    }
}

struct Observation {
    person: Uuid,
    source: &'static str,
    value_type: &'static str,
    value: &'static str,
    recorded: &'static str,
}

async fn record(ch: &insight_clickhouse::Client, tenant: Uuid, observations: &[Observation]) {
    for (id, observation) in observations.iter().enumerate() {
        ch.query(
            "INSERT INTO identity.identity_persons \
             (id, value_type, insight_source_type, insight_source_id, insight_tenant_id, \
              value, value_effective, person_id, author_person_id, created_at, _synced_at) \
             VALUES (?, ?, ?, generateUUIDv4(), toUUID(?), ?, ?, toUUID(?), toUUID(?), \
                     toDateTime64(?, 6, 'UTC'), now64(3))",
        )
        .bind(id)
        .bind(observation.value_type)
        .bind(observation.source)
        .bind(tenant.to_string())
        .bind(observation.value)
        .bind(observation.value)
        .bind(observation.person.to_string())
        .bind(Uuid::nil().to_string())
        .bind(observation.recorded)
        .execute()
        .await
        .expect("insert an identity observation");
    }
}

async fn visit(ch: &insight_clickhouse::Client, tenant: Uuid, person: Uuid) {
    ch.query(
        "INSERT INTO product_usage.usage_events \
         (tenant_id, person_id, session_id, event_name, path, target, app_name, app_version) \
         VALUES (toUUID(?), toUUID(?), 's-1', 'page_view', '/people', '', 'insight', '0.0.0')",
    )
    .bind(tenant.to_string())
    .bind(person.to_string())
    .execute()
    .await
    .expect("insert a usage event");
}

fn window(tenant: Uuid) -> WindowBinds {
    let today = Utc::now().date_naive().to_string();

    WindowBinds {
        tenant: tenant.to_string(),
        since: today.clone(),
        until: today,
    }
}

async fn named(
    ch: &insight_clickhouse::Client,
    binds: &WindowBinds,
    search: Option<&PersonSearch>,
) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = people_query(ch, binds, ROSTER, Order::default(), search)
        .fetch_all::<UsagePerson>()
        .await
        .expect("read the people list")
        .into_iter()
        .map(|person| (person.person_id, person.display_name))
        .collect();

    rows.sort();
    rows
}

#[tokio::test]
#[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn a_person_on_the_roster_is_named_by_the_roster_whatever_another_source_said_since() -> R {
    let Some(ch) = client_or_skip() else {
        return Ok(());
    };
    create_relations(&ch).await;

    let tenant = Uuid::now_v7();
    let rostered = Uuid::now_v7();
    let unrostered = Uuid::now_v7();
    let rostered_by_parts = Uuid::now_v7();
    let observations = [
        Observation {
            person: rostered,
            source: ROSTER,
            value_type: "display_name",
            value: "Ada Example",
            recorded: "2026-01-01 00:00:00",
        },
        Observation {
            person: rostered,
            source: "youtrack",
            value_type: "display_name",
            value: "ada",
            recorded: "2026-02-01 00:00:00",
        },
        Observation {
            person: unrostered,
            source: "gitlab",
            value_type: "display_name",
            value: "Bob Sample",
            recorded: "2026-01-01 00:00:00",
        },
        Observation {
            person: unrostered,
            source: "youtrack",
            value_type: "display_name",
            value: "bob",
            recorded: "2026-02-01 00:00:00",
        },
        Observation {
            person: rostered_by_parts,
            source: ROSTER,
            value_type: "first_name",
            value: "Cy",
            recorded: "2026-01-01 00:00:00",
        },
        Observation {
            person: rostered_by_parts,
            source: ROSTER,
            value_type: "last_name",
            value: "Placeholder",
            recorded: "2026-01-01 00:00:00",
        },
        Observation {
            person: rostered_by_parts,
            source: "youtrack",
            value_type: "display_name",
            value: "cy",
            recorded: "2026-02-01 00:00:00",
        },
    ];
    record(&ch, tenant, &observations).await;
    for person in [rostered, unrostered, rostered_by_parts] {
        visit(&ch, tenant, person).await;
    }
    let binds = window(tenant);

    let mut expected = vec![
        (rostered.to_string(), "Ada Example".to_owned()),
        (unrostered.to_string(), "bob".to_owned()),
        (rostered_by_parts.to_string(), "Cy Placeholder".to_owned()),
    ];
    expected.sort();
    assert_eq!(named(&ch, &binds, None).await, expected);

    let search = PersonSearch::parse(Some("Example"))?.ok_or("a real search")?;
    assert_eq!(
        named(&ch, &binds, Some(&search)).await,
        vec![(rostered.to_string(), "Ada Example".to_owned())],
        "the search should match the name the list shows"
    );
    Ok(())
}
