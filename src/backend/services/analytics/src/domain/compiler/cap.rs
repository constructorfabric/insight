//! Renders the group cap a grouped read applies: the rank each scanned row's
//! dimension values earn it, and the dimension columns the capped result
//! reports back.
//!
//! A capped read reports one row per kept group plus, optionally, one row for
//! everything else. Rank 0 is that remainder row, so the kept groups rank from
//! 1 and their dimension values are reported from the cap rather than read
//! back out of the scan.

use std::fmt::Write;

use crate::domain::definitions::definition::MeasureDefinition;

use super::dimensions::{dimension_aliases, dimension_value_expr};
use super::error::CompileError;
use super::request::{GroupLimit, RankedDimension};
use super::sql::{QueryParam, dimension_binding};

const NULL_TEXT: &str = "CAST(NULL AS Nullable(String))";

/// A cap whose every group names exactly one value per dimension the read
/// groups by, transposed into the column each dimension index reports.
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

    /// The `dim_{index}_value` / `dim_{index}_label` columns of a capped
    /// result, answered from the cap: a kept group reports the values that
    /// selected it, and the remainder row reports none.
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

    /// The predicate that drops the rows outside the kept groups, for a cap
    /// that reports no remainder row.
    pub fn remainder_predicate(&self) -> Option<&'static str> {
        if self.limit.include_remainder {
            None
        } else {
            Some("group_rank > 0")
        }
    }
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
