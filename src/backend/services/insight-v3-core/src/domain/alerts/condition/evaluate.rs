//! A condition judged over what a metric answered.
//!
//! Each test on a row is true, false, or unknown; groups combine those the
//! way Kleene logic does, so an unknown cell decides nothing it cannot. The
//! quantifier then folds the rows' verdicts into the check's outcome.

use std::cmp::Ordering;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::Value;

use super::witness::{MAX_WITNESS_ROWS, Witness, kept};
use super::{Comparison, Condition, DateTest, Leaf, Node, NumberTest, Quantifier, Test, TextTest};
use crate::domain::alerts::scalar::{Number, Outcome, UnknownReason};
use crate::domain::query::metric_query::RunResult;

/// What one test, group or row came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    True,
    False,
    Unknown(UnknownReason),
}

impl Verdict {
    fn of(holds: bool) -> Self {
        if holds { Self::True } else { Self::False }
    }
}

/// The columns a condition reads, by their place in the result.
struct Positions<'a> {
    names: Vec<&'a str>,
    at: Vec<usize>,
}

impl Positions<'_> {
    fn cell<'r>(&self, row: &'r [Value], column: &str) -> &'r Value {
        self.names
            .iter()
            .position(|name| *name == column)
            .and_then(|index| row.get(self.at[index]))
            .unwrap_or(&Value::Null)
    }
}

/// The outcome of a condition over a run's rows, checked at `evaluated_at`.
///
/// `groups` are the metric's `group_by` columns and `context` the columns
/// the rule asks to carry, both shown in the witness beside the columns the
/// condition reads. `imposed_limit` is where the run stops without the
/// metric having asked to: a run that reached it may have left rows out, and
/// each of those counts as unknown.
pub(crate) fn evaluate(
    result: &RunResult,
    condition: &Condition,
    groups: &[String],
    context: &[String],
    imposed_limit: Option<usize>,
    evaluated_at: DateTime<Utc>,
) -> Outcome {
    let names = condition.columns();
    let mut at = Vec::with_capacity(names.len());
    for name in &names {
        let Some(position) = result.columns.iter().position(|column| column == name) else {
            return Outcome::Unknown(UnknownReason::ColumnMissing);
        };
        at.push(position);
    }
    let positions = Positions { names, at };

    let verdicts: Vec<Verdict> = result
        .rows
        .iter()
        .map(|row| judge(&condition.test, row, &positions, evaluated_at))
        .collect();

    let cut = imposed_limit.is_some_and(|limit| result.rows.len() >= limit);
    let breached = match fold(condition.rows, &verdicts, cut) {
        Ok(breached) => breached,
        Err(reason) => return Outcome::Unknown(reason),
    };

    let shown = |index: usize| match condition.rows {
        Quantifier::One => true,
        Quantifier::Any | Quantifier::All | Quantifier::None | Quantifier::Count { .. } => {
            verdicts[index] == Verdict::True
        }
    };
    let witness = witness(result, condition, groups, context, &verdicts, shown);

    Outcome::Valid { breached, witness }
}

