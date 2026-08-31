//! What the page shows, and the order it shows it in.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a broken fixture should fail the test loudly"
)]

use chrono::{DateTime, TimeZone, Utc};

use super::model::{Attention, ConnectorSummary, LastSync, LedgerFacts, SyncStatus, by_attention};
use super::model::{ConnectorHealth, ConnectorHealthResponse, SyncFact, SyncHistoryResponse};
use super::name::ConnectorName;

fn moment(offset: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + offset, 0)
        .single()
        .unwrap()
}

fn sync(status: SyncStatus, started: Option<i64>) -> LastSync {
    LastSync {
        job_id: "8412".to_owned(),
        status,
        started_at: started.map(moment),
        job_updated_at: started.map(|at| moment(at - 10)),
        duration_ms: Some(142_000),
        records_reported: Some(12_400),
    }
}

fn summary(name: &str, configured: bool, last: Option<LastSync>) -> ConnectorSummary {
    ConnectorSummary {
        connector: name.to_owned(),
        configured,
        last_sync: last,
    }
}

// ── the mover's vocabulary ─────────────────────────────────────────────────

#[test]
fn every_documented_status_survives_the_round_trip() {
    for word in [
        "pending",
        "running",
        "incomplete",
        "succeeded",
        "failed",
        "cancelled",
        "unknown",
    ] {
        assert_eq!(
            SyncStatus::parse(word).as_str(),
            word,
            "should store the mover's own word: {word}"
        );
    }
}

#[test]
fn a_word_outside_the_vocabulary_reads_as_unknown() {
    for word in ["", "SUCCEEDED", "succeeded_partially", "ok", "  running"] {
        assert_eq!(
            SyncStatus::parse(word),
            SyncStatus::Unknown,
            "should not be interpreted: {word:?}"
        );
    }
}

// ── what needs acting on ───────────────────────────────────────────────────

#[test]
fn a_sync_that_did_not_finish_its_work_needs_attention_first() {
    for status in [SyncStatus::Failed, SyncStatus::Incomplete] {
        let row = summary("a", true, Some(sync(status, Some(0))));
        assert_eq!(row.attention(), Attention::Failing, "{status:?}");
    }
}

#[test]
fn an_unreadable_state_is_not_filed_with_the_quiet_ones() {
    let row = summary("a", true, Some(sync(SyncStatus::Unknown, Some(0))));
    assert_eq!(row.attention(), Attention::Unreadable);
}

#[test]
fn an_active_or_finished_sync_is_settled() {
    for status in [
        SyncStatus::Pending,
        SyncStatus::Running,
        SyncStatus::Succeeded,
        SyncStatus::Cancelled,
    ] {
        let row = summary("a", true, Some(sync(status, Some(0))));
        assert_eq!(row.attention(), Attention::Settled, "{status:?}");
    }
}

#[test]
fn a_configured_connector_with_no_sync_has_never_run() {
    assert_eq!(summary("a", true, None).attention(), Attention::NeverRan);
}

#[test]
fn history_without_configuration_reads_as_no_longer_configured() {
    let row = summary("a", false, Some(sync(SyncStatus::Succeeded, Some(0))));
    assert_eq!(row.attention(), Attention::NoLongerConfigured);
}

#[test]
fn a_removed_connector_is_not_reported_as_failing() {
    // A connector taken out of configuration is a decision, not a fault — even
    // when the last thing it did was fail.
    let row = summary("a", false, Some(sync(SyncStatus::Failed, Some(0))));
    assert_eq!(row.attention(), Attention::NoLongerConfigured);
}

// ── the order ──────────────────────────────────────────────────────────────

#[test]
fn rows_are_ordered_worst_first() {
    let mut rows = vec![
        summary("quiet", true, Some(sync(SyncStatus::Succeeded, Some(0)))),
        summary("gone", false, Some(sync(SyncStatus::Succeeded, Some(0)))),
        summary("fresh", true, None),
        summary("murky", true, Some(sync(SyncStatus::Unknown, Some(0)))),
        summary("broken", true, Some(sync(SyncStatus::Failed, Some(0)))),
    ];
    by_attention(&mut rows);
    let order: Vec<&str> = rows.iter().map(|row| row.connector.as_str()).collect();
    assert_eq!(order, ["broken", "murky", "quiet", "fresh", "gone"]);
}

#[test]
fn inside_a_band_the_most_recent_activity_comes_first() {
    let mut rows = vec![
        summary("older", true, Some(sync(SyncStatus::Failed, Some(100)))),
        summary("newest", true, Some(sync(SyncStatus::Failed, Some(900)))),
        summary("middle", true, Some(sync(SyncStatus::Failed, Some(500)))),
    ];
    by_attention(&mut rows);
    let order: Vec<&str> = rows.iter().map(|row| row.connector.as_str()).collect();
    assert_eq!(order, ["newest", "middle", "older"]);
}

#[test]
fn a_row_with_no_activity_sorts_after_rows_that_have_some() {
    let mut rows = vec![
        summary("unstarted", true, Some(sync(SyncStatus::Failed, None))),
        summary("started", true, Some(sync(SyncStatus::Failed, Some(1)))),
    ];
    by_attention(&mut rows);
    assert_eq!(rows[0].connector, "started");
}

#[test]
fn the_order_is_stable_for_rows_that_tie() {
    let mut rows = vec![
        summary("zulu", true, None),
        summary("alpha", true, None),
        summary("mike", true, None),
    ];
    by_attention(&mut rows);
    let order: Vec<&str> = rows.iter().map(|row| row.connector.as_str()).collect();
    assert_eq!(order, ["alpha", "mike", "zulu"], "ties break on the name");
}

