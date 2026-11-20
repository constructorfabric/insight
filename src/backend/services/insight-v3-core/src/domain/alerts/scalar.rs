use std::cmp::Ordering;
use std::fmt;

use serde::Serialize;

use super::condition::Witness;

const SIGNIFICANT_DIGITS: i32 = 6;
const MAX_DECIMALS: i32 = 12;

const EXACT_FLOAT_BOUND: u128 = 1 << 53;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Number {
    Int(i128),
    Float(f64),
}

impl Number {
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

    pub(crate) fn parse_written(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::String(text) => integer_literal(text).map(Self::Int),
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::Array(_)
            | serde_json::Value::Object(_) => Self::parse(value),
        }
    }

    pub(crate) fn compare(self, other: Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Int(left), Self::Int(right)) => Some(left.cmp(&right)),
            (Self::Float(left), Self::Float(right)) => left.partial_cmp(&right),
            (Self::Int(int), Self::Float(float)) => as_exact_float(int)?.partial_cmp(&float),
            (Self::Float(float), Self::Int(int)) => float.partial_cmp(&as_exact_float(int)?),
        }
    }

    pub(crate) fn to_json(self) -> serde_json::Value {
        match self {
            Self::Int(int) => i64::try_from(int)
                .ok()
                .filter(|_| is_safe_integer(int))
                .map_or_else(
                    || serde_json::Value::String(int.to_string()),
                    serde_json::Value::from,
                ),
            Self::Float(float) => serde_json::Number::from_f64(float)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
        }
    }

    pub(crate) fn exact(self) -> String {
        match self {
            Self::Int(int) => int.to_string(),
            Self::Float(float) => javascript_float(float),
        }
    }

    pub(crate) fn readable(self) -> String {
        let Self::Float(float) = self else {
            return self.to_string();
        };
        if float == 0.0 {
            return "0".to_owned();
        }

        #[expect(
            clippy::cast_possible_truncation,
            reason = "log10 of a finite f64 fits in i32"
        )]
        let magnitude = float.abs().log10().floor() as i32;
        let decimals = usize::try_from((SIGNIFICANT_DIGITS - 1 - magnitude).clamp(0, MAX_DECIMALS))
            .unwrap_or_default();
        let written = format!("{float:.decimals$}");
        let trimmed = if written.contains('.') {
            written.trim_end_matches('0').trim_end_matches('.')
        } else {
            written.as_str()
        };

        trimmed.to_owned()
    }

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

fn javascript_float(float: f64) -> String {
    if float == 0.0 {
        return "0".to_owned();
    }

    let scientific = format!("{:e}", float.abs());
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let Ok(exponent) = exponent.parse::<i32>() else {
        return scientific;
    };
    let count = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    let point = exponent + 1;
    let sign = if float < 0.0 { "-" } else { "" };

    let written = if count <= point && point <= 21 {
        let zeros = usize::try_from(point - count).unwrap_or_default();
        format!("{digits}{}", "0".repeat(zeros))
    } else if 0 < point && point <= 21 {
        let (whole, fraction) = digits.split_at(usize::try_from(point).unwrap_or_default());
        format!("{whole}.{fraction}")
    } else if -6 < point && point <= 0 {
        let zeros = usize::try_from(-point).unwrap_or_default();
        format!("0.{}{digits}", "0".repeat(zeros))
    } else {
        let shown = exponent;
        let exponent_sign = if shown >= 0 { "+" } else { "-" };
        let (first, rest) = digits.split_at(1);
        let mantissa = if rest.is_empty() {
            first.to_owned()
        } else {
            format!("{first}.{rest}")
        };
        format!("{mantissa}e{exponent_sign}{}", shown.unsigned_abs())
    };

    format!("{sign}{written}")
}

fn as_exact_float(int: i128) -> Option<f64> {
    if int.unsigned_abs() > EXACT_FLOAT_BOUND {
        return None;
    }

    #[expect(clippy::cast_precision_loss, reason = "guarded by EXACT_FLOAT_BOUND")]
    Some(int as f64)
}

/// Whether `JSON.parse` keeps the integer as it is: `Number.isSafeInteger`.
fn is_safe_integer(int: i128) -> bool {
    int.unsigned_abs() < EXACT_FLOAT_BOUND
}

fn integer_literal(text: &str) -> Option<i128> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    text.parse().ok()
}

impl fmt::Display for Number {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(int) => write!(formatter, "{int}"),
            Self::Float(float) => write!(formatter, "{float}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnknownReason {
    NoRows,
    ManyRows,
    ColumnMissing,
    Null,
    NonNumeric,
    NonText,
    NonDate,
    Incomparable,
    RowLimit,
    MetricMissing,
    CompileFailed,
    RunFailed,
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
            Self::NonText => "non_text",
            Self::NonDate => "non_date",
            Self::Incomparable => "incomparable",
            Self::RowLimit => "row_limit",
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
            Self::NonText,
            Self::NonDate,
            Self::Incomparable,
            Self::RowLimit,
            Self::MetricMissing,
            Self::CompileFailed,
            Self::RunFailed,
            Self::Timeout,
        ]
        .into_iter()
        .find(|reason| reason.as_str() == value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Valid { breached: bool, witness: Witness },
    Unknown(UnknownReason),
}

impl Outcome {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Valid { breached: true, .. } => "breach",
            Self::Valid {
                breached: false, ..
            } => "no_breach",
            Self::Unknown(_) => "unknown",
        }
    }

    pub(crate) fn witness(&self) -> Option<&Witness> {
        match self {
            Self::Valid { witness, .. } => Some(witness),
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
