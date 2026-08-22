//! The `since`/`until` pair the admin read models accept.

use chrono::{Duration, NaiveDate, Utc};
use toolkit_canonical_errors::CanonicalError;

const MAX_WINDOW_DAYS: i64 = 400;

const DEFAULT_WINDOW_DAYS: i64 = 29;

#[derive(Debug)]
pub(crate) struct Window {
    pub(crate) since: NaiveDate,
    pub(crate) until: NaiveDate,
}

/// The caller's own resource-namespaced `invalid_argument`.
pub(crate) type Violation = fn(&str, &str) -> CanonicalError;

pub(crate) fn parse_window(
    since: Option<&str>,
    until: Option<&str>,
    violation: Violation,
) -> Result<Window, CanonicalError> {
    let until = parse_day("until", until, violation)?.unwrap_or_else(|| Utc::now().date_naive());
    let since = parse_day("since", since, violation)?
        .unwrap_or_else(|| until - Duration::days(DEFAULT_WINDOW_DAYS));

    if since > until {
        return Err(violation("since", "since must not be after until"));
    }
    if (until - since).num_days() >= MAX_WINDOW_DAYS {
        return Err(violation("since", "the window must not exceed 400 days"));
    }

    Ok(Window { since, until })
}

fn parse_day(
    field: &str,
    value: Option<&str>,
    violation: Violation,
) -> Result<Option<NaiveDate>, CanonicalError> {
    value
        .map(|day| {
            NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .map_err(|_| violation(field, "date must use YYYY-MM-DD"))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(field: &str, description: &str) -> CanonicalError {
        CanonicalError::internal(format!("{field}: {description}")).create()
    }

    fn window(since: Option<&str>, until: Option<&str>) -> Option<Window> {
        parse_window(since, until, refused).ok()
    }

    #[test]
    fn a_day_that_cannot_exist_is_refused_rather_than_queried() {
        assert!(window(Some("2026-99-99"), None).is_none());
        assert!(window(None, Some("not-a-day")).is_none());
        assert!(window(Some("2026-08-01"), None).is_some());
    }

    #[test]
    fn a_window_that_runs_backwards_is_refused() {
        assert!(window(Some("2026-08-02"), Some("2026-08-01")).is_none());
        assert!(window(Some("2026-08-01"), Some("2026-08-01")).is_some());
    }

    #[test]
    fn the_widest_window_is_the_one_the_message_promises() {
        assert!(
            window(Some("2026-01-01"), Some("2027-02-04")).is_some(),
            "400 days"
        );
        assert!(
            window(Some("2026-01-01"), Some("2027-02-05")).is_none(),
            "401 days"
        );
    }

    #[test]
    fn an_absent_since_reaches_back_a_month_from_until() {
        let window = window(None, Some("2026-08-30")).map(|w| w.since.to_string());

        assert_eq!(window, Some("2026-08-01".to_owned()));
    }
}
