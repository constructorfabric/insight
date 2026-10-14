//! What a rule asks of a metric's rows: a quantifier over the rows and a
//! tree of tests on each row's columns, parsed against the column kinds the
//! metric answers so every test is known to fit its column.

mod evaluate;
mod parse;
mod shape;
mod stored;
mod text;
mod witness;
mod write;

#[cfg(test)]
mod tests;

use chrono::{DateTime, TimeDelta, Utc};

use super::scalar::Number;

pub(crate) use evaluate::evaluate;
pub(crate) use parse::ConditionError;
#[cfg(test)]
pub(crate) use parse::Problem;
pub(crate) use shape::ConditionShape;
pub(crate) use witness::Witness;

// INVARIANT: these bounds only ever widen. A stored condition is read back
// through the same parse, so tightening one makes every row past it
// unreadable.

/// How deeply groups may nest, counting `all`, `any` and `not` alike.
pub(crate) const MAX_GROUP_DEPTH: usize = 3;
pub(crate) const MAX_LEAVES: usize = 20;
/// The longest `in` or `not_in` list.
pub(crate) const MAX_LIST_VALUES: usize = 50;
pub(crate) const MAX_TEXT_CHARS: usize = 256;

/// A rule's whole condition.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Condition {
    pub(crate) rows: Quantifier,
    pub(crate) test: Node,
}

/// Which rows must pass the test for the rule to be breached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Quantifier {
    /// Exactly one row, and it passes; any other row count is unknown.
    One,
    Any,
    All,
    None,
    /// The number of passing rows, compared.
    Count {
        comparison: Comparison,
        value: u64,
    },
}

/// A tree of tests on one row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    All(Vec<Node>),
    Any(Vec<Node>),
    Not(Box<Node>),
    Leaf(Leaf),
}

/// One test on one column.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Leaf {
    pub(crate) column: String,
    pub(crate) test: Test,
}

/// A test, shaped by the kind of column it reads.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Test {
    Number(NumberTest),
    Text(TextTest),
    Date(DateTest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Comparison {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum NumberTest {
    Compare {
        comparison: Comparison,
        threshold: Number,
    },
    /// Both ends inclusive; `low` is never above `high`.
    Between { low: Number, high: Number },
}

/// Text compares exactly and case-sensitively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TextTest {
    Is(String),
    IsNot(String),
    Contains(String),
    StartsWith(String),
    EndsWith(String),
    In(Vec<String>),
    NotIn(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DateTest {
    Before(DateTime<Utc>),
    After(DateTime<Utc>),
    /// Both ends inclusive; `from` is never after `to`.
    Between {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    },
    /// Earlier than the check's own time minus the age.
    OlderThan(Age),
    /// At or after the check's own time minus the age.
    NewerThan(Age),
}

/// A span measured back from a check, kept as written so it reads back the
/// same: `P7D`, `P2W`, `PT6H`, `PT30M`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Age {
    pub(crate) amount: u32,
    pub(crate) unit: AgeUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgeUnit {
    Minutes,
    Hours,
    Days,
    Weeks,
}

impl Comparison {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        [Self::Gt, Self::Ge, Self::Lt, Self::Le, Self::Eq, Self::Ne]
            .into_iter()
            .find(|comparison| comparison.as_str() == value)
    }
}

impl Age {
    /// `P<n>D`, `P<n>W`, `PT<n>H` or `PT<n>M`, with `n` above zero.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (body, timed) = match text.strip_prefix("PT") {
            Some(body) => (body, true),
            None => (text.strip_prefix('P')?, false),
        };
        let (digits, unit) = body.split_at_checked(body.len().checked_sub(1)?)?;
        if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let amount = digits.parse::<u32>().ok().filter(|amount| *amount > 0)?;

        let unit = match (timed, unit) {
            (true, "H") => AgeUnit::Hours,
            (true, "M") => AgeUnit::Minutes,
            (false, "D") => AgeUnit::Days,
            (false, "W") => AgeUnit::Weeks,
            _ => return None,
        };

        Some(Self { amount, unit })
    }

    pub(crate) fn span(self) -> TimeDelta {
        let amount = i64::from(self.amount);

        match self.unit {
            AgeUnit::Minutes => TimeDelta::minutes(amount),
            AgeUnit::Hours => TimeDelta::hours(amount),
            AgeUnit::Days => TimeDelta::days(amount),
            AgeUnit::Weeks => TimeDelta::weeks(amount),
        }
    }
}

impl Condition {
    /// Every column the test reads, in the order first named, once each.
    pub(crate) fn columns(&self) -> Vec<&str> {
        let mut named = Vec::new();
        self.test.collect_columns(&mut named);
        named
    }
}

impl Node {
    fn collect_columns<'a>(&'a self, into: &mut Vec<&'a str>) {
        match self {
            Self::All(children) | Self::Any(children) => {
                for child in children {
                    child.collect_columns(into);
                }
            }
            Self::Not(child) => child.collect_columns(into),
            Self::Leaf(leaf) => {
                if !into.contains(&leaf.column.as_str()) {
                    into.push(&leaf.column);
                }
            }
        }
    }
}

#[cfg(test)]
impl Condition {
    /// Exactly one row, and its `column` above `threshold`.
    pub(crate) fn one_above(column: &str, threshold: i64) -> Self {
        Self {
            rows: Quantifier::One,
            test: Node::Leaf(Leaf {
                column: column.to_owned(),
                test: Test::Number(NumberTest::Compare {
                    comparison: Comparison::Gt,
                    threshold: Number::Int(threshold.into()),
                }),
            }),
        }
    }
}

#[cfg(test)]
impl Witness {
    /// The one row a one-row check read, holding `column` alone.
    pub(crate) fn one_value(column: &str, value: serde_json::Value, breached: bool) -> Self {
        Self {
            matched: u64::from(breached),
            answered: 1,
            columns: vec![column.to_owned()],
            rows: vec![vec![value]],
        }
    }
}
