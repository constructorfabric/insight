//! Unit contract for the ingestion-intensity read.
//!
//! Everything here is pure: grain/series/scope parsing, window resolution and
//! the emitted SQL shape. The admin gate and the ClickHouse round-trip belong
//! to the stand suite (`tests/stand/api/`), which has a real identity service
//! and a real view to read.
//!
//! Fallible calls are compared through `.ok()` rather than unwrapped — the
//! workspace denies `unwrap_used` / `expect_used`, and `Option<T>` equality
//! asserts the value and the success in one line.

use chrono::{DateTime, Duration, TimeZone as _, Utc};

use super::{Grain, MAX_POINTS, Series, Window, intensity_sql, parse_scope};

/// Fixed instants for the window tests. `single()` cannot be `None` for the
/// literals below; defaulting instead of unwrapping keeps the lint config
/// satisfied, and `the_instant_helper_is_honest` pins that it never does.
fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .unwrap_or_default()
}

fn noon() -> DateTime<Utc> {
    at(2026, 8, 26, 12, 0)
}

#[test]
fn the_instant_helper_is_honest() {
    assert_eq!(noon().to_rfc3339(), "2026-08-26T12:00:00+00:00");
}

#[test]
fn grain_defaults_to_fifteen_minutes() {
    assert_eq!(Grain::parse(None).ok(), Some(Grain::FifteenMinutes));
}

#[test]
fn grain_set_is_closed() {
    assert_eq!(Grain::parse(Some("1s")).ok(), Some(Grain::Second));
    // An interval expression reaching the merge() scan is the thing the closed
    // set exists to prevent.
    for rejected in ["1m", "INTERVAL 1 DAY", "15", "", "15M"] {
        assert!(Grain::parse(Some(rejected)).is_err(), "accepted {rejected}");
    }
}

#[test]
fn series_default_follows_the_scope() {
    assert_eq!(Series::parse(None, false).ok(), Some(Series::Connector));
    assert_eq!(Series::parse(None, true).ok(), Some(Series::Stream));
}

#[test]
fn series_set_is_closed() {
    assert_eq!(
        Series::parse(Some("total"), false).ok(),
        Some(Series::Total)
    );
    assert!(Series::parse(Some("database"), false).is_err());
}

#[test]
fn scope_must_be_a_bronze_slug() {
    assert_eq!(
        parse_scope(Some("bronze_bamboohr")).ok(),
        Some(Some("bronze_bamboohr".to_owned())),
    );
    assert_eq!(parse_scope(None).ok(), Some(None));
    // An empty value is what an unset query param serialises to.
    assert_eq!(parse_scope(Some("")).ok(), Some(None));
}

#[test]
fn scope_rejects_anything_that_could_widen_the_scan() {
    for rejected in [
        "silver_jira",
        "bronze_jira; DROP TABLE x",
        "bronze_*",
        "bronze_Jira",
        "BRONZE_jira",
        "bronze_jira'",
        "bronze_jira`",
        "bronze_jira ",
        "jira",
    ] {
        assert!(parse_scope(Some(rejected)).is_err(), "accepted {rejected}");
    }
}

#[test]
fn window_defaults_a_day_back_at_fifteen_minutes() {
    let now = noon();
    assert_eq!(
        Window::resolve(None, None, Grain::FifteenMinutes, now).ok(),
        Some(Window {
            from: now - Duration::days(1),
            to: now,
        }),
    );
}

#[test]
fn window_defaults_thirty_minutes_back_at_one_second() {
    let now = noon();
    assert_eq!(
        Window::resolve(None, None, Grain::Second, now).ok(),
        Some(Window {
            from: now - Duration::minutes(30),
            to: now,
        }),
    );
}

#[test]
fn window_normalises_a_non_utc_offset() {
    // The SPA sends UTC, but a hand-edited link need not: an offset bound must
    // land on the same instant the server would have picked, not the same digits.
    assert_eq!(
        Window::resolve(
            Some("2026-08-26T09:00:00+02:00"),
            Some("2026-08-26T10:00:00+02:00"),
            Grain::FifteenMinutes,
            noon(),
        )
        .ok(),
        Some(Window {
            from: at(2026, 8, 26, 7, 0),
            to: at(2026, 8, 26, 8, 0),
        }),
    );
}

#[test]
fn window_rejects_an_inverted_pair() {
    assert!(
        Window::resolve(
            Some("2026-08-26T11:00:00Z"),
            Some("2026-08-26T10:00:00Z"),
            Grain::FifteenMinutes,
            noon(),
        )
        .is_err(),
    );
}

#[test]
fn window_rejects_an_empty_pair() {
    assert!(
        Window::resolve(
            Some("2026-08-26T10:00:00Z"),
            Some("2026-08-26T10:00:00Z"),
            Grain::FifteenMinutes,
            noon(),
        )
        .is_err(),
    );
}

#[test]
fn window_rejects_a_non_rfc3339_bound() {
    for rejected in ["2026-08-26", "2026-08-26 10:00:00", "yesterday", ""] {
        assert!(
            Window::resolve(Some(rejected), None, Grain::FifteenMinutes, noon()).is_err(),
            "accepted {rejected}",
        );
    }
}

#[test]
fn one_second_grain_refuses_a_window_it_cannot_bucket() {
    // A day at one-second buckets is 86_400 groups per band, past the cap
    // before any connector splits it.
    assert!(
        Window::resolve(
            Some("2026-08-25T12:00:00Z"),
            Some("2026-08-26T12:00:00Z"),
            Grain::Second,
            noon(),
        )
        .is_err(),
    );
    assert!(
        Window::resolve(
            Some("2026-08-26T11:00:00Z"),
            Some("2026-08-26T12:00:00Z"),
            Grain::Second,
            noon(),
        )
        .is_ok(),
    );
}

