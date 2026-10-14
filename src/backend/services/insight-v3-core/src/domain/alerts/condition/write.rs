//! A condition written back as the JSON a draft takes, so what a reader is
//! shown can be sent again unchanged.

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};

use super::{
    Age, AgeUnit, Condition, DateTest, Leaf, Node, NumberTest, Quantifier, Test, TextTest,
};

impl Condition {
    pub(crate) fn to_json(&self) -> Value {
        json!({ "rows": rows(self.rows), "where": node(&self.test) })
    }
}

fn rows(quantifier: Quantifier) -> Value {
    match quantifier {
        Quantifier::One => json!("one"),
        Quantifier::Any => json!("any"),
        Quantifier::All => json!("all"),
        Quantifier::None => json!("none"),
        Quantifier::Count { comparison, value } => {
            json!({ "count": { "op": comparison.as_str(), "value": value } })
        }
    }
}

fn node(node: &Node) -> Value {
    match node {
        Node::All(children) => {
            json!({ "all": children.iter().map(self::node).collect::<Vec<_>>() })
        }
        Node::Any(children) => {
            json!({ "any": children.iter().map(self::node).collect::<Vec<_>>() })
        }
        Node::Not(child) => json!({ "not": self::node(child) }),
        Node::Leaf(leaf) => self::leaf(leaf),
    }
}

fn leaf(leaf: &Leaf) -> Value {
    let (op, value) = match &leaf.test {
        Test::Number(test) => number(*test),
        Test::Text(test) => text(test),
        Test::Date(test) => date(*test),
    };

    json!({ "column": leaf.column, "op": op, "value": value })
}

fn number(test: NumberTest) -> (&'static str, Value) {
    match test {
        NumberTest::Compare {
            comparison,
            threshold,
        } => (comparison.as_str(), threshold.to_json()),
        NumberTest::Between { low, high } => ("between", json!([low.to_json(), high.to_json()])),
    }
}

fn text(test: &TextTest) -> (&'static str, Value) {
    match test {
        TextTest::Is(value) => ("==", json!(value)),
        TextTest::IsNot(value) => ("!=", json!(value)),
        TextTest::Contains(value) => ("contains", json!(value)),
        TextTest::StartsWith(value) => ("starts_with", json!(value)),
        TextTest::EndsWith(value) => ("ends_with", json!(value)),
        TextTest::In(values) => ("in", json!(values)),
        TextTest::NotIn(values) => ("not_in", json!(values)),
    }
}

fn date(test: DateTest) -> (&'static str, Value) {
    match test {
        DateTest::Before(at) => ("before", json!(moment(at))),
        DateTest::After(at) => ("after", json!(moment(at))),
        DateTest::Between { from, to } => ("between", json!([moment(from), moment(to)])),
        DateTest::OlderThan(age) => ("older_than", json!(age.to_string())),
        DateTest::NewerThan(age) => ("newer_than", json!(age.to_string())),
    }
}

fn moment(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

impl std::fmt::Display for Age {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let amount = self.amount;
        match self.unit {
            AgeUnit::Minutes => write!(formatter, "PT{amount}M"),
            AgeUnit::Hours => write!(formatter, "PT{amount}H"),
            AgeUnit::Days => write!(formatter, "P{amount}D"),
            AgeUnit::Weeks => write!(formatter, "P{amount}W"),
        }
    }
}
