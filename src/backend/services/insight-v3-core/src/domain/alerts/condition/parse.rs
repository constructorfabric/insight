//! A condition as JSON, read into its typed tree against the columns the
//! metric answers. Every refusal names the path of the part it is about.

mod path;

use chrono::{DateTime, Datelike as _, Utc};
use serde_json::{Map, Value};
use thiserror::Error;

use super::{
    Age, Comparison, Condition, DateTest, Leaf, MAX_GROUP_DEPTH, MAX_LEAVES, MAX_LIST_VALUES,
    MAX_TEXT_CHARS, Node, NumberTest, Quantifier, Test, TextTest,
};
use crate::domain::alerts::scalar::Number;
use crate::domain::query::metric_query::ColumnKind;

use path::{At, ROOT};

/// The longest span `older_than` or `newer_than` may reach back.
const MAX_AGE_DAYS: u64 = 36_500;

const NUMBER_OPERATORS: &str = "`>`, `>=`, `<`, `<=`, `==`, `!=`, `between`";
const TEXT_OPERATORS: &str = "`==`, `!=`, `contains`, `starts_with`, `ends_with`, `in`, `not_in`";
const DATE_OPERATORS: &str = "`before`, `after`, `between`, `older_than`, `newer_than`";
const COUNT_OPERATORS: &str = "`>`, `>=`, `<`, `<=`, `==`, `!=`";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{path}: {problem}")]
pub(crate) struct ConditionError {
    /// Where in the condition, as `where.all[1].value`; empty for the whole.
    pub(crate) path: String,
    pub(crate) problem: Problem,
}

