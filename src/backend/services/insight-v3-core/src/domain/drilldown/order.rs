//! The order a page's rows travel in, and everything that has to agree with
//! it: the `ORDER BY`, the columns the cursor replays, and how a replayed key
//! re-enters the comparison.

use std::fmt::Write as _;

use super::cursor::{CursorError, CursorKey, KeyValue};
use super::{Column, Sort};
use crate::domain::query::metric_query::{ColumnKind, FilterBind, RECORD_COLUMN, TWIN_COLUMN};

/// The alias the metric's own query is read under.
pub(super) const INNER: &str = "__m";
const FLAG: &str = "__drilldown_flag";
const KEY: &str = "__drilldown_key";
const TIE: &str = "__drilldown_tie";

/// Whether a metric's column would collide with what the wrapper writes
/// beside it.
pub(super) fn is_reserved(name: &str) -> bool {
    name == FLAG
        || name == KEY
        || name == RECORD_COLUMN
        || name == TWIN_COLUMN
        || name.starts_with(TIE)
}

/// INVARIANT: one direction for the whole key. Blank cells are pushed last
/// by the leading flag, which flips with the direction, rather than by a
/// second direction, so the next page stays one tuple comparison instead of
/// a chain of them.
#[derive(Debug)]
pub(super) struct OrderKey {
    key: String,
    kind: ColumnKind,
    descending: bool,
    /// Every column as text - the sorted one too, so two rows its number
    /// form cannot tell apart still are - then whatever the query carries to
    /// tell rows apart that the columns cannot.
    ties: Vec<String>,
}

impl OrderKey {
    pub(super) fn build(columns: &[Column], sort: &Sort, hidden: &[String]) -> Self {
        let kind = columns
            .iter()
            .find(|column| column.key == sort.key)
            .map_or(ColumnKind::Text, |column| column.kind);
        let ties = columns
            .iter()
            .map(|column| column.key.clone())
            .chain(hidden.iter().cloned())
            .collect();

        Self {
            key: sort.key.clone(),
            kind,
            descending: sort.descending,
            ties,
        }
    }

    fn column(name: &str) -> String {
        format!("{INNER}.`{name}`")
    }

    /// The sorted cell as a number, and null where it holds none.
    fn number(&self) -> String {
        format!("toFloat64OrNull(toString({}))", Self::column(&self.key))
    }

    /// Whether the sorted cell is blank: null, empty, or - for a number -
    /// not a finite one.
    fn blank_sql(&self) -> String {
        match self.kind {
            ColumnKind::Number => {
                let number = self.number();

                format!("(isNull({number}) OR NOT isFinite({number}))")
            }
            ColumnKind::Text | ColumnKind::Date => {
                let column = Self::column(&self.key);

                format!("(isNull({column}) OR toString({column}) = '')")
            }
        }
    }

    /// The blank flag as a number the tuple can compare, inverted for a
    /// downward order so a blank sorts past every filled cell either way.
    fn flag_sql(&self) -> String {
        let blank = self.blank_sql();

        if self.descending {
            format!("toUInt8(NOT {blank})")
        } else {
            format!("toUInt8({blank})")
        }
    }

    /// The sorted cell in a form with no null in it: the flag has already
    /// said where a blank goes, so what stands in for it only has to compare.
    fn key_sql(&self) -> String {
        match self.kind {
            ColumnKind::Number => format!("ifNotFinite(ifNull({}, 0), 0)", self.number()),
            ColumnKind::Text | ColumnKind::Date => {
                format!("ifNull(toString({}), '')", Self::column(&self.key))
            }
        }
    }

    fn tie_sql(name: &str) -> String {
        format!("ifNull(toString({}), '')", Self::column(name))
    }

    /// The key's elements as the page carries them out, so the cursor holds
    /// exactly what the warehouse compared and not a re-spelling of it.
    pub(super) fn projection(&self) -> String {
        let mut sql = format!(
            ", {} AS `{FLAG}`, {} AS `{KEY}`",
            self.flag_sql(),
            self.key_sql()
        );
        for (index, tie) in self.ties.iter().enumerate() {
            let _ = write!(sql, ", {} AS `{TIE}{index}`", Self::tie_sql(tie));
        }

        sql
    }

    pub(super) fn order_by(&self) -> String {
        let direction = if self.descending { "DESC" } else { "ASC" };
        let mut sql = format!("`{FLAG}` {direction}, `{KEY}` {direction}");
        for index in 0..self.ties.len() {
            let _ = write!(sql, ", `{TIE}{index}` {direction}");
        }

        sql
    }

