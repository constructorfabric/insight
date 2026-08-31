#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct YearMonth {
    pub(crate) year: i32,
    pub(crate) month: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    Overdue,
    Slot(usize),
    Future,
}

impl YearMonth {
    pub(crate) fn placement(self, window_start: Self, months: usize) -> Placement {
        let offset = self.months_since_epoch() - window_start.months_since_epoch();

        let Ok(slot) = usize::try_from(offset) else {
            return Placement::Overdue;
        };

        if slot >= months {
            return Placement::Future;
        }

        Placement::Slot(slot)
    }

    fn months_since_epoch(self) -> i64 {
        i64::from(self.year) * 12 + i64::from(self.month)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Milestone {
    Due(YearMonth),
    Backlog,
    Unrecognized(String),
}

impl Milestone {
    pub(crate) fn parse(title: &str) -> Option<Self> {
        let title = title.trim();

        if title.is_empty() {
            return None;
        }

        if title.eq_ignore_ascii_case("backlog") {
            return Some(Self::Backlog);
        }

        Some(
            parse_year_month(title).map_or_else(|| Self::Unrecognized(title.to_owned()), Self::Due),
        )
    }
}

fn parse_year_month(title: &str) -> Option<YearMonth> {
    let (year, month) = title.split_once('.').or_else(|| title.split_once('-'))?;

    let year: i32 = year.parse().ok()?;
    let month: u32 = month.parse().ok()?;

    if !(1..=12).contains(&month) {
        return None;
    }

    Some(YearMonth {
        year: if year < 100 { 2000 + year } else { year },
        month,
    })
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test setup panics on a broken fixture"
)]
mod tests {
    use super::{Milestone, Placement, YearMonth};

    fn due(year: i32, month: u32) -> Milestone {
        Milestone::Due(YearMonth { year, month })
    }

    #[test]
    fn short_and_iso_month_titles_parse_to_the_same_month() {
        for title in ["26.09", "2026-09"] {
            assert_eq!(
                Milestone::parse(title),
                Some(due(2026, 9)),
                "title: {title}"
            );
        }
    }

    #[test]
    fn the_backlog_title_is_its_own_state() {
        assert_eq!(Milestone::parse("Backlog"), Some(Milestone::Backlog));
    }

    #[test]
    fn an_unreadable_title_is_kept_rather_than_dropped() {
        assert_eq!(
            Milestone::parse("later this year"),
            Some(Milestone::Unrecognized("later this year".to_owned()))
        );
    }

    #[test]
    fn an_empty_title_is_no_milestone() {
        assert_eq!(Milestone::parse(""), None);
    }

    const WINDOW_START: YearMonth = YearMonth {
        year: 2030,
        month: 8,
    };
    const WINDOW_MONTHS: usize = 9;

    fn placement(year: i32, month: u32) -> Placement {
        YearMonth { year, month }.placement(WINDOW_START, WINDOW_MONTHS)
    }

    #[test]
    fn a_month_inside_the_window_lands_on_its_slot() {
        let cases = [
            ((2030, 8), Placement::Slot(0)),
            ((2030, 9), Placement::Slot(1)),
            ((2031, 4), Placement::Slot(8)),
        ];

        for ((year, month), expected) in cases {
            assert_eq!(placement(year, month), expected, "month: {year}-{month}");
        }
    }

    #[test]
    fn a_month_before_the_window_is_overdue_not_backlog() {
        assert_eq!(placement(2030, 5), Placement::Overdue);
        assert_eq!(placement(2029, 12), Placement::Overdue);
    }

    #[test]
    fn a_month_past_the_window_is_future_work() {
        assert_eq!(placement(2031, 5), Placement::Future);
    }
}
