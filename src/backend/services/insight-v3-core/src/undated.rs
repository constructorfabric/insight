//! How much of what a metric reads carries no clock at all.

use serde::Deserialize;

/// `ClickHouse`'s `JSON` format writes 64-bit integers as strings, so the
/// count arrives either way round.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Number {
    Text(String),
    Value(i64),
}

impl Number {
    fn value(&self) -> Option<i64> {
        match self {
            Self::Text(text) => text.parse().ok(),
            Self::Value(value) => Some(*value),
        }
    }
}

#[derive(Debug, Deserialize)]
struct UndatedRow {
    undated: Option<Number>,
}

#[derive(Debug, Deserialize)]
struct UndatedAnswer {
    data: Vec<UndatedRow>,
}

/// How many of a metric's rows carry no clock.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct UndatedCount {
    count: u64,
}

impl UndatedCount {
    pub(crate) fn parse(body: &[u8]) -> Result<Self, serde_json::Error> {
        let answer: UndatedAnswer = serde_json::from_slice(body)?;
        let Some(row) = answer.data.first() else {
            return Ok(Self::default());
        };

        Ok(Self {
            count: row
                .undated
                .as_ref()
                .and_then(Number::value)
                .unwrap_or_default()
                .unsigned_abs(),
        })
    }

    pub(crate) fn count(self) -> u64 {
        self.count
    }
}

#[cfg(test)]
#[path = "undated/tests.rs"]
mod tests;
