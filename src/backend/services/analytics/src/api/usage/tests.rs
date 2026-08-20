use super::*;

fn record(name: &str, data: serde_json::Value) -> TelemetryRecord {
    TelemetryRecord {
        name: Some(name.to_owned()),
        data: Some(data),
        ..TelemetryRecord::default()
    }
}

#[test]
fn a_value_survives_the_sdk_field_stringification() {
    let stringified = serde_json::json!({ "path": "\"/people\"" });
    assert_eq!(data_field(Some(&stringified), "path"), "/people");

    let plain = serde_json::json!({ "target": "pr_cycle_time" });
    assert_eq!(data_field(Some(&plain), "target"), "pr_cycle_time");
}

#[test]
fn a_record_without_data_has_no_path() {
    assert_eq!(data_field(None, "path"), "");
    assert_eq!(data_field(Some(&serde_json::json!({})), "path"), "");
}

#[test]
fn the_session_decides_identity_not_the_payload() {
    let tenant = Uuid::now_v7();
    let person = Uuid::now_v7();
    let row = to_row(
        &record("drill", serde_json::json!({ "target": "pr_cycle_time" })),
        &TelemetryRecord::default(),
        tenant,
        person,
        Utc::now(),
    );
    assert_eq!(row.tenant_id, tenant);
    assert_eq!(row.person_id, person);
    assert_eq!(row.target, "pr_cycle_time");
}

#[test]
fn a_field_the_batch_shares_is_read_from_the_hoisted_meta() {
    let hoisted = TelemetryRecord {
        name: Some(PAGE_VIEW.to_owned()),
        context_session_id: Some("s-1".to_owned()),
        ..TelemetryRecord::default()
    };
    let sent = TelemetryRecord {
        data: Some(serde_json::json!({ "path": "/portal/manage" })),
        ..TelemetryRecord::default()
    };

    let row = to_row(&sent, &hoisted, Uuid::now_v7(), Uuid::now_v7(), Utc::now());
    assert_eq!(row.event_name, PAGE_VIEW);
    assert_eq!(row.session_id, "s-1");
    assert_eq!(row.path, "/portal/manage");
}

#[test]
fn a_records_own_value_beats_the_hoisted_one() {
    let hoisted = TelemetryRecord {
        name: Some(PAGE_VIEW.to_owned()),
        ..TelemetryRecord::default()
    };

    let row = to_row(
        &record("drill", serde_json::json!({ "target": "pr_cycle_time" })),
        &hoisted,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Utc::now(),
    );
    assert_eq!(row.event_name, "drill");
}

