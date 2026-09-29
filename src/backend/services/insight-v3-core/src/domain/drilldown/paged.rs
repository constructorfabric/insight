//! The metric's query, wrapped: read as a subquery, ordered by the reader's
//! key, resumed past the cursor and cut one row past the page.

use std::fmt::Write as _;

use super::Column;
use super::cursor::CursorKey;
use super::order::{INNER, OrderKey};
use crate::domain::query::metric_query::{CompiledQuery, FilterBind, TWIN_COLUMN};

/// The alias the metric's own query is read under when its twins have to be
/// numbered first, which takes a layer of its own.
const INNERMOST: &str = "__i";

/// INVARIANT: one row more than the page holds. Whether there is a next page
/// is answered by that row's existence, and never by a count.
pub(super) fn wrap(
    inner: &CompiledQuery,
    order: &OrderKey,
    resume: Option<&CursorKey>,
    limit: usize,
    columns: &[Column],
) -> (String, Vec<FilterBind>) {
    let mut binds = inner.binds().to_vec();
    let mut sql = format!(
        "SELECT {INNER}.*{} FROM ({}) AS {INNER}",
        order.projection(),
        source(inner, columns)
    );

    if let Some(key) = resume {
        let predicate = order.cursor_predicate(key, &mut binds);
        let _ = write!(sql, " WHERE {predicate}");
    }

    let _ = write!(
        sql,
        " ORDER BY {} LIMIT {}",
        order.order_by(),
        limit.saturating_add(1)
    );

    (sql, binds)
}

/// The metric's query as the wrapper reads it. Where twins have to be
/// numbered, they are counted off here: identical rows are interchangeable,
/// so which of them is the first does not matter, only that each page cut
/// between two of them keeps the rest.
///
/// INVARIANT: numbered before the cursor is compared, in a layer of their
/// own. A window function is computed after `WHERE`, so a comparison in the
/// same select could not see the number.
fn source(inner: &CompiledQuery, columns: &[Column]) -> String {
    if !inner.numbered() {
        return inner.sql().to_owned();
    }

    let partition: Vec<String> = columns
        .iter()
        .map(|column| format!("{INNERMOST}.`{}`", column.key))
        .collect();

    format!(
        "SELECT {INNERMOST}.*, row_number() OVER (PARTITION BY {}) AS `{TWIN_COLUMN}` FROM ({}) AS {INNERMOST}",
        partition.join(", "),
        inner.sql()
    )
}