// ── the wire ───────────────────────────────────────────────────────────────

#[test]
fn absence_crosses_the_wire_as_absence() {
    let fact = SyncFact::from(LastSync {
        job_id: "1".to_owned(),
        status: SyncStatus::Pending,
        started_at: None,
        job_updated_at: Some(moment(0)),
        duration_ms: None,
        records_reported: None,
    });
    assert!(fact.started_at.is_none());
    assert!(fact.duration_ms.is_none());
    assert!(
        fact.records_reported.is_none(),
        "an unmeasured count must not ship as zero"
    );
}

#[test]
fn a_reported_zero_crosses_the_wire_as_zero() {
    let fact = SyncFact::from(LastSync {
        records_reported: Some(0),
        ..sync(SyncStatus::Succeeded, Some(0))
    });
    assert_eq!(fact.records_reported, Some(0));
}

#[test]
fn the_ordering_stamp_is_not_serialised() {
    // It exists to order rows, and the mover's update stamp is not something the
    // page has any use for. Serialising it would invite a client to read it as
    // the start.
    let json = serde_json::to_string(&SyncFact::from(sync(SyncStatus::Succeeded, Some(0))))
        .expect("serialisable");
    assert!(!json.contains("job_updated_at"), "{json}");
}

#[test]
fn a_configured_connector_that_never_synced_ships_a_null_last_sync() {
    let health = ConnectorHealth::from(summary("a", true, None));
    assert!(health.configured);
    assert!(health.last_sync.is_none());
}

#[test]
fn stamps_are_rfc3339_in_utc_with_millis() {
    assert_eq!(super::model::stamp(moment(0)), "2023-11-14T22:13:20.000Z");
}

// ── the name at the boundary ───────────────────────────────────────────────

#[test]
fn a_descriptor_style_name_parses() {
    for name in ["jira", "claude-team", "git-gitlab", "a1"] {
        assert!(ConnectorName::parse(name).is_some(), "should accept {name}");
    }
}

#[test]
fn anything_that_is_not_a_connector_name_is_refused() {
    let long = "a".repeat(65);
    for name in [
        "",
        "-leading",
        "trailing-",
        "Upper",
        "with_underscore",
        "with space",
        "quote'; DROP",
        "../etc",
        long.as_str(),
    ] {
        assert!(
            ConnectorName::parse(name).is_none(),
            "should refuse {name:?}"
        );
    }
}

#[test]
fn an_underscore_is_refused_because_the_bronze_mapping_needs_it() {
    assert!(ConnectorName::parse("ai_dev").is_none());
}

// ── the answer the page receives ───────────────────────────────────────────

#[test]
fn the_answer_carries_two_clocks_and_the_measured_interval() {
    let response = ConnectorHealthResponse::from_facts(
        LedgerFacts {
            sealed_at: Some(moment(0)),
            typical_read_interval_ms: Some(900_000),
            summaries: vec![summary(
                "a",
                true,
                Some(sync(SyncStatus::Succeeded, Some(0))),
            )],
            has_history: true,
        },
        moment(600),
    );
    assert_eq!(response.as_of, super::model::stamp(moment(600)));
    assert_eq!(
        response.checked_at.as_deref(),
        Some("2023-11-14T22:13:20.000Z")
    );
    assert_eq!(response.typical_read_interval_ms, Some(900_000));
    assert!(response.history_available);
    assert_eq!(response.connectors.len(), 1);
}

#[test]
fn an_empty_ledger_answers_without_a_checked_at() {
    let response = ConnectorHealthResponse::from_facts(LedgerFacts::default(), moment(0));
    assert!(response.checked_at.is_none(), "nothing has been read yet");
    assert!(response.typical_read_interval_ms.is_none());
    assert!(!response.history_available);
    assert!(response.connectors.is_empty());
    assert!(!response.as_of.is_empty(), "the answer still dates itself");
}

#[test]
fn the_answer_keeps_the_order_it_was_given() {
    // The service sorts by what needs acting on; the page must not have to
    // re-derive that, so the order it receives is the order it renders.
    let mut summaries = vec![
        summary("quiet", true, Some(sync(SyncStatus::Succeeded, Some(0)))),
        summary("broken", true, Some(sync(SyncStatus::Failed, Some(0)))),
    ];
    by_attention(&mut summaries);
    let response = ConnectorHealthResponse::from_facts(
        LedgerFacts {
            summaries,
            has_history: true,
            ..LedgerFacts::default()
        },
        moment(0),
    );
    let order: Vec<&str> = response
        .connectors
        .iter()
        .map(|row| row.connector.as_str())
        .collect();
    assert_eq!(order, ["broken", "quiet"]);
}

#[test]
fn the_window_says_how_large_it_is() {
    let response = SyncHistoryResponse::build(
        "example-tracker".to_owned(),
        vec![sync(SyncStatus::Succeeded, Some(0))],
        50,
    );
    assert_eq!(response.connector, "example-tracker");
    assert_eq!(response.window, 50, "the page can say the list is a window");
    assert_eq!(response.syncs.len(), 1);
}

#[test]
fn a_connector_with_no_recorded_sync_answers_an_empty_window() {
    let response = SyncHistoryResponse::build("nobody".to_owned(), Vec::new(), 50);
    assert!(response.syncs.is_empty());
    assert_eq!(response.window, 50, "still a window, just an empty one");
}

// ── the name at the boundary, both ways ────────────────────────────────────

#[test]
fn a_parsed_name_round_trips_unchanged() {
    let parsed = ConnectorName::parse("claude-team").expect("a valid name");
    assert_eq!(parsed.as_str(), "claude-team");
    assert_eq!(parsed.into_string(), "claude-team");
}
