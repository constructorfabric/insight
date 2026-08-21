//! The `since`/`until` pair the admin read models over `product_usage` accept.

use chrono::{Duration, NaiveDate, Utc};
use toolkit_canonical_errors::CanonicalError;

const MAX_WINDOW_DAYS: i64 = 400;

const DEFAULT_WINDOW_DAYS: i64 = 29;

/// Both tables key on `(tenant_id, ts)`, so one predicate binds either.
pub(crate) const WINDOW: &str =
    "tenant_id = toUUID(?) AND toDate(ts) >= toDate(?) AND toDate(ts) <= toDate(?)";

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
