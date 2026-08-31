//! Renders the group cap a grouped read applies: the rank each scanned row's
//! dimension values earn it, and the dimension columns the capped result
//! reports back.
//!
//! INVARIANT: rank 0 is the remainder row's, so a kept group ranks from 1.

use std::fmt::Write;

use crate::domain::definitions::definition::MeasureDefinition;

use super::dimensions::{dimension_aliases, dimension_value_expr};
use super::error::CompileError;
use super::request::{GroupLimit, RankedDimension};
use super::sql::{QueryParam, dimension_binding};

const NULL_TEXT: &str = "CAST(NULL AS Nullable(String))";

/// The `rank` / `remainder` / `group_label` columns a capped read closes its
/// projection with, read off the rank the cap gave the group.
pub(super) const CAPPED_RANK_COLUMNS: &str = concat!(
    "    if(group_rank = 0, CAST(NULL AS Nullable(UInt32)), toNullable(group_rank)) AS rank,\n",
    "    toUInt8(group_rank = 0) AS remainder,\n",
    "    if(group_rank = 0, toNullable('Other'), CAST(NULL AS Nullable(String))) AS group_label\n",
);

/// The same three columns of an uncapped read, which keeps every group and so
/// ranks none. The row decoder expects them either way.
pub(super) const UNCAPPED_RANK_COLUMNS: &str = concat!(
    "    CAST(NULL AS Nullable(UInt32)) AS rank,\n",
    "    toUInt8(0) AS remainder,\n",
    "    CAST(NULL AS Nullable(String)) AS group_label\n",
);

/// A cap transposed from groups into the column each dimension index reports.
pub(super) struct GroupCap<'a> {
    limit: &'a GroupLimit,
    dimension_count: usize,
    /// Per dimension index, what each kept group contributes to that column.
    columns: Vec<Vec<(u32, &'a RankedDimension)>>,
}

impl<'a> GroupCap<'a> {
    pub fn resolve(limit: &'a GroupLimit, dimension_count: usize) -> Result<Self, CompileError> {
        let mut columns = vec![Vec::with_capacity(limit.groups.len()); dimension_count];
        for group in &limit.groups {
            if group.dimensions.len() != dimension_count {
                return Err(CompileError::GroupCapArity {
                    rank: group.rank,
                    named: group.dimensions.len(),
                    requested: dimension_count,
                });
            }
            for (column, dimension) in columns.iter_mut().zip(&group.dimensions) {
                column.push((group.rank, dimension));
            }
        }

        Ok(Self {
            limit,
            dimension_count,
            columns,
        })
    }

    /// Every scanned row's rank: the kept group whose values it matches, or 0.
    pub fn rank_expr(&self, params: &mut Vec<QueryParam>) -> String {
        if self.limit.groups.is_empty() {
            return "toUInt32(0)".to_owned();
        }

        let comparisons = (0..self.dimension_count)
            .map(|index| format!("raw_dim_{index} = ?"))
            .collect::<Vec<_>>()
            .join(" AND ");

        let mut branches = Vec::with_capacity(self.limit.groups.len() * 2 + 1);
        for group in &self.limit.groups {
            params.extend(
                group
                    .dimensions
                    .iter()
                    .map(|dimension| QueryParam::Text(dimension.value.clone())),
            );
            branches.push(format!("({comparisons})"));
            branches.push(format!("toUInt32({})", group.rank));
        }
        branches.push("toUInt32(0)".to_owned());

        format!("multiIf({})", branches.join(", "))
    }

    /// A kept group reports the values that selected it; the remainder row
    /// reports none.
    pub fn dimension_select(&self, params: &mut Vec<QueryParam>) -> String {
        let mut select = String::new();

        for (index, column) in self.columns.iter().enumerate() {
            let (value_alias, label_alias) = dimension_aliases(index);
            if column.is_empty() {
                let _ = writeln!(select, "    {NULL_TEXT} AS {value_alias},");
                let _ = writeln!(select, "    {NULL_TEXT} AS {label_alias},");
                continue;
            }

            let mut value_branches = Vec::with_capacity(column.len() * 2 + 1);
            let mut label_branches = Vec::with_capacity(column.len() * 2 + 1);
            let mut values = Vec::with_capacity(column.len());
            let mut labels = Vec::with_capacity(column.len());

            for (rank, dimension) in column {
                value_branches.push(format!("group_rank = {rank}"));
                value_branches.push("toNullable(?)".to_owned());
                values.push(QueryParam::Text(dimension.value.clone()));
                label_branches.push(format!("group_rank = {rank}"));
                match &dimension.label {
                    Some(label) => {
                        label_branches.push("toNullable(?)".to_owned());
                        labels.push(QueryParam::Text(label.clone()));
                    }
                    None => label_branches.push(NULL_TEXT.to_owned()),
                }
            }

            params.extend(values);
            params.extend(labels);
            value_branches.push(NULL_TEXT.to_owned());
            label_branches.push(NULL_TEXT.to_owned());

            let _ = writeln!(
                select,
                "    multiIf({}) AS {value_alias},",
                value_branches.join(", ")
            );
            let _ = writeln!(
                select,
                "    multiIf({}) AS {label_alias},",
                label_branches.join(", ")
            );
        }

        select
    }

    /// The predicate a cap with no remainder row drops its other rows by.
    pub fn remainder_predicate(&self) -> Option<&'static str> {
        if self.limit.include_remainder {
            None
        } else {
            Some("group_rank > 0")
        }
    }
}

/// The stages every capped read opens with: the scan carrying the columns the
/// cap ranks by, the rank each scanned row earns, and the rows the cap keeps.
pub(super) fn ranked_scan_ctes(
    head: String,
    scan: &str,
    projections: &str,
    predicates: &[String],
    rank: &str,
    remainder_predicate: Option<&'static str>,
) -> String {
    let mut sql = head;
    sql.push_str("scoped AS (\n    SELECT\n        *,\n");
    let _ = writeln!(sql, "{projections}");
    let _ = writeln!(sql, "    FROM {scan}");
    let _ = writeln!(sql, "    WHERE {}", predicates.join("\n      AND "));
    let _ = writeln!(sql, "),");
    let _ = writeln!(sql, "ranked AS (");
    let _ = writeln!(sql, "    SELECT");
    let _ = writeln!(sql, "        *,");
    let _ = writeln!(sql, "        {rank} AS group_rank");
    let _ = writeln!(sql, "    FROM scoped");
    let _ = writeln!(sql, "),");
    let _ = writeln!(sql, "filtered AS (");
    let _ = writeln!(sql, "    SELECT *");
    let _ = writeln!(sql, "    FROM ranked");
    if let Some(predicate) = remainder_predicate {
        let _ = writeln!(sql, "    WHERE {predicate}");
    }
    let _ = writeln!(sql, "),");
    sql
}

/// The dimension values a scanned row is ranked by, projected beside it.
pub(super) fn raw_dimension_select(
    measure: &MeasureDefinition,
    keys: &[String],
) -> Result<String, CompileError> {
    let mut projections = Vec::with_capacity(keys.len());
    for (index, key) in keys.iter().enumerate() {
        let binding = dimension_binding(measure, key)?;
        projections.push(format!(
            "        {} AS raw_dim_{index}",
            dimension_value_expr(binding)
        ));
    }
    Ok(projections.join(",\n"))
}
