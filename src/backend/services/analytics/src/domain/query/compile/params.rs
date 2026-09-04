//! Bound values, in the spelling ClickHouse receives them in.
//!
//! SAFETY: every caller-supplied value in a compiled statement is one of these.
//! Nothing here is ever written into the SQL text.

use crate::domain::query::contract::dto::ScalarDto;

#[derive(Debug, Clone, PartialEq)]
pub enum QueryParam {
    Text(String),
    Int(i64),
    UInt(u64),
    Float(f64),
}

impl QueryParam {
    /// A dimension compares as text, whatever the column's own type is.
    pub fn text_of(scalar: &ScalarDto) -> Self {
        Self::Text(match scalar {
            ScalarDto::Bool(value) => value.to_string(),
            ScalarDto::Number(value) => value.to_string(),
            ScalarDto::Text(value) => value.clone(),
        })
    }

    /// A measurable compares against the numeric column itself.
    pub fn number_of(scalar: &ScalarDto) -> Self {
        let ScalarDto::Number(number) = scalar else {
            return Self::text_of(scalar);
        };
        if let Some(value) = number.as_i64() {
            Self::Int(value)
        } else if let Some(value) = number.as_u64() {
            Self::UInt(value)
        } else if let Some(value) = number.as_f64() {
            Self::Float(value)
        } else {
            Self::Text(number.to_string())
        }
    }

    pub fn binding(&self) -> serde_json::Value {
        match self {
            Self::Text(value) => serde_json::Value::String(value.clone()),
            Self::Int(value) => serde_json::Value::from(*value),
            Self::UInt(value) => serde_json::Value::from(*value),
            Self::Float(value) => serde_json::Number::from_f64(*value)
                .map_or_else(serde_json::Value::default, serde_json::Value::Number),
        }
    }
}

pub fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn number(raw: &str) -> ScalarDto {
        ScalarDto::Number(raw.parse().expect("a JSON number"))
    }

    #[test]
    fn a_dimension_value_binds_as_the_text_the_answer_reports() {
        assert_eq!(
            QueryParam::text_of(&ScalarDto::Text("github".to_owned())),
            QueryParam::Text("github".to_owned())
        );
        assert_eq!(
            QueryParam::text_of(&number("42")),
            QueryParam::Text("42".to_owned())
        );
        assert_eq!(
            QueryParam::text_of(&ScalarDto::Bool(true)),
            QueryParam::Text("true".to_owned())
        );
    }

    #[test]
    fn a_measurable_value_keeps_the_numeric_width_it_was_written_in() {
        assert_eq!(QueryParam::number_of(&number("500")), QueryParam::Int(500));
        assert_eq!(QueryParam::number_of(&number("-1")), QueryParam::Int(-1));
        assert_eq!(
            QueryParam::number_of(&number("18446744073709551615")),
            QueryParam::UInt(u64::MAX)
        );
        assert_eq!(
            QueryParam::number_of(&number("1.5")),
            QueryParam::Float(1.5)
        );
    }

    #[test]
    fn a_bound_value_reaches_the_driver_with_its_own_json_type() {
        assert!(QueryParam::Int(5).binding().is_number());
        assert!(QueryParam::Text("5".to_owned()).binding().is_string());
    }

    #[test]
    fn placeholders_are_one_per_value() {
        assert_eq!(placeholders(1), "?");
        assert_eq!(placeholders(3), "?, ?, ?");
    }
}