/// Whether the rows' verdicts breach the quantifier, or why that cannot be
/// told. `cut` says rows past these may exist, each of them unknown.
fn fold(quantifier: Quantifier, verdicts: &[Verdict], cut: bool) -> Result<bool, UnknownReason> {
    let passed = verdicts
        .iter()
        .filter(|verdict| **verdict == Verdict::True)
        .count();
    let failed = verdicts
        .iter()
        .filter(|verdict| **verdict == Verdict::False)
        .count();
    let unknown = verdicts
        .iter()
        .find_map(|verdict| match verdict {
            Verdict::Unknown(reason) => Some(*reason),
            Verdict::True | Verdict::False => None,
        })
        .or(cut.then_some(UnknownReason::RowLimit));
    let undecided = verdicts.len() - passed - failed;

    match quantifier {
        Quantifier::One => match verdicts {
            [] => Err(UnknownReason::NoRows),
            [Verdict::True] => Ok(true),
            [Verdict::False] => Ok(false),
            [Verdict::Unknown(reason)] => Err(*reason),
            [_, _, ..] => Err(UnknownReason::ManyRows),
        },
        Quantifier::Any => match (passed, unknown) {
            (1.., _) => Ok(true),
            (0, Some(reason)) => Err(reason),
            (0, None) => Ok(false),
        },
        Quantifier::All => match (verdicts.is_empty(), failed, unknown) {
            (true, _, _) => Err(UnknownReason::NoRows),
            (false, 1.., _) => Ok(false),
            (false, 0, Some(reason)) => Err(reason),
            (false, 0, None) => Ok(true),
        },
        Quantifier::None => match (verdicts.is_empty(), passed, unknown) {
            (true, _, _) => Err(UnknownReason::NoRows),
            (false, 1.., _) => Ok(false),
            (false, 0, Some(reason)) => Err(reason),
            (false, 0, None) => Ok(true),
        },
        Quantifier::Count { comparison, value } => {
            let low = u64::try_from(passed).unwrap_or(u64::MAX);
            let high = if cut {
                u64::MAX
            } else {
                u64::try_from(passed + undecided).unwrap_or(u64::MAX)
            };
            let at_low = compares(comparison, low.cmp(&value));
            let at_high = compares(comparison, high.cmp(&value));
            let outside = value < low || value > high;

            let settled = match comparison {
                Comparison::Gt | Comparison::Ge | Comparison::Lt | Comparison::Le => {
                    (at_low == at_high).then_some(at_low)
                }
                Comparison::Eq => (low == high || outside).then_some(at_low && !outside),
                Comparison::Ne => (low == high || outside).then_some(at_low || outside),
            };

            settled.ok_or_else(|| unknown.unwrap_or(UnknownReason::Incomparable))
        }
    }
}

fn compares(comparison: Comparison, ordering: Ordering) -> bool {
    match comparison {
        Comparison::Gt => ordering == Ordering::Greater,
        Comparison::Ge => ordering != Ordering::Less,
        Comparison::Lt => ordering == Ordering::Less,
        Comparison::Le => ordering != Ordering::Greater,
        Comparison::Eq => ordering == Ordering::Equal,
        Comparison::Ne => ordering != Ordering::Equal,
    }
}

fn judge(node: &Node, row: &[Value], positions: &Positions<'_>, at: DateTime<Utc>) -> Verdict {
    match node {
        Node::All(children) => {
            let mut unknown = None;
            for child in children {
                match judge(child, row, positions, at) {
                    Verdict::False => return Verdict::False,
                    Verdict::Unknown(reason) => {
                        unknown.get_or_insert(reason);
                    }
                    Verdict::True => {}
                }
            }
            unknown.map_or(Verdict::True, Verdict::Unknown)
        }
        Node::Any(children) => {
            let mut unknown = None;
            for child in children {
                match judge(child, row, positions, at) {
                    Verdict::True => return Verdict::True,
                    Verdict::Unknown(reason) => {
                        unknown.get_or_insert(reason);
                    }
                    Verdict::False => {}
                }
            }
            unknown.map_or(Verdict::False, Verdict::Unknown)
        }
        Node::Not(child) => match judge(child, row, positions, at) {
            Verdict::True => Verdict::False,
            Verdict::False => Verdict::True,
            unknown @ Verdict::Unknown(_) => unknown,
        },
        Node::Leaf(leaf) => test(leaf, positions.cell(row, &leaf.column), at),
    }
}

fn test(leaf: &Leaf, cell: &Value, at: DateTime<Utc>) -> Verdict {
    if cell.is_null() {
        return Verdict::Unknown(UnknownReason::Null);
    }

    match &leaf.test {
        Test::Number(test) => number(*test, cell),
        Test::Text(test) => text(test, cell),
        Test::Date(test) => date(*test, cell, at),
    }
}

