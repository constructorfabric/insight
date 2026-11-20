use serde_json::{Value, json};

use crate::domain::alerts::scalar::Number;

pub(crate) const MAX_WITNESS_ROWS: usize = 5;
pub(crate) const MAX_WITNESS_CELL_CHARS: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Witness {
    pub(crate) matched: u64,
    pub(crate) answered: u64,
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<Vec<Value>>,
}

impl Witness {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "matched": self.matched,
            "answered": self.answered,
            "columns": self.columns,
            "rows": self.rows,
        })
    }

    pub(crate) fn from_json(value: &Value) -> Option<Self> {
        let matched = value.get("matched")?.as_u64()?;
        let answered = value.get("answered")?.as_u64()?;
        let columns = value
            .get("columns")?
            .as_array()?
            .iter()
            .map(|column| column.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        let rows = value
            .get("rows")?
            .as_array()?
            .iter()
            .map(|row| {
                row.as_array()
                    .filter(|cells| cells.len() == columns.len())
                    .cloned()
            })
            .collect::<Option<Vec<_>>>()?;
        if rows.len() > MAX_WITNESS_ROWS {
            return None;
        }

        Some(Self {
            matched,
            answered,
            columns,
            rows,
        })
    }
}

pub(super) fn kept(cell: &Value) -> Value {
    match cell {
        Value::Number(_) => Number::parse(cell).map_or_else(|| cell.clone(), Number::to_json),
        Value::String(text) => Value::String(cut(text)),
        Value::Array(_) | Value::Object(_) => Value::String(cut(&cell.to_string())),
        Value::Null | Value::Bool(_) => cell.clone(),
    }
}

fn cut(text: &str) -> String {
    text.chars().take(MAX_WITNESS_CELL_CHARS).collect()
}
