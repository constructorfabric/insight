use std::cmp::Ordering;
use std::collections::HashMap;

use serde::Deserialize;

use super::model::Gear;
use super::progress::LadderStep;
use super::response::Placement;

/// The column a caller orders by. Named for what a reader sees, not for the
/// field it happens to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GearSort {
    #[default]
    Gear,
    Subsystem,
    Spec,
    Sdk,
    Impl,
    Effort,
    Remaining,
    Milestone,
    Forecast,
    Assignees,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Direction {
    #[default]
    Asc,
    Desc,
}

/// The order a caller asked for. Both halves default, so a request that names
/// neither still has an order — and one that names a column it does not
/// recognise is refused rather than silently reordered.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Sort {
    pub(crate) sort: GearSort,
    pub(crate) direction: Direction,
}

/// One column's value, in a form that orders. A gear carrying no value sorts
/// last in both directions: "nobody recorded this" is not a small number, and
/// ordering it as one reads as if it were.
#[derive(Clone, Copy)]
enum Key<'a> {
    Text(Option<&'a str>),
    Number(Option<f64>),
}

/// What a column needs beyond the gear itself: where it falls against the
/// window, and when the schedule lands it.
pub(crate) struct Context<'a, P: Fn(&Gear) -> Placement> {
    pub(crate) placement: P,
    pub(crate) forecasts: &'a HashMap<String, String>,
}

/// Gears ordered for presentation. Placement and forecast are decided
/// elsewhere; this only decides sequence.
pub(crate) fn order<P: Fn(&Gear) -> Placement>(
    gears: &mut [&Gear],
    sort: Sort,
    context: &Context<'_, P>,
) {
    gears.sort_by(|left, right| {
        let ordering = compare(
            key(left, sort.sort, context),
            key(right, sort.sort, context),
        );

        match sort.direction {
            Direction::Asc => ordering,
            Direction::Desc => reverse_present(ordering, left, right, sort.sort, context),
        }
    });
}

/// Reversing must not lift absent values to the top, so a pair where one side
/// is absent keeps its ascending order.
fn reverse_present<P: Fn(&Gear) -> Placement>(
    ordering: Ordering,
    left: &Gear,
    right: &Gear,
    sort: GearSort,
    context: &Context<'_, P>,
) -> Ordering {
    if is_absent(key(left, sort, context)) || is_absent(key(right, sort, context)) {
        return ordering;
    }

    ordering.reverse()
}

fn is_absent(key: Key<'_>) -> bool {
    match key {
        Key::Text(value) => value.is_none(),
        Key::Number(value) => value.is_none(),
    }
}

fn compare(left: Key<'_>, right: Key<'_>) -> Ordering {
    match (left, right) {
        (Key::Text(left), Key::Text(right)) => match (left, right) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(left), Some(right)) => left.cmp(right),
        },
        (Key::Number(left), Key::Number(right)) => match (left, right) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(left), Some(right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
        },
        _ => Ordering::Equal,
    }
}

fn key<'a, P: Fn(&Gear) -> Placement>(
    gear: &'a Gear,
    sort: GearSort,
    context: &'a Context<'a, P>,
) -> Key<'a> {
    match sort {
        GearSort::Gear => Key::Text(Some(&gear.title)),
        GearSort::Subsystem => Key::Text(gear.subsystem.as_deref()),
        GearSort::Spec => Key::Number(percent(gear.design)),
        GearSort::Sdk => Key::Number(percent(gear.sdk)),
        GearSort::Impl => Key::Number(percent(gear.status)),
        GearSort::Effort => Key::Number(gear.effort_man_days),
        GearSort::Remaining => Key::Number(gear.remaining_man_days()),
        // Lateness leads: an overdue gear is not merely an earlier month, and
        // sorting it as one buries the overruns among ordinary past milestones.
        GearSort::Milestone => Key::Number(match (context.placement)(gear) {
            Placement::Overdue { days } => Some(-days_as_f64(days)),
            _ => gear.milestone_sort_key(),
        }),
        GearSort::Forecast => Key::Text(context.forecasts.get(&gear.id()).map(String::as_str)),
        GearSort::Assignees => Key::Text(gear.assignees.first().map(String::as_str)),
    }
}

/// A day count never approaches the mantissa's limit, and ordering only needs
/// the comparison to hold.
#[expect(clippy::cast_precision_loss, reason = "a day count is far below 2^53")]
fn days_as_f64(days: i64) -> f64 {
    days as f64
}

fn percent(step: Option<LadderStep>) -> Option<f64> {
    step.and_then(LadderStep::percent_complete).map(f64::from)
}
