use chrono::{Datelike, Days, NaiveDate};

use super::dto::ReportGranularity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReportBucket {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl From<ReportGranularity> for ReportBucket {
    fn from(value: ReportGranularity) -> Self {
        match value {
            ReportGranularity::Day => Self::Day,
            ReportGranularity::Week => Self::Week,
            ReportGranularity::Month => Self::Month,
            ReportGranularity::Quarter => Self::Quarter,
            ReportGranularity::Year => Self::Year,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedPeriod {
    pub(crate) label: String,
    pub(crate) bucket_start: NaiveDate,
    pub(crate) from: NaiveDate,
    pub(crate) to: NaiveDate,
}

pub(crate) fn enumerate_periods(
    from: NaiveDate,
    to: NaiveDate,
    bucket: ReportBucket,
) -> Vec<PlannedPeriod> {
    if from > to {
        return Vec::new();
    }

    let mut periods = Vec::new();
    let mut bucket_start = containing_bucket_start(from, bucket);
    loop {
        let next_start = following_bucket_start(bucket_start, bucket);
        let bucket_end = next_start
            .and_then(|start| start.checked_sub_days(Days::new(1)))
            .unwrap_or(NaiveDate::MAX);
        let period_from = from.max(bucket_start);
        let period_to = to.min(bucket_end);

        periods.push(PlannedPeriod {
            label: bucket_label(bucket_start, bucket),
            bucket_start,
            from: period_from,
            to: period_to,
        });

        if period_to == to {
            break;
        }
        bucket_start = next_start.unwrap_or(NaiveDate::MAX);
    }

    periods
}

pub(crate) fn count_periods(from: NaiveDate, to: NaiveDate, bucket: ReportBucket) -> Option<usize> {
    if from > to {
        return Some(0);
    }

    let first = containing_bucket_start(from, bucket);
    let last = containing_bucket_start(to, bucket);
    let distance = match bucket {
        ReportBucket::Day => (last - first).num_days(),
        ReportBucket::Week => (last - first).num_days().checked_div(7)?,
        ReportBucket::Month => calendar_index(last, 12)?.checked_sub(calendar_index(first, 12)?)?,
        ReportBucket::Quarter => calendar_index(last, 4)?.checked_sub(calendar_index(first, 4)?)?,
        ReportBucket::Year => i64::from(last.year()).checked_sub(i64::from(first.year()))?,
    };
    let count = distance.checked_add(1)?;

    usize::try_from(count).ok()
}

fn calendar_index(date: NaiveDate, periods_per_year: i64) -> Option<i64> {
    let period = match periods_per_year {
        12 => i64::from(date.month0()),
        4 => i64::from(date.month0() / 3),
        _ => return None,
    };

    i64::from(date.year())
        .checked_mul(periods_per_year)?
        .checked_add(period)
}

pub(crate) fn containing_bucket_start(date: NaiveDate, bucket: ReportBucket) -> NaiveDate {
    match bucket {
        ReportBucket::Day => date,
        ReportBucket::Week => date
            .checked_sub_days(Days::new(u64::from(date.weekday().num_days_from_monday())))
            .unwrap_or(NaiveDate::MIN),
        ReportBucket::Month => calendar_date(date.year(), date.month(), 1),
        ReportBucket::Quarter => {
            let first_month = ((date.month() - 1) / 3) * 3 + 1;
            calendar_date(date.year(), first_month, 1)
        }
        ReportBucket::Year => calendar_date(date.year(), 1, 1),
    }
}

fn following_bucket_start(date: NaiveDate, bucket: ReportBucket) -> Option<NaiveDate> {
    match bucket {
        ReportBucket::Day => date.checked_add_days(Days::new(1)),
        ReportBucket::Week => date.checked_add_days(Days::new(7)),
        ReportBucket::Month => add_calendar_months(date, 1),
        ReportBucket::Quarter => add_calendar_months(date, 3),
        ReportBucket::Year => date
            .year()
            .checked_add(1)
            .and_then(|year| NaiveDate::from_ymd_opt(year, 1, 1)),
    }
}

fn add_calendar_months(date: NaiveDate, months: u32) -> Option<NaiveDate> {
    let zero_based_month = date.month0().checked_add(months)?;
    let year_offset = i32::try_from(zero_based_month / 12).ok()?;
    let year = date.year().checked_add(year_offset)?;
    let month = zero_based_month % 12 + 1;

    NaiveDate::from_ymd_opt(year, month, 1)
}

fn calendar_date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or(NaiveDate::MIN)
}

fn bucket_label(start: NaiveDate, bucket: ReportBucket) -> String {
    match bucket {
        ReportBucket::Day | ReportBucket::Week => start.format("%Y-%m-%d").to_string(),
        ReportBucket::Month => start.format("%Y-%m").to_string(),
        ReportBucket::Quarter => format!("{}-Q{}", start.year(), (start.month0() / 3) + 1),
        ReportBucket::Year => start.format("%Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .unwrap_or_else(|error| panic!("fixture date must parse: {error}"))
    }

    #[test]
    fn clips_calendar_period_boundaries() {
        let cases = [
            (
                ReportBucket::Day,
                "2024-02-28",
                "2024-03-01",
                vec![
                    ("2024-02-28", "2024-02-28", "2024-02-28"),
                    ("2024-02-29", "2024-02-29", "2024-02-29"),
                    ("2024-03-01", "2024-03-01", "2024-03-01"),
                ],
            ),
            (
                ReportBucket::Week,
                "2026-05-13",
                "2026-05-26",
                vec![
                    ("2026-05-11", "2026-05-13", "2026-05-17"),
                    ("2026-05-18", "2026-05-18", "2026-05-24"),
                    ("2026-05-25", "2026-05-25", "2026-05-26"),
                ],
            ),
            (
                ReportBucket::Month,
                "2024-02-10",
                "2024-04-03",
                vec![
                    ("2024-02", "2024-02-10", "2024-02-29"),
                    ("2024-03", "2024-03-01", "2024-03-31"),
                    ("2024-04", "2024-04-01", "2024-04-03"),
                ],
            ),
            (
                ReportBucket::Quarter,
                "2025-12-20",
                "2026-04-05",
                vec![
                    ("2025-Q4", "2025-12-20", "2025-12-31"),
                    ("2026-Q1", "2026-01-01", "2026-03-31"),
                    ("2026-Q2", "2026-04-01", "2026-04-05"),
                ],
            ),
            (
                ReportBucket::Year,
                "2023-07-01",
                "2025-02-01",
                vec![
                    ("2023", "2023-07-01", "2023-12-31"),
                    ("2024", "2024-01-01", "2024-12-31"),
                    ("2025", "2025-01-01", "2025-02-01"),
                ],
            ),
        ];

        for (bucket, from, to, expected) in cases {
            let actual = enumerate_periods(date(from), date(to), bucket)
                .into_iter()
                .map(|period| {
                    (
                        period.label,
                        period.from.format("%Y-%m-%d").to_string(),
                        period.to.format("%Y-%m-%d").to_string(),
                    )
                })
                .collect::<Vec<_>>();
            let expected = expected
                .into_iter()
                .map(|(label, from, to)| (label.to_owned(), from.to_owned(), to.to_owned()))
                .collect::<Vec<_>>();

            assert_eq!(actual, expected, "wrong periods for {bucket:?}");
            assert_eq!(
                count_periods(date(from), date(to), bucket),
                Some(actual.len()),
                "wrong period count for {bucket:?}"
            );
        }
    }
}
