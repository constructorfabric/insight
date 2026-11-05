use chrono::{Duration, Utc};

use super::{Error, sync_history};

type R = Result<(), Box<dyn std::error::Error>>;

pub(in crate::domain) async fn completed_passes_collapse_ticks_and_include_only_the_cutoff(
    ch: &insight_clickhouse::Client,
) -> R {
    ch.query("TRUNCATE TABLE ingestion_history.sync_events")
        .execute()
        .await?;

    let cutoff = chrono::SubsecRound::trunc_subsecs(Utc::now(), 3);
    let first = cutoff - Duration::minutes(2);
    let duplicate = cutoff - Duration::minutes(1);
    for (tick, at) in [
        ("tick-old", first),
        ("tick-old", duplicate),
        ("tick-cutoff", cutoff),
        ("tick-later", cutoff + Duration::milliseconds(1)),
    ] {
        ch.query(
            "INSERT INTO ingestion_history.sync_events (ts, tick_id, event) \
             VALUES (fromUnixTimestamp64Milli(?), ?, 'sweep.completed')",
        )
        .bind(at.timestamp_millis())
        .bind(tick)
        .execute()
        .await?;
    }

    assert_eq!(
        sync_history::completed_passes(ch, cutoff).await?,
        [cutoff, duplicate]
    );
    assert_eq!(
        sync_history::completed_passes(ch, cutoff - Duration::milliseconds(1)).await?,
        [duplicate]
    );
    assert!(
        sync_history::completed_passes(ch, first - Duration::milliseconds(1))
            .await?
            .is_empty()
    );

    ch.query("TRUNCATE TABLE ingestion_history.sync_events")
        .execute()
        .await?;
    Ok(())
}

pub(in crate::domain) async fn schedules_follow_the_latest_completed_tick(
    ch: &insight_clickhouse::Client,
) -> R {
    ch.query("TRUNCATE TABLE ingestion_history.sync_events")
        .execute()
        .await?;

    let now = Utc::now();
    let older_ms = (now - Duration::minutes(30)).timestamp_millis();
    let configured_ms = (now - Duration::minutes(20)).timestamp_millis();
    let pass_completed_ms = (now - Duration::minutes(10)).timestamp_millis();
    let incomplete_pass_ms = (now - Duration::minutes(5)).timestamp_millis();

    ch.query(
        "INSERT INTO ingestion_history.sync_events \
         (ts, tick_id, connector, tenant_id, source_id, event, schedules, \
          schedule_timezone, schedule_origin, schedule_suspended) VALUES \
         (fromUnixTimestamp64Milli(?), 'tick-old', 'example-tracker', 'tenant-under-test', 'main', \
          'connector.configured', ['0 1 * * *'], 'UTC', 'descriptor', false), \
         (fromUnixTimestamp64Milli(?), 'tick-old', 'example-tracker', 'tenant-under-test', 'old-only', \
          'connector.configured', ['0 2 * * *'], 'UTC', 'descriptor', false), \
         (fromUnixTimestamp64Milli(?), 'tick-completed', 'example-tracker', 'tenant-under-test', 'main', \
          'connector.configured', ['0 5 * * *', '0 17 * * *'], 'UTC', 'api', true), \
         (fromUnixTimestamp64Milli(?), 'tick-completed', 'example-tracker', 'another-tenant', 'second', \
          'connector.configured', ['0 7 * * *'], '', 'annotation', false), \
         (fromUnixTimestamp64Milli(?), 'tick-open', 'example-tracker', 'tenant-under-test', 'main', \
          'connector.configured', ['0 23 * * *'], 'UTC', 'descriptor', false), \
         (fromUnixTimestamp64Milli(?), 'tick-open', 'example-tracker', 'tenant-under-test', 'open-only', \
          'connector.configured', ['0 23 * * *'], 'UTC', 'descriptor', false), \
         (fromUnixTimestamp64Milli(?), 'tick-completed', '', '', '', 'sweep.completed', [], '', '', false), \
         (fromUnixTimestamp64Milli(?), 'tick-old', '', '', '', 'sweep.completed', [], '', '', false)",
    )
    .bind(older_ms)
    .bind(older_ms)
    .bind(configured_ms)
    .bind(configured_ms)
    .bind(incomplete_pass_ms)
    .bind(incomplete_pass_ms)
    .bind(pass_completed_ms)
    .bind(older_ms)
    .execute()
    .await?;

    ch.query(
        "INSERT INTO ingestion_history.sync_events \
         (ts, tick_id, connector, tenant_id, source_id, event) VALUES \
         (fromUnixTimestamp64Milli(?), 'tick-completed', 'example-tracker', \
          'tenant-under-test', 'no-schedule', 'connector.configured')",
    )
    .bind(configured_ms)
    .execute()
    .await?;

    ch.query(
        "ALTER TABLE ingestion_history.sync_events UPDATE inherited_schedules = ['0 2 * * *'], \
         inherited_schedule_origin = 'annotation' WHERE tick_id = 'tick-completed' AND source_id = 'main' \
         SETTINGS mutations_sync = 1",
    ).execute().await?;

    for (source, crons, origin, suspended) in [
        ("main", &["0 5 * * *", "0 17 * * *"][..], "api", true),
        ("second", &["0 7 * * *"][..], "annotation", false),
    ] {
        let schedule = sync_history::applied_schedule(ch, "example-tracker", source)
            .await?
            .ok_or_else(|| format!("missing applied schedule for {source}"))?;
        assert_eq!(schedule.crons, crons, "{source}");
        assert_eq!(schedule.origin, origin, "{source}");
        assert_eq!(
            schedule.timezone.as_deref(),
            if source == "main" { Some("UTC") } else { None },
            "{source}"
        );
        assert_eq!(schedule.suspended, suspended, "{source}");
        assert_inherited_schedule(&schedule, source)?;
        assert_eq!(
            schedule.recorded_at.timestamp_millis(),
            pass_completed_ms,
            "{source}"
        );
    }

    for source in ["old-only", "open-only"] {
        assert!(
            matches!(
                sync_history::applied_schedule(ch, "example-tracker", source).await,
                Err(Error::NotFound { .. })
            ),
            "{source} is absent from the latest completed tick"
        );
    }
    assert!(
        sync_history::applied_schedule(ch, "example-tracker", "no-schedule")
            .await?
            .is_none()
    );

    ch.query("TRUNCATE TABLE ingestion_history.sync_events")
        .execute()
        .await?;
    Ok(())
}

fn assert_inherited_schedule(schedule: &super::dto::RecordedSchedule, source: &str) -> R {
    let inherited = serde_json::to_value(&schedule.inherited)?;
    assert_eq!(
        inherited,
        if source == "main" {
            serde_json::json!({"crons": ["0 2 * * *"], "origin": "annotation"})
        } else {
            serde_json::Value::Null
        },
        "{source}"
    );

    Ok(())
}
