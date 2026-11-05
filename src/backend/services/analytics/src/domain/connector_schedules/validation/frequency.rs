use super::{STAR_BIT, Violation};

const DAY_MINUTES: u16 = 1440;
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

#[derive(Debug, Clone, Copy)]
struct Start {
    minute: u16,
    expression: usize,
}

#[derive(Debug, Clone, Copy)]
struct DayStarts {
    first: Start,
    last: Start,
    closest: Option<(Start, Start)>,
}

#[derive(Debug, Clone, Copy)]
struct Gap {
    minutes: u16,
    earlier: Start,
    later: Start,
    weekday: usize,
    next_day: bool,
}

impl Gap {
    fn across_midnight(earlier: Start, later: Start, weekday: usize) -> Self {
        Self {
            minutes: DAY_MINUTES - earlier.minute + later.minute,
            earlier,
            later,
            weekday,
            next_day: true,
        }
    }

    fn retain_shortest(self, shortest: &mut Option<Self>) {
        if shortest.is_none_or(|previous| self.minutes < previous.minutes) {
            *shortest = Some(self);
        }
    }

    fn violation(self, crons: &[String], minimum: u16) -> Violation {
        let earlier = self.earlier;
        let later = self.later;
        let starts = if earlier.expression == later.expression {
            format!(
                "expression {} {:?} has starts",
                later.expression, crons[later.expression]
            )
        } else {
            format!(
                "expressions {} {:?} and {} {:?} start",
                earlier.expression,
                crons[earlier.expression],
                later.expression,
                crons[later.expression]
            )
        };
        let first_day = WEEKDAYS[self.weekday];
        let second_day = if self.next_day {
            format!("{} ", WEEKDAYS[(self.weekday + 1) % 7])
        } else {
            String::new()
        };
        Violation::new(
            format!("crons[{}]", later.expression),
            format!(
                "{starts} {} minutes apart ({first_day} {:02}:{:02} and {second_day}{:02}:{:02} UTC); a schedule may start at most once every {minimum} minutes",
                self.minutes,
                earlier.minute / 60,
                earlier.minute % 60,
                later.minute / 60,
                later.minute % 60
            ),
        )
    }
}

pub(super) fn check(
    crons: &[String],
    expressions: &[[u64; 5]],
    minimum: u16,
) -> Result<(), Violation> {
    let masks = calendar_masks(expressions);
    let times: Vec<_> = expressions
        .iter()
        .enumerate()
        .map(expression_starts)
        .collect();
    let count = 1 << expressions.len();
    let mut days = vec![None; count];
    let mut pairs = vec![false; count * count];
    let mut shortest = None;
    let mut weekday = 6;
    let mut previous = 0;

    for year in 2000..2400 {
        let february = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
            29
        } else {
            28
        };
        for (month, length) in [31, february, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
            .into_iter()
            .enumerate()
        {
            for day in masks[month + 1].iter().take(length + 1).skip(1) {
                let mask = usize::from(day[weekday]);
                if mask != 0 && days[mask].is_none() {
                    let starts = day_starts(mask, &times);
                    if let Some((earlier, later)) = starts.closest {
                        Gap {
                            minutes: later.minute - earlier.minute,
                            earlier,
                            later,
                            weekday,
                            next_day: false,
                        }
                        .retain_shortest(&mut shortest);
                    }
                    days[mask] = Some(starts);
                }
                let pair = previous * count + mask;
                if !pairs[pair] {
                    if let (Some(earlier), Some(later)) = (days[previous], days[mask]) {
                        Gap::across_midnight(earlier.last, later.first, (weekday + 6) % 7)
                            .retain_shortest(&mut shortest);
                    }
                    pairs[pair] = true;
                }
                previous = mask;
                weekday = (weekday + 1) % 7;
            }
        }
    }
    let first = usize::from(masks[1][1][6]);
    if let (Some(earlier), Some(later)) = (days[previous], days[first]) {
        Gap::across_midnight(earlier.last, later.first, 5).retain_shortest(&mut shortest);
    }

    if let Some(gap) = shortest.filter(|gap| gap.minutes < minimum) {
        return Err(gap.violation(crons, minimum));
    }
    Ok(())
}

fn calendar_masks(expressions: &[[u64; 5]]) -> [[[u16; 7]; 32]; 13] {
    let mut masks = [[[0; 7]; 32]; 13];
    for (month, days) in masks.iter_mut().enumerate().skip(1) {
        for (day, weekdays) in days.iter_mut().enumerate().skip(1) {
            for (weekday, mask) in weekdays.iter_mut().enumerate() {
                for (index, bits) in expressions.iter().enumerate() {
                    let dom = bits[2] & (1 << day) != 0;
                    let dow = bits[4] & (1 << weekday) != 0;
                    let matches = if (bits[2] | bits[4]) & STAR_BIT != 0 {
                        dom && dow
                    } else {
                        dom || dow
                    };
                    if bits[3] & (1 << month) != 0 && matches {
                        *mask |= 1 << index;
                    }
                }
            }
        }
    }
    masks
}

fn expression_starts((expression, bits): (usize, &[u64; 5])) -> Vec<Start> {
    let mut starts = Vec::new();
    for hour in 0..24 {
        if bits[1] & (1 << hour) == 0 {
            continue;
        }
        for minute in 0..60 {
            if bits[0] & (1 << minute) != 0 {
                starts.push(Start {
                    minute: hour * 60 + minute,
                    expression,
                });
            }
        }
    }
    starts
}

fn day_starts(mask: usize, times: &[Vec<Start>]) -> DayStarts {
    let mut starts = Vec::new();
    for (index, times) in times.iter().enumerate() {
        if mask & (1 << index) != 0 {
            starts.extend_from_slice(times);
        }
    }
    starts.sort_unstable_by_key(|start| (start.minute, start.expression));
    let closest = starts
        .windows(2)
        .min_by_key(|pair| pair[1].minute - pair[0].minute)
        .map(|pair| (pair[0], pair[1]));
    DayStarts {
        first: starts[0],
        last: starts[starts.len() - 1],
        closest,
    }
}
