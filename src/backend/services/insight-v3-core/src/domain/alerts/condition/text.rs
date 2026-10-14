//! A condition in the words a recipient reads: "open above 10 and (team is
//! core or stale above 3)".

use std::fmt;

use chrono::{DateTime, Utc};

use super::{
    Age, AgeUnit, Comparison, Condition, DateTest, Leaf, Node, NumberTest, Quantifier, Test,
    TextTest,
};

impl fmt::Display for Condition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.rows {
            Quantifier::One => {}
            Quantifier::Any => formatter.write_str("any row: ")?,
            Quantifier::All => formatter.write_str("every row: ")?,
            Quantifier::None => formatter.write_str("no row: ")?,
            Quantifier::Count { comparison, value } => write!(
                formatter,
                "number of matching rows {} {value}: ",
                comparison_words(comparison)
            )?,
        }

        write!(formatter, "{}", self.test)
    }
}

impl fmt::Display for Node {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All(children) => joined(formatter, children, " and "),
            Self::Any(children) => joined(formatter, children, " or "),
            Self::Not(child) => write!(formatter, "not ({child})"),
            Self::Leaf(leaf) => write!(formatter, "{leaf}"),
        }
    }
}

/// Children joined by a word; a group inside a group keeps its parentheses.
fn joined(formatter: &mut fmt::Formatter<'_>, children: &[Node], word: &str) -> fmt::Result {
    for (index, child) in children.iter().enumerate() {
        if index > 0 {
            formatter.write_str(word)?;
        }
        match child {
            Node::All(_) | Node::Any(_) => write!(formatter, "({child})")?,
            Node::Not(_) | Node::Leaf(_) => write!(formatter, "{child}")?,
        }
    }

    Ok(())
}

impl fmt::Display for Leaf {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let column = &self.column;

        match &self.test {
            Test::Number(NumberTest::Compare {
                comparison,
                threshold,
            }) => write!(
                formatter,
                "{column} {} {}",
                comparison_words(*comparison),
                threshold.exact()
            ),
            Test::Number(NumberTest::Between { low, high }) => write!(
                formatter,
                "{column} between {} and {}",
                low.exact(),
                high.exact()
            ),
            Test::Text(test) => write!(formatter, "{column} {}", text_words(test)),
            Test::Date(test) => write!(formatter, "{column} {}", date_words(*test)),
        }
    }
}

fn comparison_words(comparison: Comparison) -> &'static str {
    match comparison {
        Comparison::Gt => "above",
        Comparison::Ge => "at or above",
        Comparison::Lt => "below",
        Comparison::Le => "at or below",
        Comparison::Eq => "is",
        Comparison::Ne => "is not",
    }
}

fn text_words(test: &TextTest) -> String {
    match test {
        TextTest::Is(value) => format!("is {value}"),
        TextTest::IsNot(value) => format!("is not {value}"),
        TextTest::Contains(value) => format!("contains {value}"),
        TextTest::StartsWith(value) => format!("starts with {value}"),
        TextTest::EndsWith(value) => format!("ends with {value}"),
        TextTest::In(values) => format!("is one of {}", values.join(", ")),
        TextTest::NotIn(values) => format!("is none of {}", values.join(", ")),
    }
}

fn date_words(test: DateTest) -> String {
    match test {
        DateTest::Before(at) => format!("before {}", stamp(at)),
        DateTest::After(at) => format!("after {}", stamp(at)),
        DateTest::Between { from, to } => format!("between {} and {}", stamp(from), stamp(to)),
        DateTest::OlderThan(age) => format!("older than {}", age_words(age)),
        DateTest::NewerThan(age) => format!("newer than {}", age_words(age)),
    }
}

fn stamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn age_words(age: Age) -> String {
    let unit = match age.unit {
        AgeUnit::Minutes => "minute",
        AgeUnit::Hours => "hour",
        AgeUnit::Days => "day",
        AgeUnit::Weeks => "week",
    };
    let plural = if age.amount == 1 { "" } else { "s" };

    format!("{} {unit}{plural}", age.amount)
}