    /// Refuses a key this order could not have issued. The wrong kind of
    /// value, or a tail of the wrong length, would reach the warehouse as a
    /// comparison it refuses - and that refusal would read as ours.
    pub(super) fn check(&self, key: &CursorKey) -> Result<(), CursorError> {
        let fits = match (self.kind, &key.value) {
            (ColumnKind::Number, KeyValue::Number(number)) => number.is_finite(),
            (ColumnKind::Text | ColumnKind::Date, KeyValue::Text(_)) => true,
            (ColumnKind::Number, KeyValue::Text(_))
            | (ColumnKind::Text | ColumnKind::Date, KeyValue::Number(_)) => false,
        };
        if !fits || key.ties.len() != self.ties.len() {
            return Err(CursorError::Malformed);
        }

        Ok(())
    }

    /// The rows past the one the cursor holds, in this order.
    ///
    /// INVARIANT: bound in the order the tuple names its elements, after
    /// everything the inner query binds.
    pub(super) fn cursor_predicate(&self, key: &CursorKey, binds: &mut Vec<FilterBind>) -> String {
        let mut elements = vec![self.flag_sql(), self.key_sql()];
        elements.extend(self.ties.iter().map(|tie| Self::tie_sql(tie)));

        binds.push(FilterBind::Bool(key.flag));
        binds.push(match &key.value {
            KeyValue::Number(number) => FilterBind::Float(*number),
            KeyValue::Text(text) => FilterBind::Str(text.clone()),
        });
        binds.extend(key.ties.iter().map(|tie| FilterBind::Str(tie.clone())));

        let placeholders = vec!["?"; elements.len()].join(", ");
        let operator = if self.descending { "<" } else { ">" };

        format!(
            "tuple({}) {operator} tuple({placeholders})",
            elements.join(", ")
        )
    }

    /// The key of a row the page carried out, read off the projected cells.
    pub(super) fn key_of(
        &self,
        row: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CursorKey, CursorError> {
        let flag = match row.get(FLAG) {
            Some(serde_json::Value::Number(number)) => number.as_u64() == Some(1),
            Some(serde_json::Value::String(text)) => text == "1",
            _ => return Err(CursorError::Malformed),
        };
        let value = match self.kind {
            ColumnKind::Number => match row.get(KEY) {
                Some(serde_json::Value::Number(number)) => {
                    KeyValue::Number(number.as_f64().unwrap_or_default())
                }
                Some(serde_json::Value::String(text)) => {
                    KeyValue::Number(text.parse().unwrap_or_default())
                }
                _ => return Err(CursorError::Malformed),
            },
            ColumnKind::Text | ColumnKind::Date => match row.get(KEY) {
                Some(serde_json::Value::String(text)) => KeyValue::Text(text.clone()),
                _ => return Err(CursorError::Malformed),
            },
        };
        let mut ties = Vec::with_capacity(self.ties.len());
        for index in 0..self.ties.len() {
            match row.get(&format!("{TIE}{index}")) {
                Some(serde_json::Value::String(text)) => ties.push(text.clone()),
                _ => return Err(CursorError::Malformed),
            }
        }

        Ok(CursorKey { flag, value, ties })
    }
}

/// The row as a reader sees it: the key's working cells and the hidden
/// columns gone, and each number a number rather than the text the warehouse
/// writes a wide integer as.
pub(super) fn presented(
    mut row: serde_json::Map<String, serde_json::Value>,
    columns: &[Column],
    hidden: &[String],
) -> serde_json::Map<String, serde_json::Value> {
    row.remove(FLAG);
    row.remove(KEY);
    row.retain(|name, _| !name.starts_with(TIE));
    for name in hidden {
        row.remove(name);
    }

    for column in columns {
        if column.kind != ColumnKind::Number {
            continue;
        }
        if let Some(serde_json::Value::String(text)) = row.get(&column.key) {
            let number = text
                .parse::<i64>()
                .map(serde_json::Number::from)
                .ok()
                .or_else(|| {
                    text.parse::<f64>()
                        .ok()
                        .and_then(serde_json::Number::from_f64)
                });
            if let Some(number) = number {
                row.insert(column.key.clone(), serde_json::Value::Number(number));
            }
        }
    }

    row
}