#[test]
fn every_field_the_caller_controls_is_bounded() {
    let long = "x".repeat(4096);
    let mut sent = record(
        "page_view",
        serde_json::json!({ "path": long.clone(), "target": long.clone() }),
    );
    sent.name = Some(long.clone());
    sent.context_session_id = Some(long.clone());
    sent.context_app_name = Some(long.clone());
    sent.context_app_version = Some(long);

    let row = to_row(
        &sent,
        &TelemetryRecord::default(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Utc::now(),
    );
    assert_eq!(row.event_name.chars().count(), MAX_NAME);
    assert_eq!(row.app_name.chars().count(), MAX_NAME);
    assert_eq!(row.session_id.chars().count(), MAX_FIELD);
    assert_eq!(row.app_version.chars().count(), MAX_FIELD);
    assert_eq!(row.path.chars().count(), MAX_PATH);
    assert_eq!(row.target.chars().count(), MAX_PATH);
}

#[test]
fn a_hoisted_field_is_bounded_before_the_batch_multiplies_it() {
    // `meta` is one object cloned into every record, so an unclipped field
    // there costs MAX_RECORDS times its own size.
    let hoisted = TelemetryRecord {
        context_app_name: Some("x".repeat(4096)),
        ..TelemetryRecord::default()
    };
    let row = to_row(
        &TelemetryRecord::default(),
        &hoisted,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Utc::now(),
    );
    assert_eq!(row.app_name.chars().count(), MAX_NAME);
}

#[test]
fn the_widest_window_is_the_one_the_message_promises() {
    let window = |since: &str, until: &str| {
        UsageRangeQuery {
            since: Some(since.to_owned()),
            until: Some(until.to_owned()),
        }
        .window()
    };
    // Both bounds are inclusive, so 400 days spans since..=since+399.
    assert!(window("2026-01-01", "2027-02-04").is_ok(), "400 days");
    assert!(window("2026-01-01", "2027-02-05").is_err(), "401 days");
}

#[test]
fn the_actions_breakdown_leaves_out_what_nobody_did() {
    assert!(actions_sql("1").contains("NOT IN ('page_view', 'session_start')"));
}

#[test]
fn the_sdks_own_page_view_is_dropped_as_a_duplicate() {
    let row = |data: serde_json::Value, name: &str| {
        to_row(
            &record(name, data),
            &TelemetryRecord::default(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Utc::now(),
        )
    };
    assert!(!is_recordable(&row(
        serde_json::json!({ "url": "/portal" }),
        PAGE_VIEW
    )));
    assert!(is_recordable(&row(
        serde_json::json!({ "path": "/portal" }),
        PAGE_VIEW
    )));
    assert!(
        is_recordable(&row(
            serde_json::json!({ "target": "pr_cycle_time" }),
            "drill"
        )),
        "an action carries no path"
    );
}

#[test]
fn every_caller_value_in_a_read_is_a_placeholder() {
    let visitors = "V";
    assert_eq!(WINDOW.matches('?').count(), 3);
    for sql in [
        totals_sql(visitors),
        by_day_sql(visitors),
        by_page_sql(visitors),
        actions_sql(visitors),
    ] {
        assert_eq!(
            sql.matches('?').count(),
            3,
            "a read that does not bind exactly the window has interpolated a value: {sql}"
        );
    }
    assert_eq!(
        people_sql().matches('?').count(),
        4,
        "people_query binds the tenant a second time for the identity join"
    );
}

#[test]
fn a_visitor_is_named_from_the_mirrored_identity_rows() {
    let sql = people_sql();
    assert!(sql.contains("identity.identity_persons"), "{sql}");
    assert!(sql.contains("display_name"), "{sql}");
}

#[test]
fn a_malformed_day_is_refused_rather_than_queried() {
    let query = UsageRangeQuery {
        since: Some("2026-99-99".to_owned()),
        until: None,
    };
    assert!(query.window().is_err(), "a date that cannot exist is a 400");

    let ok = UsageRangeQuery {
        since: Some("2026-01-31".to_owned()),
        until: Some("2026-02-01".to_owned()),
    };
    assert_eq!(
        ok.window().ok().map(|w| w.since.to_string()),
        Some("2026-01-31".to_owned())
    );
}

#[test]
fn a_clock_hours_out_still_places_the_event_correctly() {
    let arrival = Utc::now();
    let skew = 3 * 60 * 60 * 1000;
    let buffered = 90_000;

    let mut event = record("page_view", serde_json::json!({ "path": "/portal" }));
    event.time_triggered = Some(arrival.timestamp_millis() + skew - buffered);
    event.time_sent = Some(arrival.timestamp_millis() + skew);

    let row = to_row(
        &event,
        &TelemetryRecord::default(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        arrival,
    );

    assert_eq!(row.ts, arrival - Duration::milliseconds(buffered));
}

#[test]
fn two_events_flushed_together_keep_their_own_times() {
    let arrival = Utc::now();
    let meta = TelemetryRecord::default();
    let sent = arrival.timestamp_millis();

    let mut first = record("page_view", serde_json::json!({ "path": "/a" }));
    first.time_triggered = Some(sent - 120_000);
    first.time_sent = Some(sent);
    let mut second = record("page_view", serde_json::json!({ "path": "/b" }));
    second.time_triggered = Some(sent - 1_000);
    second.time_sent = Some(sent);

    let rows = [
        to_row(&first, &meta, Uuid::now_v7(), Uuid::now_v7(), arrival),
        to_row(&second, &meta, Uuid::now_v7(), Uuid::now_v7(), arrival),
    ];

    assert_eq!(rows[0].ts, arrival - Duration::milliseconds(120_000));
    assert_eq!(rows[1].ts, arrival - Duration::milliseconds(1_000));
}

#[test]
fn a_record_with_no_browser_time_falls_back_to_arrival() {
    let arrival = Utc::now();

    let row = to_row(
        &record("page_view", serde_json::json!({ "path": "/portal" })),
        &TelemetryRecord::default(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        arrival,
    );

    assert_eq!(row.ts, arrival);
}

#[test]
fn a_clock_that_ran_backwards_is_not_trusted() {
    let arrival = Utc::now();
    let mut event = record("page_view", serde_json::json!({ "path": "/portal" }));
    event.time_triggered = Some(arrival.timestamp_millis());
    event.time_sent = Some(arrival.timestamp_millis() - 5_000);

    let row = to_row(
        &event,
        &TelemetryRecord::default(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        arrival,
    );

    assert_eq!(row.ts, arrival);
}

#[test]
fn an_event_held_longer_than_a_day_is_not_trusted() {
    let arrival = Utc::now();
    let sent = arrival.timestamp_millis();
    let mut event = record("page_view", serde_json::json!({ "path": "/portal" }));
    event.time_triggered = Some(sent - MAX_BUFFERED_MS - 1);
    event.time_sent = Some(sent);

    let row = to_row(
        &event,
        &TelemetryRecord::default(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        arrival,
    );

    assert_eq!(row.ts, arrival);
}

#[test]
fn a_batch_whose_flush_stamp_was_hoisted_into_meta_keeps_its_own_times() {
    let arrival = Utc::now();
    let sent = arrival.timestamp_millis();

    let meta = TelemetryRecord {
        time_sent: Some(sent),
        ..TelemetryRecord::default()
    };

    let mut first = record("session_start", serde_json::json!({}));
    first.time_triggered = Some(sent - 4_000);
    let mut second = record("page_view", serde_json::json!({ "path": "/portal" }));
    second.time_triggered = Some(sent - 1_000);

    let rows = [
        to_row(&first, &meta, Uuid::now_v7(), Uuid::now_v7(), arrival),
        to_row(&second, &meta, Uuid::now_v7(), Uuid::now_v7(), arrival),
    ];

    assert_eq!(rows[0].ts, arrival - Duration::milliseconds(4_000));
    assert_eq!(rows[1].ts, arrival - Duration::milliseconds(1_000));
}

#[test]
fn a_batch_keeps_every_record_the_sdks_duplicate_rides_with() {
    let tenant = Uuid::now_v7();
    let person = Uuid::now_v7();
    // Neither page view carries its own name: the SDK hoists the one they share.
    let req = UsageIngestRequest {
        meta: TelemetryRecord {
            name: Some("page_view".to_owned()),
            context_session_id: Some("session-1".to_owned()),
            ..TelemetryRecord::default()
        },
        records: vec![
            TelemetryRecord {
                data: Some(serde_json::json!({ "path": "\"/ic/:id/team\"" })),
                ..TelemetryRecord::default()
            },
            TelemetryRecord {
                data: Some(serde_json::json!({
                    "url": "\"/ic/bbbbbbbb-0000-0000-0000-000000010001/team\""
                })),
                ..TelemetryRecord::default()
            },
            record(
                "drill",
                serde_json::json!({ "target": "git.commits", "path": "\"/ic/:id/team\"" }),
            ),
        ],
    };

    let rows = recordable_rows(&req, tenant, person, Utc::now());

    let kept: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row.event_name.as_str(), row.path.as_str()))
        .collect();
    assert_eq!(
        kept,
        vec![("page_view", "/ic/:id/team"), ("drill", "/ic/:id/team")]
    );
    assert!(
        rows.iter().all(|row| !row.path.contains("bbbbbbbb")),
        "a raw person id must never reach a stored row"
    );
}
