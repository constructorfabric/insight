use super::{
    MIGRATIONS, Row, Stamps, TABLE, TENANT, apply_migration, configured, insert, read_health, seal,
    summary_for,
};

pub(super) async fn configured_siblings_keep_their_own_schedules(
    ch: &insight_clickhouse::Client,
    at: &Stamps,
) {
    insert(
        ch,
        &[
            Row {
                schedules: &["0 5 * * *"],
                schedule_timezone: "UTC",
                schedule_origin: "annotation",
                schedule_suspended: true,
                ..configured("example-tracker", "tick-siblings", &at.tick_3)
            },
            Row {
                schedules: &["0 17 * * *"],
                schedule_timezone: "Europe/Berlin",
                schedule_origin: "descriptor",
                schedule_suspended: false,
                ..configured("example-tracker", "tick-siblings", &at.tick_3).of_instance("second")
            },
            seal("tick-siblings", &at.tick_3),
        ],
    )
    .await;

    let facts = read_health(ch).await.expect("read sibling schedules");

    assert_eq!(facts.summaries.len(), 2);
    for (source, expected) in [
        (
            "main",
            serde_json::json!({"crons": ["0 5 * * *"], "timezone": "UTC", "origin": "annotation", "suspended": true}),
        ),
        (
            "second",
            serde_json::json!({"crons": ["0 17 * * *"], "timezone": "Europe/Berlin", "origin": "descriptor", "suspended": false}),
        ),
    ] {
        let summary = facts
            .summaries
            .iter()
            .find(|row| row.instance.source_id == source)
            .expect("configured sibling");
        assert_eq!(summary.instance.connector, "example-tracker");
        assert!(summary.configured, "{source}");
        assert_eq!(
            serde_json::to_value(&summary.schedule).expect("serialisable schedule"),
            expected,
            "{source}"
        );
    }
}

pub(super) async fn the_newest_sealed_tick_supplies_the_schedule(
    ch: &insight_clickhouse::Client,
    at: &Stamps,
) {
    insert(
        ch,
        &[
            Row {
                schedules: &["0 5 * * *"],
                ..configured("example-tracker", "tick-a", &at.tick_1)
            },
            seal("tick-a", &at.tick_1),
            Row {
                schedules: &["0 17 * * *"],
                schedule_timezone: "UTC",
                schedule_origin: "descriptor",
                schedule_suspended: true,
                ..configured("example-tracker", "tick-b", &at.tick_2)
            },
            seal("tick-b", &at.tick_2),
            Row {
                schedules: &["0 23 * * *"],
                ..configured("example-tracker", "tick-c", &at.tick_3)
            },
        ],
    )
    .await;

    let facts = read_health(ch).await.expect("read sealed schedule");
    assert_eq!(facts.summaries.len(), 1);
    let summary = summary_for(&facts.summaries, "example-tracker");
    assert!(summary.configured);
    assert_eq!(
        serde_json::to_value(&summary.schedule).expect("serialisable schedule"),
        serde_json::json!({"crons": ["0 17 * * *"], "timezone": "UTC", "origin": "descriptor", "suspended": true})
    );
}

pub(super) async fn a_configured_row_before_the_schedule_migration_reads_as_unknown(
    ch: &insight_clickhouse::Client,
    at: &Stamps,
) {
    ch.query(&format!("DROP TABLE {TABLE}"))
        .execute()
        .await
        .expect("drop schedule table");
    apply_migration(ch, MIGRATIONS[0]).await;
    ch.query(&format!(
        "INSERT INTO {TABLE} (ts, tick_id, connector, tenant_id, source_id, event) \
         VALUES (toDateTime64('{}', 3, 'UTC'), 'tick-before-schedule', 'example-tracker', '{TENANT}', 'main', 'connector.configured')",
        at.tick_1,
    )).execute().await.expect("insert configured row before migration");
    apply_migration(ch, MIGRATIONS[1]).await;
    insert(ch, &[seal("tick-before-schedule", &at.tick_1)]).await;

    let facts = read_health(ch).await.expect("read migrated configured row");

    assert_eq!(facts.summaries.len(), 1);
    let summary = summary_for(&facts.summaries, "example-tracker");
    assert!(summary.configured);
    assert!(summary.schedule.is_none());
}
