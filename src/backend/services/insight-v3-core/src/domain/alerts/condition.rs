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

pub(crate) const MAX_GROUP_DEPTH: usize = 3;
pub(crate) const MAX_LEAVES: usize = 20;
pub(crate) const MAX_LIST_VALUES: usize = 50;
pub(crate) const MAX_TEXT_CHARS: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Condition {
    pub(crate) rows: Quantifier,
    pub(crate) test: Node,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Quantifier {
    One,
    Any,
    All,
    None,
    Count { comparison: Comparison, value: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    All(Vec<Node>),
    Any(Vec<Node>),
    Not(Box<Node>),
    Leaf(Leaf),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Leaf {
    pub(crate) column: String,
    pub(crate) test: Test,
}

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
    Between {
        low: Number,
        high: Number,
    },
}

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
    Between {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    },
    OlderThan(Age),
    NewerThan(Age),
}

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
    pub(crate) fn one_value(column: &str, value: serde_json::Value, breached: bool) -> Self {
        Self {
            matched: u64::from(breached),
            answered: 1,
            columns: vec![column.to_owned()],
            rows: vec![vec![value]],
        }
    }
}
