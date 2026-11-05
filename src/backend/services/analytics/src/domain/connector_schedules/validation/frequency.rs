use super::Violation;

const DAY_MINUTES: u16 = 1440;

#[derive(Debug, Clone, Copy)]
struct Start {
    minute: u16,
    expression: usize,
}

#[derive(Debug, Clone, Copy)]
struct Gap {
    minutes: u16,
    earlier: Start,
    later: Start,
}

impl Gap {
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
        Violation::new(
            format!("crons[{}]", later.expression),
            format!(
                "{starts} {} minutes apart ({:02}:{:02} and {:02}:{:02} UTC); a schedule may start at most once every {minimum} minutes",
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
    let mut starts: Vec<_> = expressions
        .iter()
        .enumerate()
        .flat_map(expression_starts)
        .collect();
    starts.sort_unstable_by_key(|start| (start.minute, start.expression));
    let first = starts[0];
    let last = starts[starts.len() - 1];
    let wrap = Gap {
        minutes: DAY_MINUTES - last.minute + first.minute,
        earlier: last,
        later: first,
    };
    let shortest = starts
        .windows(2)
        .map(|pair| Gap {
            minutes: pair[1].minute - pair[0].minute,
            earlier: pair[0],
            later: pair[1],
        })
        .chain(std::iter::once(wrap))
        .min_by_key(|gap| gap.minutes)
        .unwrap_or(wrap);

    if shortest.minutes < minimum {
        return Err(shortest.violation(crons, minimum));
    }
    Ok(())
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