impl ConditionError {
    /// The draft field this is about: `condition`, then the path inside it,
    /// as `condition.where.all[1].value`.
    pub(crate) fn field(&self) -> String {
        if self.path.is_empty() {
            "condition".to_owned()
        } else {
            format!("condition.{}", self.path)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum Problem {
    #[error("must be an object")]
    NotObject,
    #[error("is required")]
    Missing,
    #[error("is not a key this object takes")]
    UnknownKey,
    #[error("must be `one`, `any`, `all`, `none`, or an object with `count`")]
    Rows,
    #[error("must be a whole number of rows, zero or more")]
    Count,
    #[error(
        "must be a group holding one of `all`, `any`, `not`, or a test holding `column`, `op` and `value`"
    )]
    Node,
    #[error("a group needs at least one condition")]
    EmptyGroup,
    #[error("groups nest at most {MAX_GROUP_DEPTH} deep")]
    TooDeep,
    #[error("a condition holds at most {MAX_LEAVES} tests")]
    TooManyTests,
    #[error("must be the name of a result column")]
    ColumnName,
    #[error("the metric answers no column `{0}`")]
    NoSuchColumn(String),
    #[error("must be an operator name")]
    OperatorName,
    #[error("`{operator}` does not apply to a {kind} column; it takes {allowed}")]
    Operator {
        operator: String,
        kind: &'static str,
        allowed: &'static str,
    },
    #[error("must be a finite JSON number, or an integer written as digits")]
    NumberValue,
    #[error("must be text of at most {MAX_TEXT_CHARS} characters")]
    TextValue,
    #[error("must be a list of 1 to {MAX_LIST_VALUES} texts")]
    ListValue,
    #[error("must be an RFC 3339 timestamp")]
    DateValue,
    #[error("must be a duration of up to 100 years, such as P7D, P2W, PT6H or PT30M")]
    AgeValue,
    #[error("must be a list of two values")]
    Pair,
    #[error("the first bound must not be above the second")]
    BoundsOrder,
    #[error("the bounds cannot be compared exactly")]
    BoundsIncomparable,
}

impl Condition {
    /// The condition a draft writes, checked against the metric's columns.
    pub(crate) fn parse(
        value: &Value,
        columns: &[(String, ColumnKind)],
    ) -> Result<Self, ConditionError> {
        let object = only_keys(value, &ROOT, &["rows", "where"])?;
        let rows = quantifier(required(object, &ROOT, "rows")?, &ROOT.key("rows"))?;

        let at = ROOT.key("where");
        let mut tests = 0;
        let test = node(
            required(object, &ROOT, "where")?,
            &at,
            0,
            &mut tests,
            columns,
        )?;

        Ok(Self { rows, test })
    }
}

fn quantifier(value: &Value, at: &At<'_>) -> Result<Quantifier, ConditionError> {
    if let Some(word) = value.as_str() {
        return match word {
            "one" => Ok(Quantifier::One),
            "any" => Ok(Quantifier::Any),
            "all" => Ok(Quantifier::All),
            "none" => Ok(Quantifier::None),
            _ => at.refuse(Problem::Rows),
        };
    }
    if !value.is_object() {
        return at.refuse(Problem::Rows);
    }

    let object = only_keys(value, at, &["count"])?;
    let written = required(object, at, "count")?;
    let at = at.key("count");
    let count = only_keys(written, &at, &["op", "value"])?;

    let op_at = at.key("op");
    let Some(op) = required(count, &at, "op")?.as_str() else {
        return op_at.refuse(Problem::OperatorName);
    };
    let Some(comparison) = Comparison::parse(op) else {
        return op_at.refuse(Problem::Operator {
            operator: op.to_owned(),
            kind: "row count",
            allowed: COUNT_OPERATORS,
        });
    };

    let Some(value) = required(count, &at, "value")?.as_u64() else {
        return at.key("value").refuse(Problem::Count);
    };

    Ok(Quantifier::Count { comparison, value })
}

fn node(
    value: &Value,
    at: &At<'_>,
    depth: usize,
    tests: &mut usize,
    columns: &[(String, ColumnKind)],
) -> Result<Node, ConditionError> {
    let Some(object) = value.as_object() else {
        return at.refuse(Problem::Node);
    };

    let group = ["all", "any", "not"]
        .into_iter()
        .find(|key| object.contains_key(*key));
    let Some(group) = group else {
        return leaf(value, at, tests, columns).map(Node::Leaf);
    };

    only_keys(value, at, &[group])?;
    if depth == MAX_GROUP_DEPTH {
        return at.refuse(Problem::TooDeep);
    }

    let at = at.key(group);
    let inner = &object[group];
    if group == "not" {
        let child = node(inner, &at, depth + 1, tests, columns)?;
        return Ok(Node::Not(Box::new(child)));
    }

    let Some(items) = inner.as_array() else {
        return at.refuse(Problem::Node);
    };
    if items.is_empty() {
        return at.refuse(Problem::EmptyGroup);
    }

    let children = items
        .iter()
        .enumerate()
        .map(|(index, item)| node(item, &at.index(index), depth + 1, tests, columns))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(if group == "all" {
        Node::All(children)
    } else {
        Node::Any(children)
    })
}

fn leaf(
    value: &Value,
    at: &At<'_>,
    tests: &mut usize,
    columns: &[(String, ColumnKind)],
) -> Result<Leaf, ConditionError> {
    let object = only_keys(value, at, &["column", "op", "value"])?;

    *tests += 1;
    if *tests > MAX_LEAVES {
        return at.refuse(Problem::TooManyTests);
    }

    let column_at = at.key("column");
    let Some(column) = required(object, at, "column")?.as_str() else {
        return column_at.refuse(Problem::ColumnName);
    };
    let Some(kind) = columns
        .iter()
        .find_map(|(name, kind)| (name == column).then_some(*kind))
    else {
        return column_at.refuse(Problem::NoSuchColumn(column.to_owned()));
    };

    let op_at = at.key("op");
    let Some(op) = required(object, at, "op")?.as_str() else {
        return op_at.refuse(Problem::OperatorName);
    };

    let value = required(object, at, "value")?;
    let value_at = at.key("value");
    let test = match kind {
        ColumnKind::Number => Test::Number(number_test(op, value, &op_at, &value_at)?),
        ColumnKind::Text => Test::Text(text_test(op, value, &op_at, &value_at)?),
        ColumnKind::Date => Test::Date(date_test(op, value, &op_at, &value_at)?),
    };

    Ok(Leaf {
        column: column.to_owned(),
        test,
    })
}

fn number_test(
    op: &str,
    value: &Value,
    op_at: &At<'_>,
    value_at: &At<'_>,
) -> Result<NumberTest, ConditionError> {
    if op == "between" {
        let (low, high) = pair(value, value_at, number)?;
        return match low.compare(high) {
            Some(std::cmp::Ordering::Greater) => value_at.refuse(Problem::BoundsOrder),
            Some(_) => Ok(NumberTest::Between { low, high }),
            None => value_at.refuse(Problem::BoundsIncomparable),
        };
    }

    let Some(comparison) = Comparison::parse(op) else {
        return op_at.refuse(wrong_operator(op, "number", NUMBER_OPERATORS));
    };

    Ok(NumberTest::Compare {
        comparison,
        threshold: number(value, value_at)?,
    })
}

fn text_test(
    op: &str,
    value: &Value,
    op_at: &At<'_>,
    value_at: &At<'_>,
) -> Result<TextTest, ConditionError> {
    let one = || text(value, value_at);

    Ok(match op {
        "==" => TextTest::Is(one()?),
        "!=" => TextTest::IsNot(one()?),
        "contains" => TextTest::Contains(one()?),
        "starts_with" => TextTest::StartsWith(one()?),
        "ends_with" => TextTest::EndsWith(one()?),
        "in" => TextTest::In(texts(value, value_at)?),
        "not_in" => TextTest::NotIn(texts(value, value_at)?),
        _ => return op_at.refuse(wrong_operator(op, "text", TEXT_OPERATORS)),
    })
}

fn date_test(
    op: &str,
    value: &Value,
    op_at: &At<'_>,
    value_at: &At<'_>,
) -> Result<DateTest, ConditionError> {
    Ok(match op {
        "before" => DateTest::Before(moment(value, value_at)?),
        "after" => DateTest::After(moment(value, value_at)?),
        "between" => {
            let (from, to) = pair(value, value_at, moment)?;
            if from > to {
                return value_at.refuse(Problem::BoundsOrder);
            }
            DateTest::Between { from, to }
        }
        "older_than" => DateTest::OlderThan(age(value, value_at)?),
        "newer_than" => DateTest::NewerThan(age(value, value_at)?),
        _ => return op_at.refuse(wrong_operator(op, "date", DATE_OPERATORS)),
    })
}

fn wrong_operator(op: &str, kind: &'static str, allowed: &'static str) -> Problem {
    Problem::Operator {
        operator: op.to_owned(),
        kind,
        allowed,
    }
}

fn number(value: &Value, at: &At<'_>) -> Result<Number, ConditionError> {
    Number::parse_written(value).map_or_else(|| at.refuse(Problem::NumberValue), Ok)
}

fn text(value: &Value, at: &At<'_>) -> Result<String, ConditionError> {
    match value.as_str() {
        Some(text) if text.chars().count() <= MAX_TEXT_CHARS => Ok(text.to_owned()),
        Some(_) | None => at.refuse(Problem::TextValue),
    }
}

fn texts(value: &Value, at: &At<'_>) -> Result<Vec<String>, ConditionError> {
    let Some(items) = value.as_array() else {
        return at.refuse(Problem::ListValue);
    };
    if items.is_empty() || items.len() > MAX_LIST_VALUES {
        return at.refuse(Problem::ListValue);
    }

    items
        .iter()
        .enumerate()
        .map(|(index, item)| text(item, &at.index(index)))
        .collect()
}

/// The years an RFC 3339 timestamp can be written back in once in UTC.
const WRITABLE_YEARS: std::ops::RangeInclusive<i32> = 0..=9999;

fn moment(value: &Value, at: &At<'_>) -> Result<DateTime<Utc>, ConditionError> {
    value
        .as_str()
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .filter(|moment| WRITABLE_YEARS.contains(&moment.to_utc().year()))
        .map_or_else(
            || at.refuse(Problem::DateValue),
            |moment| Ok(moment.to_utc()),
        )
}

fn age(value: &Value, at: &At<'_>) -> Result<Age, ConditionError> {
    value
        .as_str()
        .and_then(Age::parse)
        .filter(|age| age.span().num_days().unsigned_abs() <= MAX_AGE_DAYS)
        .map_or_else(|| at.refuse(Problem::AgeValue), Ok)
}

fn pair<T>(
    value: &Value,
    at: &At<'_>,
    each: impl Fn(&Value, &At<'_>) -> Result<T, ConditionError>,
) -> Result<(T, T), ConditionError> {
    let Some([first, second]) = value.as_array().map(Vec::as_slice) else {
        return at.refuse(Problem::Pair);
    };

    Ok((each(first, &at.index(0))?, each(second, &at.index(1))?))
}

fn required<'v>(
    object: &'v Map<String, Value>,
    at: &At<'_>,
    key: &str,
) -> Result<&'v Value, ConditionError> {
    object
        .get(key)
        .map_or_else(|| at.key(key).refuse(Problem::Missing), Ok)
}

/// The object, once it is one holding no key outside `allowed`.
fn only_keys<'v>(
    value: &'v Value,
    at: &At<'_>,
    allowed: &[&str],
) -> Result<&'v Map<String, Value>, ConditionError> {
    let Some(object) = value.as_object() else {
        return at.refuse(Problem::NotObject);
    };
    if let Some(stray) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return at.key(stray).refuse(Problem::UnknownKey);
    }

    Ok(object)
}
