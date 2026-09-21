use chrono::{DateTime, TimeZone as _, Utc};

use super::*;

fn utc(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, 0, 0)
        .single()
        .unwrap_or_else(|| panic!("{year}-{month}-{day}T{hour} is one instant"))
}

fn range(token: &str) -> RequestedRange {
    RequestedRange::parse(token).unwrap_or_else(|error| panic!("`{token}` parses: {error}"))
}

fn resolve(token: &str, now: DateTime<Utc>) -> Window {
    range(token)
        .resolve(now)
        .unwrap_or_else(|error| panic!("`{token}` resolves: {error}"))
}

fn bounds(window: &Window) -> Bounds {
    match window {
        Window::Unwindowed => Bounds::Unbounded,
        Window::Requested { bounds, .. } => *bounds,
    }
}

fn cap(token: &str) -> MaximumRange {
    MaximumRange::parse(token).unwrap_or_else(|error| panic!("`{token}` parses: {error}"))
}

#[test]
fn supported_tokens_choose_their_distinct_window_and_grain() {
    let now = utc(2026, 9, 10, 15);
    let cases = [
        ("PDC", utc(2026, 9, 9, 0), utc(2026, 9, 10, 0), Grain::Hour),
        ("P7D", utc(2026, 9, 3, 15), now, Grain::Day),
        ("P30D", utc(2026, 8, 11, 15), now, Grain::Day),
        ("PMC", utc(2026, 8, 1, 0), utc(2026, 9, 1, 0), Grain::Day),
        ("PQC", utc(2026, 4, 1, 0), utc(2026, 7, 1, 0), Grain::Week),
        ("P1Y", utc(2025, 9, 10, 15), now, Grain::Month),
    ];

    for (token, from, to, grain) in cases {
        let resolved = resolve(token, now);

        assert_eq!(bounds(&resolved), Bounds::Finite { from, to }, "{token}");
        assert_eq!(resolved.grain(), Some(grain), "{token}");
    }
}

#[test]
fn a_previous_day_window_is_yesterday_by_the_clock_not_by_the_newest_row() {
    let resolved = resolve("PDC", utc(2026, 9, 14, 9));

    assert_eq!(
        bounds(&resolved),
        Bounds::Finite {
            from: utc(2026, 9, 13, 0),
            to: utc(2026, 9, 14, 0),
        }
    );
}

#[test]
fn a_rolling_window_ends_now_rather_than_at_the_newest_row() {
    let now = utc(2026, 9, 14, 9);

    let resolved = resolve("P7D", now);

    assert_eq!(
        bounds(&resolved),
        Bounds::Finite {
            from: utc(2026, 9, 7, 9),
            to: now,
        }
    );
}

#[test]
fn a_window_past_the_end_of_the_data_stays_empty_rather_than_sliding_back() {
    let newest_row = utc(2026, 9, 11, 5);

    let resolved = resolve("PDC", utc(2026, 9, 14, 9));

    let Bounds::Finite { from, .. } = bounds(&resolved) else {
        panic!("a previous-day window is finite");
    };
    assert!(
        from > newest_row,
        "the window starts after the newest row, so it selects nothing"
    );
}

#[test]
fn all_time_is_unbounded_but_month_bucketed() {
    let resolved = resolve("inf", utc(2026, 9, 10, 15));

    assert_eq!(bounds(&resolved), Bounds::Unbounded);
    assert_eq!(resolved.grain(), Some(Grain::Month));
}

#[test]
fn explicit_intervals_are_canonical_increasing_dates_with_span_grains() {
    let now = utc(2026, 9, 10, 15);
    let cases = [
        ("2026-09-01/2026-09-02", Grain::Hour),
        ("2026-08-01/2026-09-01", Grain::Day),
        ("2026-06-01/2026-09-01", Grain::Week),
        ("2026-01-01/2026-09-01", Grain::Month),
    ];

    for (token, grain) in cases {
        let resolved = resolve(token, now);
        assert_eq!(resolved.grain(), Some(grain), "{token}");
    }

    for invalid in [
        "P14D",
        "2026-09-01",
        "2026-09-01/2026-09-01",
        "2026-09-02/2026-09-01",
        "2026-02-30/2026-03-01",
        "2026-9-1/2026-09-02",
        "2026-09-01/2026-09-02/2026-09-03",
    ] {
        assert!(
            RequestedRange::parse(invalid).is_err(),
            "should reject {invalid}"
        );
    }
}

#[test]
fn maximum_ranges_accept_positive_days_months_and_years_only() {
    for valid in ["P1D", "P31D", "P1M", "P12M", "P1Y"] {
        assert!(MaximumRange::parse(valid).is_ok(), "should accept {valid}");
    }
    for invalid in [
        "",
        "P0D",
        "P-1D",
        "PT24H",
        "P1W",
        "P1.5D",
        "P999999999999999999999D",
    ] {
        assert!(
            MaximumRange::parse(invalid).is_err(),
            "should reject {invalid}"
        );
    }
}

#[test]
fn maximum_range_accepts_its_exact_boundary_and_rejects_wider_or_unbounded() {
    let now = utc(2026, 9, 10, 15);
    let month = cap("P1M");
    let exact = resolve("2026-02-01/2026-03-01", now);
    let wider = resolve("2026-01-31/2026-03-01", now);
    let unbounded = resolve("inf", now);

    assert!(month.allows(&exact));
    assert!(!month.allows(&wider));
    assert!(!month.allows(&unbounded));
}

#[test]
fn a_request_naming_no_range_asks_for_the_legacy_window() {
    let requested = WindowRequest::parse(None, None)
        .unwrap_or_else(|error| panic!("an empty request parses: {error}"));

    let window = requested
        .resolve(utc(2026, 9, 10, 15))
        .unwrap_or_else(|error| panic!("an empty request resolves: {error}"));

    assert_eq!(window, Window::legacy());
    assert!(matches!(window, Window::Unwindowed));
}

#[test]
fn a_request_that_wants_a_total_keeps_the_window_and_drops_the_bucket() {
    let now = utc(2026, 9, 10, 15);
    let requested = WindowRequest::parse(Some("P7D"), Some(false))
        .unwrap_or_else(|error| panic!("the request parses: {error}"));

    let window = requested
        .resolve(now)
        .unwrap_or_else(|error| panic!("the request resolves: {error}"));

    assert_eq!(
        bounds(&window),
        Bounds::Finite {
            from: utc(2026, 9, 3, 15),
            to: now,
        }
    );
    assert_eq!(window.grain(), None);
    assert!(matches!(window, Window::Requested { .. }));
}

#[test]
fn a_request_refuses_a_range_nobody_offers() {
    assert_eq!(
        WindowRequest::parse(Some("P14D"), None),
        Err(WindowError::Range("P14D".to_owned()))
    );
}
