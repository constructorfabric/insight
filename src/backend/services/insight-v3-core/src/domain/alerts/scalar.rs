//! The one number a check reads out of a metric's rows, and whether it can
//! be compared at all.

use std::cmp::Ordering;
use std::fmt;

use serde::Serialize;

use super::rule::Condition;
use crate::domain::query::metric_query::RunResult;

/// Integers this large and larger are no longer exact as `f64`, so comparing
/// one with a float would be comparing a rounded value.
const EXACT_FLOAT_BOUND: i128 = 1 << 53;

/// A number a metric produced or a threshold names, kept as what it is: an
/// integer is never rounded through a float on its way to a comparison.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Number {
    Int(i128),
    Float(f64),
}

impl Number {
    /// A JSON number, or nothing for anything else — including a numeric
    /// string, which the warehouse sends for wide integers and which
    /// [`crate::domain::query::metric_query::runner`] has already turned into
    /// a number where the column type said to.
    pub(crate) fn parse(value: &serde_json::Value) -> Option<Self> {
        let number = value.as_number()?;
        if let Some(int) = number.as_i64() {
            return Some(Self::Int(i128::from(int)));
        }
        if let Some(int) = number.as_u64() {
            return Some(Self::Int(i128::from(int)));
        }

        number
            .as_f64()
            .filter(|float| float.is_finite())
            .map(Self::Float)
    }

    /// How this compares with `other`, or nothing where the comparison would
    /// be between a rounded value and an exact one.
    pub(crate) fn compare(self, other: Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Int(left), Self::Int(right)) => Some(left.cmp(&right)),
            (Self::Float(left), Self::Float(right)) => left.partial_cmp(&right),
            (Self::Int(int), Self::Float(float)) => as_exact_float(int)?.partial_cmp(&float),
            (Self::Float(float), Self::Int(int)) => float.partial_cmp(&as_exact_float(int)?),
        }
    }

    /// The number as JSON carries it.
    pub(crate) fn to_json(self) -> serde_json::Value {
        match self {
            Self::Int(int) => i64::try_from(int).map_or_else(
                |_| serde_json::Value::String(int.to_string()),
                serde_json::Value::from,
            ),
            Self::Float(float) => serde_json::Number::from_f64(float)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
        }
    }

    /// The number as a row stores it, which is also how it is parsed back.
    pub(crate) fn to_stored(self) -> String {
        match self {
            Self::Int(int) => int.to_string(),
            Self::Float(float) => format!("{float:?}"),
        }
    }

    /// A number as [`Number::to_stored`] wrote it.
    pub(crate) fn from_stored(value: &str) -> Option<Self> {
        if let Ok(int) = value.parse::<i128>() {
            return Some(Self::Int(int));
        }

        value
            .parse::<f64>()
            .ok()
            .filter(|float| float.is_finite())
            .map(Self::Float)
    }
}

fn as_exact_float(int: i128) -> Option<f64> {
    if int.abs() >= EXACT_FLOAT_BOUND {
        return None;
    }

    // SAFETY: bounded above by 2^53, so the conversion is exact.
    #[expect(clippy::cast_precision_loss, reason = "guarded by EXACT_FLOAT_BOUND")]
    Some(int as f64)
}

impl fmt::Display for Number {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(int) => write!(formatter, "{int}"),
            Self::Float(float) => write!(formatter, "{float}"),
        }
    }
}

/// Why a check could not say whether the condition is met.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnknownReason {
    /// The metric answered no rows.
    NoRows,
    /// The metric answered more than one row, and no row is the one.
    ManyRows,
    /// The rule names a column the metric does not produce.
    ColumnMissing,
    /// The value is null.
    Null,
    /// The value is not a number.
    NonNumeric,
    /// The value is a number too wide to compare with the threshold exactly.
    Incomparable,
    /// The rule names a metric that is not stored.
    MetricMissing,
    /// The metric's body or window could not be compiled.
    CompileFailed,
    /// The warehouse did not answer.
    RunFailed,
    /// The warehouse did not answer in time.
    Timeout,
}

impl UnknownReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoRows => "no_rows",
            Self::ManyRows => "many_rows",
            Self::ColumnMissing => "column_missing",
            Self::Null => "null",
            Self::NonNumeric => "non_numeric",
            Self::Incomparable => "incomparable",
            Self::MetricMissing => "metric_missing",
            Self::CompileFailed => "compile_failed",
            Self::RunFailed => "run_failed",
            Self::Timeout => "timeout",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        [
            Self::NoRows,
            Self::ManyRows,
            Self::ColumnMissing,
            Self::Null,
            Self::NonNumeric,
            Self::Incomparable,
            Self::MetricMissing,
            Self::CompileFailed,
            Self::RunFailed,
            Self::Timeout,
        ]
        .into_iter()
        .find(|reason| reason.as_str() == value)
    }
}

/// What one check found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Outcome {
    /// One number, compared: the condition is met or it is not.
    Valid { value: Number, breached: bool },
    /// No number to compare, for the reason given.
    Unknown(UnknownReason),
}

impl Outcome {
    /// The word a row and a reader see.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Valid { breached: true, .. } => "breach",
            Self::Valid {
                breached: false, ..
            } => "no_breach",
            Self::Unknown(_) => "unknown",
        }
    }

    pub(crate) fn value(&self) -> Option<Number> {
        match self {
            Self::Valid { value, .. } => Some(*value),
            Self::Unknown(_) => None,
        }
    }

    pub(crate) fn reason(&self) -> Option<UnknownReason> {
        match self {
            Self::Valid { .. } => None,
            Self::Unknown(reason) => Some(*reason),
        }
    }
}

/// The outcome of comparing what a metric answered with a condition.
///
/// Exactly one row and exactly the named column count; anything else is
/// unknown rather than a guess at which row was meant.
pub(crate) fn classify(result: &RunResult, column: &str, condition: &Condition) -> Outcome {
    let Some(position) = result.columns.iter().position(|name| name == column) else {
        return Outcome::Unknown(UnknownReason::ColumnMissing);
    };

    let row = match result.rows.as_slice() {
        [] => return Outcome::Unknown(UnknownReason::NoRows),
        [row] => row,
        _ => return Outcome::Unknown(UnknownReason::ManyRows),
    };

    let cell = row.get(position).unwrap_or(&serde_json::Value::Null);
    if cell.is_null() {
        return Outcome::Unknown(UnknownReason::Null);
    }
    let Some(value) = Number::parse(cell) else {
        return Outcome::Unknown(UnknownReason::NonNumeric);
    };

    match condition.holds(value) {
        Some(breached) => Outcome::Valid { value, breached },
        None => Outcome::Unknown(UnknownReason::Incomparable),
    }
}
