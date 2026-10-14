//! The rows a check shows for what it found: how many passed, of how many,
//! and up to a few of them, cut to the columns a reader needs.

use serde_json::{Value, json};

use crate::domain::alerts::scalar::Number;

// INVARIANT: these bounds only ever widen; a stored witness is read back
// against them.

/// The most rows a witness keeps.
pub(crate) const MAX_WITNESS_ROWS: usize = 5;
/// The longest text cell a witness keeps.
pub(crate) const MAX_WITNESS_CELL_CHARS: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Witness {
    /// Rows the test passed on.
    pub(crate) matched: u64,
    /// Rows the metric answered.
    pub(crate) answered: u64,
    /// Group-by columns, then condition columns, each once.
    pub(crate) columns: Vec<String>,
    /// INVARIANT: every row holds one cell per column, in column order, and
    /// there are at most [`MAX_WITNESS_ROWS`].
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

    /// A witness as [`Witness::to_json`] wrote it, or nothing for anything
    /// else.
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

/// A cell as a witness keeps it: an integer past what JavaScript holds
/// exactly as its digits, text cut to [`MAX_WITNESS_CELL_CHARS`], and a list
/// or an object as its JSON text, cut the same way.
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