#[test]
fn each_grain_span_stays_under_the_group_cap() {
    // The cap that protects the merge() scan must not be reachable by a window
    // the validator accepts, or the surface truncates on a legitimate ask.
    let cap = i64::try_from(MAX_POINTS).unwrap_or(i64::MAX);
    let buckets =
        |grain: Grain, seconds_per_bucket: i64| grain.max_span().num_seconds() / seconds_per_bucket;
    assert!(buckets(Grain::FifteenMinutes, 15 * 60) < cap);
    assert!(buckets(Grain::Second, 1) < cap);
}

#[test]
fn sql_buckets_and_bands_per_the_request() {
    let org_wide = intensity_sql(Grain::FifteenMinutes, Series::Connector, false);
    assert!(org_wide.contains("toStartOfInterval(extracted_at, INTERVAL 15 MINUTE)"));
    assert!(org_wide.contains("connector AS `key`"));
    assert!(org_wide.contains("insight.bronze_insert_events"));

    let live = intensity_sql(Grain::Second, Series::Stream, true);
    assert!(live.contains("toStartOfSecond(extracted_at)"));
    assert!(live.contains("stream AS `key`"));

    let total = intensity_sql(Grain::FifteenMinutes, Series::Total, false);
    assert!(total.contains("'all' AS `key`"));
}

#[test]
fn sql_scopes_only_when_asked() {
    let scoped = intensity_sql(Grain::FifteenMinutes, Series::Stream, true);
    let org_wide = intensity_sql(Grain::FifteenMinutes, Series::Connector, false);
    // The scope predicate names source_database, which maps to merge()'s
    // _database virtual column and prunes non-matching tables before any read.
    assert!(scoped.contains("AND source_database = ?"));
    assert!(!org_wide.contains("source_database"));
    // Bind arity must match the clause count, or the driver shifts the window.
    assert_eq!(scoped.matches('?').count(), 3);
    assert_eq!(org_wide.matches('?').count(), 2);
}

#[test]
fn sql_never_deduplicates() {
    // Duplicate physical rows are the signal: a FINAL here would silently
    // convert insert intensity into logical row counts.
    let sql = intensity_sql(Grain::FifteenMinutes, Series::Connector, false);
    assert!(!sql.contains("FINAL"));
    assert!(!sql.contains("LIMIT 1 BY"));
}

#[test]
fn sql_fetches_one_row_past_the_cap() {
    // The extra row is how `truncated` is detected without a second scan.
    let sql = intensity_sql(Grain::FifteenMinutes, Series::Connector, false);
    assert!(sql.ends_with(&format!("LIMIT {}", MAX_POINTS + 1)));
}

/// The refusal and failure envelopes.
///
/// Both are what a caller actually receives, so their shape is contract rather
/// than detail: the refusal has to be actionable, and the read failure must not
/// hand the caller anything about the warehouse behind it.
mod envelopes {
    use axum::http::StatusCode;
    use toolkit_canonical_errors::Problem;

    use super::super::{admin_only, read_error};

    fn problem(error: toolkit_canonical_errors::CanonicalError) -> Option<serde_json::Value> {
        serde_json::to_value(Problem::from(error)).ok()
    }

    #[test]
    fn the_refusal_says_what_is_missing() {
        let envelope = problem(admin_only()).unwrap_or_default();
        assert_eq!(envelope["status"], u16::from(StatusCode::FORBIDDEN));
        // `detail` is the category's generic sentence; what names the missing
        // grant is `context.reason`, which is where a caller has to look.
        assert!(
            envelope["context"]["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("admin")),
            "a caller cannot act on this refusal: {envelope}"
        );
    }

    #[test]
    fn a_read_failure_leaks_nothing_about_the_warehouse() {
        // The message names a database, a table and a host — exactly what must
        // not travel to a caller. It belongs in the server log only.
        let secret = "Code: 60. DB::Exception: Table bronze_x.y does not exist on ch-node-3";
        let envelope = problem(read_error(clickhouse::error::Error::Custom(
            secret.to_owned(),
        )))
        .unwrap_or_default();

        assert_eq!(
            envelope["status"],
            u16::from(StatusCode::INTERNAL_SERVER_ERROR)
        );
        let wire = envelope.to_string();
        for fragment in ["bronze_x", "ch-node-3", "DB::Exception", "Code: 60"] {
            assert!(
                !wire.contains(fragment),
                "{fragment} reached the caller: {wire}"
            );
        }
    }
}

/// Round-trips of the two closed sets. The response echoes these strings back,
/// so a name that did not survive the round trip would be echoed wrong.
#[test]
fn every_grain_and_series_round_trips_through_its_own_name() {
    for grain in [Grain::FifteenMinutes, Grain::Second] {
        assert_eq!(Grain::parse(Some(grain.as_str())).ok(), Some(grain));
    }
    for series in [Series::Connector, Series::Stream, Series::Total] {
        assert_eq!(
            Series::parse(Some(series.as_str()), false).ok(),
            Some(series)
        );
    }
}

#[test]
fn the_echoed_window_is_the_shape_the_chart_parses_back() {
    // Millisecond RFC 3339 with a `Z`: the SPA feeds these two strings straight
    // to Date.parse for the axis domain.
    assert_eq!(
        Window::bound(at(2026, 8, 26, 14, 30)),
        "2026-08-26T14:30:00.000Z",
    );
}