fn number(test: NumberTest, cell: &Value) -> Verdict {
    let Some(value) = Number::parse(cell) else {
        return Verdict::Unknown(UnknownReason::NonNumeric);
    };

    let holds = match test {
        NumberTest::Compare {
            comparison,
            threshold,
        } => value
            .compare(threshold)
            .map(|ordering| compares(comparison, ordering)),
        NumberTest::Between { low, high } => {
            value
                .compare(low)
                .zip(value.compare(high))
                .map(|(from_low, from_high)| {
                    from_low != Ordering::Less && from_high != Ordering::Greater
                })
        }
    };

    holds.map_or(Verdict::Unknown(UnknownReason::Incomparable), Verdict::of)
}

fn text(test: &TextTest, cell: &Value) -> Verdict {
    let value = match cell {
        Value::String(written) => written.as_str(),
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::Null | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            return Verdict::Unknown(UnknownReason::NonText);
        }
    };

    Verdict::of(match test {
        TextTest::Is(expected) => value == expected,
        TextTest::IsNot(expected) => value != expected,
        TextTest::Contains(part) => value.contains(part.as_str()),
        TextTest::StartsWith(part) => value.starts_with(part.as_str()),
        TextTest::EndsWith(part) => value.ends_with(part.as_str()),
        TextTest::In(values) => values.iter().any(|expected| expected == value),
        TextTest::NotIn(values) => values.iter().all(|expected| expected != value),
    })
}

fn date(test: DateTest, cell: &Value, at: DateTime<Utc>) -> Verdict {
    let Some(value) = cell.as_str().and_then(moment) else {
        return Verdict::Unknown(UnknownReason::NonDate);
    };

    let holds = match test {
        DateTest::Before(bound) => Some(value < bound),
        DateTest::After(bound) => Some(value > bound),
        DateTest::Between { from, to } => Some(from <= value && value <= to),
        DateTest::OlderThan(age) => at.checked_sub_signed(age.span()).map(|edge| value < edge),
        DateTest::NewerThan(age) => at.checked_sub_signed(age.span()).map(|edge| value >= edge),
    };

    holds.map_or(Verdict::Unknown(UnknownReason::Incomparable), Verdict::of)
}

/// A timestamp as the warehouse writes one, read as UTC: RFC 3339, or
/// `YYYY-MM-DD HH:MM:SS[.fff]`, or a bare date at midnight.
fn moment(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(moment) = DateTime::parse_from_rfc3339(text) {
        return Some(moment.to_utc());
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(naive.and_utc());
    }

    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .map(|midnight| midnight.and_utc())
}

fn witness(
    result: &RunResult,
    condition: &Condition,
    groups: &[String],
    context: &[String],
    verdicts: &[Verdict],
    shown: impl Fn(usize) -> bool,
) -> Witness {
    let mut columns: Vec<String> = Vec::new();
    let wanted = groups
        .iter()
        .map(String::as_str)
        .chain(condition.columns())
        .chain(context.iter().map(String::as_str));
    let mut positions = Vec::new();
    for name in wanted {
        if columns.iter().any(|kept| kept == name) {
            continue;
        }
        if let Some(position) = result.columns.iter().position(|column| column == name) {
            columns.push(name.to_owned());
            positions.push(position);
        }
    }

    let rows = result
        .rows
        .iter()
        .enumerate()
        .filter(|(index, _)| shown(*index))
        .take(MAX_WITNESS_ROWS)
        .map(|(_, row)| {
            positions
                .iter()
                .map(|position| row.get(*position).map_or(Value::Null, kept))
                .collect()
        })
        .collect();

    let matched = verdicts
        .iter()
        .filter(|verdict| **verdict == Verdict::True)
        .count();

    Witness {
        matched: u64::try_from(matched).unwrap_or(u64::MAX),
        answered: u64::try_from(result.rows.len()).unwrap_or(u64::MAX),
        columns,
        rows,
    }
}

#[cfg(test)]
mod tests;
