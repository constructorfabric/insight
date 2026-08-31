use chrono::{Duration, NaiveDate};

const CAPACITY_MAN_DAYS_PER_DAY: f64 = 1.0;
const MAX_SPAN_DAYS: i64 = 3650;
const MAX_SPAN_DAYS_F64: f64 = 3650.0;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScheduleItem<'a> {
    pub(crate) gear_number: i64,
    pub(crate) remaining_man_days: f64,
    pub(crate) assignee: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) gear_number: i64,
    pub(crate) start: NaiveDate,
    pub(crate) end: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lane {
    pub(crate) assignee: Option<String>,
    pub(crate) spans: Vec<Span>,
}

pub(crate) fn schedule(items: &[ScheduleItem<'_>], from: NaiveDate) -> Vec<Lane> {
    let mut lanes: Vec<Lane> = Vec::new();

    for item in items {
        if item.remaining_man_days <= 0.0 {
            continue;
        }

        let index = lane_for(&mut lanes, item.assignee);
        let lane = &mut lanes[index];

        let start = lane
            .spans
            .last()
            .map_or(from, |span| span.end.succ_opt().unwrap_or(span.end));
        let end = start + Duration::days(span_days(item.remaining_man_days) - 1);

        lane.spans.push(Span {
            gear_number: item.gear_number,
            start,
            end,
        });
    }

    lanes
}

fn lane_for(lanes: &mut Vec<Lane>, assignee: Option<&str>) -> usize {
    let existing = assignee.and_then(|name| {
        lanes
            .iter()
            .position(|lane| lane.assignee.as_deref() == Some(name))
    });

    if let Some(index) = existing {
        return index;
    }

    lanes.push(Lane {
        assignee: assignee.map(str::to_owned),
        spans: Vec::new(),
    });

    lanes.len() - 1
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "clamped below the cast, so the value always fits"
)]
fn span_days(remaining_man_days: f64) -> i64 {
    let days = (remaining_man_days / CAPACITY_MAN_DAYS_PER_DAY).ceil();

    if days >= MAX_SPAN_DAYS_F64 {
        return MAX_SPAN_DAYS;
    }

    days as i64
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test setup panics on a broken fixture"
)]
mod tests {
    use chrono::NaiveDate;

    use super::{ScheduleItem, schedule};

    const FIRST_GEAR: i64 = 1;
    const SECOND_GEAR: i64 = 2;

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 1, day).expect("valid date")
    }

    #[test]
    fn one_assignee_runs_their_gears_back_to_back() {
        let items = [
            ScheduleItem {
                gear_number: FIRST_GEAR,
                remaining_man_days: 3.0,
                assignee: Some("dev-one"),
            },
            ScheduleItem {
                gear_number: SECOND_GEAR,
                remaining_man_days: 2.0,
                assignee: Some("dev-one"),
            },
        ];

        let lanes = schedule(&items, day(1));

        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].spans[0].start, day(1));
        assert_eq!(lanes[0].spans[0].end, day(3));
        assert_eq!(lanes[0].spans[1].start, day(4));
        assert_eq!(lanes[0].spans[1].end, day(5));
    }

    #[test]
    fn every_unassigned_gear_gets_its_own_lane() {
        let items = [
            ScheduleItem {
                gear_number: FIRST_GEAR,
                remaining_man_days: 3.0,
                assignee: None,
            },
            ScheduleItem {
                gear_number: SECOND_GEAR,
                remaining_man_days: 3.0,
                assignee: None,
            },
        ];

        let lanes = schedule(&items, day(1));

        assert_eq!(lanes.len(), 2);
        assert_eq!(lanes[0].spans[0].start, day(1));
        assert_eq!(lanes[1].spans[0].start, day(1));
    }

    #[test]
    fn a_gear_with_nothing_left_is_not_scheduled() {
        let items = [
            ScheduleItem {
                gear_number: FIRST_GEAR,
                remaining_man_days: 0.0,
                assignee: Some("dev-one"),
            },
            ScheduleItem {
                gear_number: SECOND_GEAR,
                remaining_man_days: 2.0,
                assignee: Some("dev-one"),
            },
        ];

        let lanes = schedule(&items, day(1));

        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].spans.len(), 1);
        assert_eq!(lanes[0].spans[0].gear_number, SECOND_GEAR);
        assert_eq!(lanes[0].spans[0].start, day(1));
    }
}
