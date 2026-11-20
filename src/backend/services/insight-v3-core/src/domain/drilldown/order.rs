use std::fmt::Write as _;

use super::cursor::{CursorError, CursorKey, KeyValue};
use super::{Column, Sort};
use crate::domain::query::metric_query::{ColumnKind, FilterBind, RECORD_COLUMN, TWIN_COLUMN};

pub(super) const INNER: &str = "__m";
const FLAG: &str = "__drilldown_flag";
const KEY: &str = "__drilldown_key";
const TIE: &str = "__drilldown_tie";

pub(super) fn is_reserved(name: &str) -> bool {
    name == FLAG
        || name == KEY
        || name == RECORD_COLUMN
        || name == TWIN_COLUMN
        || name.starts_with(TIE)
}

#[derive(Debug)]
pub(super) struct OrderKey {
    key: String,
    kind: ColumnKind,
    descending: bool,
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

    fn number(&self) -> String {
        format!("toFloat64OrNull(toString({}))", Self::column(&self.key))
    }

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

    fn flag_sql(&self) -> String {
        let blank = self.blank_sql();

        if self.descending {
            format!("toUInt8(NOT {blank})")
        } else {
            format!("toUInt8({blank})")
        }
    }

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
