use super::*;

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
