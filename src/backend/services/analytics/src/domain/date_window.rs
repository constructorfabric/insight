//! The `since`/`until` pair a read model over a time range accepts.

use chrono::{Duration, NaiveDate, Utc};

const MAX_WINDOW_DAYS: i64 = 400;

const DEFAULT_WINDOW_DAYS: i64 = 29;

#[derive(Debug)]
pub(crate) struct Window {
    pub(crate) since: NaiveDate,
    pub(crate) until: NaiveDate,
}

/// Why a window was refused. The caller raises it in its own resource
/// namespace; this module knows the rule, not the surface that broke it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowError {
    Malformed { field: &'static str },
    Reversed,
    TooWide,
}

impl WindowError {
    /// The request field a reader has to change to be let in.
    pub(crate) fn field(self) -> &'static str {
        match self {
            Self::Malformed { field } => field,
            Self::Reversed | Self::TooWide => "since",
        }
    }

    pub(crate) fn description(self) -> String {
        match self {
            Self::Malformed { .. } => "date must use YYYY-MM-DD".to_owned(),
            Self::Reversed => "since must not be after until".to_owned(),
            Self::TooWide => format!("the window must not exceed {MAX_WINDOW_DAYS} days"),
        }
    }
}

pub(crate) fn parse_window(
    since: Option<&str>,
    until: Option<&str>,
) -> Result<Window, WindowError> {
    let until = parse_day("until", until)?.unwrap_or_else(|| Utc::now().date_naive());
    let since =
        parse_day("since", since)?.unwrap_or_else(|| until - Duration::days(DEFAULT_WINDOW_DAYS));

    if since > until {
        return Err(WindowError::Reversed);
    }
    if (until - since).num_days() >= MAX_WINDOW_DAYS {
        return Err(WindowError::TooWide);
    }

    Ok(Window { since, until })
}

fn parse_day(field: &'static str, value: Option<&str>) -> Result<Option<NaiveDate>, WindowError> {
    value
        .map(|day| {
            NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(|_| WindowError::Malformed { field })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(since: Option<&str>, until: Option<&str>) -> Option<WindowError> {
        parse_window(since, until).err()
    }

    #[test]
    fn a_day_that_cannot_exist_is_refused_rather_than_queried() {
        assert_eq!(
            refused(Some("2026-99-99"), None),
            Some(WindowError::Malformed { field: "since" })
        );
        assert_eq!(
            refused(None, Some("not-a-day")),
            Some(WindowError::Malformed { field: "until" })
        );
        assert_eq!(refused(Some("2026-08-01"), None), None);
    }

    #[test]
    fn a_window_that_runs_backwards_is_refused() {
        assert_eq!(
            refused(Some("2026-08-02"), Some("2026-08-01")),
            Some(WindowError::Reversed)
        );
        assert_eq!(refused(Some("2026-08-01"), Some("2026-08-01")), None);
    }

    #[test]
    fn the_widest_window_is_the_one_the_message_promises() {
        assert_eq!(refused(Some("2026-01-01"), Some("2027-02-04")), None);

        let too_wide = refused(Some("2026-01-01"), Some("2027-02-05"));

        assert_eq!(too_wide, Some(WindowError::TooWide));
        assert_eq!(
            too_wide.map(WindowError::description),
            Some("the window must not exceed 400 days".to_owned()),
            "the message has to name the cap the code enforces"
        );
    }

    #[test]
    fn an_absent_since_reaches_back_a_month_from_until() {
        let window = parse_window(None, Some("2026-08-30")).ok().map(|w| w.since);

        assert_eq!(window, NaiveDate::from_ymd_opt(2026, 8, 1));
    }

    #[test]
    fn a_refusal_names_the_field_the_reader_has_to_change() {
        assert_eq!(WindowError::Malformed { field: "until" }.field(), "until");
        assert_eq!(WindowError::Reversed.field(), "since");
        assert_eq!(WindowError::TooWide.field(), "since");
    }
}
