//! A metric's computation resolved against the measures it names, and the
//! post-aggregation transform every view projects over the folded value.

use std::collections::BTreeMap;

use crate::domain::definitions::definition::{
    Computation, MeasureDefinition, MetricDefinition, Transform,
};
use crate::domain::definitions::filter::FilterTree;
use crate::domain::field_catalog::model::{CatalogDataset, FieldCatalog};

use super::drilldown::{ROLE_DENOMINATOR, ROLE_NUMERATOR, ROLE_VALUE};
use super::error::CompileError;
use super::pool::{Pool, joined_entity, only_cte, scan_clause};
use super::sql::{
    CompiledMeasureQuery, EmptyFold, QueryParam, ReadScope, aggregate_expr,
    conditional_aggregate_expr, read_predicates, render_filter,
};

/// Everything a read renders itself from over one scan, with the parameters
/// all of it binds, in binding order.
pub(super) struct ScopedRead {
    /// Empty unless the read's entities come from a pool it must declare first.
    pub head: String,
    /// The relation to scan, already carrying the pool join when there is one.
    pub scan: String,
    /// The column the read keys its rows by.
    pub entity: String,
    pub value: String,
    pub predicates: Vec<String>,
    pub params: Vec<QueryParam>,
}

pub(super) struct Fold<'a> {
    /// A ratio folds two measures in one scan, and the numerator owns the
    /// grain both halves are read at.
    pub grain: &'a MeasureDefinition,
    /// Absent for a ratio, which keeps both measures' filters in its folds.
    pub where_filter: Option<&'a FilterTree>,
    kind: FoldKind<'a>,
}

enum FoldKind<'a> {
    Aggregate(&'a MeasureDefinition),
    Ratio {
        numerator: &'a MeasureDefinition,
        denominator: &'a MeasureDefinition,
    },
    Quantile {
        measure: &'a MeasureDefinition,
        quantile: f64,
    },
}

impl<'a> Fold<'a> {
    pub fn resolve(
        metric: &MetricDefinition,
        measures: &'a BTreeMap<String, MeasureDefinition>,
    ) -> Result<Self, CompileError> {
        let input = |key: &str| {
            measures
                .get(key)
                .ok_or_else(|| CompileError::MeasureNotFound {
                    metric: metric.key.clone(),
                    measure: key.to_owned(),
                })
        };

        match &metric.computation {
            Computation::Direct { measure } => {
                let measure = input(measure)?;
                Ok(Self {
                    grain: measure,
                    where_filter: measure.filter.as_ref(),
                    kind: FoldKind::Aggregate(measure),
                })
            }
            Computation::Ratio {
                numerator,
                denominator,
            } => {
                let numerator = input(numerator)?;
                let denominator = input(denominator)?;
                agree_on(metric, numerator, denominator)?;
                Ok(Self {
                    grain: numerator,
                    where_filter: None,
                    kind: FoldKind::Ratio {
                        numerator,
                        denominator,
                    },
                })
            }
            Computation::Percentile { measure, quantile } => {
                let measure = input(measure)?;
                Ok(Self {
                    grain: measure,
                    where_filter: measure.filter.as_ref(),
                    kind: FoldKind::Quantile {
                        measure,
                        quantile: *quantile,
                    },
                })
            }
        }
    }

    pub fn dataset<'c>(
        &self,
        catalog: &'c FieldCatalog,
    ) -> Result<&'c CatalogDataset, CompileError> {
        catalog
            .dataset(&self.grain.dataset)
            .ok_or_else(|| CompileError::UnknownDataset {
                measure: self.grain.key.clone(),
                dataset: self.grain.dataset.clone(),
            })
    }

    // INVARIANT: placeholders bind by position, so parameters are pushed in the
    // order the statement writes them: pool head, fold values, scope predicates.
    pub fn scoped_read(
        &self,
        dataset: &CatalogDataset,
        metric: &MetricDefinition,
        scope: &ReadScope<'_>,
        pool: Option<&Pool<'_>>,
    ) -> Result<ScopedRead, CompileError> {
        let mut params = Vec::new();
        let head = only_cte(pool, &mut params)?;
        let value = self.value_expr(metric, &mut params)?;
        let predicates =
            read_predicates(dataset, self.grain, self.where_filter, scope, &mut params)?;

        Ok(ScopedRead {
            head,
            scan: scan_clause(dataset, pool, self.grain, ""),
            entity: joined_entity(pool, self.grain).to_owned(),
            value,
            predicates,
            params,
        })
    }

    /// The measures the value is computed from, each tagged with the part it
    /// plays, so a ratio's two halves stay distinguishable.
    pub fn inputs(&self) -> Vec<(&'static str, &'a MeasureDefinition)> {
        match self.kind {
            FoldKind::Aggregate(measure) | FoldKind::Quantile { measure, .. } => {
                vec![(ROLE_VALUE, measure)]
            }
            FoldKind::Ratio {
                numerator,
                denominator,
            } => vec![(ROLE_NUMERATOR, numerator), (ROLE_DENOMINATOR, denominator)],
        }
    }

    /// The metric's served value as one aggregate expression over the scan.
    pub fn value_expr(
        &self,
        metric: &MetricDefinition,
        params: &mut Vec<QueryParam>,
    ) -> Result<String, CompileError> {
        let value = match self.kind {
            FoldKind::Aggregate(measure) => aggregate_expr(measure)?,
            FoldKind::Ratio {
                numerator,
                denominator,
            } => {
                // INVARIANT: an empty numerator is an unknown split and a zero
                // denominator an undefined ratio; both read NULL, never zero.
                let numerator = conditional_aggregate_expr(
                    numerator,
                    &fold_condition(numerator, params)?,
                    EmptyFold::Null,
                )?;
                let denominator = conditional_aggregate_expr(
                    denominator,
                    &fold_condition(denominator, params)?,
                    EmptyFold::Zero,
                )?;
                format!("{numerator} / nullIf({denominator}, 0)")
            }
            FoldKind::Quantile { quantile, .. } => {
                // INVARIANT: a quantile of pre-folded aggregates is not that
                // quantile, so this ranks the measure's per-row values.
                let value = self.row_value_expr(metric)?;
                format!("quantileExact({quantile})({value})")
            }
        };

        Ok(format!("toFloat64({value})"))
    }

    /// The per-row value the fold ranks; only a distribution over per-row
    /// values has one.
    pub fn row_value_expr(&self, metric: &MetricDefinition) -> Result<&'a str, CompileError> {
        match self.kind {
            FoldKind::Quantile { measure, .. } => {
                measure
                    .value_expr
                    .as_deref()
                    .ok_or_else(|| CompileError::PercentileWithoutValue {
                        metric: metric.key.clone(),
                        measure: measure.key.clone(),
                    })
            }
            FoldKind::Aggregate(_) | FoldKind::Ratio { .. } => Err(CompileError::UnsupportedView {
                metric: metric.key.clone(),
                view: "bins",
                reason: "it needs a percentile computation, which is the only one taken over the measure's own per-row values",
            }),
        }
    }
}

/// The rows one half of a ratio folds over, as an aggregate-function condition.
fn fold_condition(
    measure: &MeasureDefinition,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    match &measure.filter {
        Some(filter) => render_filter(measure, filter, params),
        None => Ok("1".to_owned()),
    }
}

/// INVARIANT: one scan is grained one way only, so a ratio's two halves must
/// read the same rows about the same subject at the same time.
fn agree_on(
    metric: &MetricDefinition,
    numerator: &MeasureDefinition,
    denominator: &MeasureDefinition,
) -> Result<(), CompileError> {
    let disagreement = if numerator.dataset != denominator.dataset {
        Some("the dataset they read")
    } else if numerator.entity != denominator.entity {
        Some("the field they identify an entity by")
    } else if numerator.event_time != denominator.event_time {
        Some("the field they take an event time from")
    } else {
        None
    };

    match disagreement {
        None => Ok(()),
        Some(aspect) => Err(CompileError::RatioInputsDisagree {
            metric: metric.key.clone(),
            numerator: numerator.key.clone(),
            denominator: denominator.key.clone(),
            aspect,
        }),
    }
}

/// INVARIANT: the row ceiling binds last, after everything the read wrote.
pub(super) fn bounded_query(
    transform: Option<&Transform>,
    mut params: Vec<QueryParam>,
    row_limit: u64,
    inner: String,
) -> CompiledMeasureQuery {
    params.push(QueryParam::UInt(row_limit));

    CompiledMeasureQuery {
        sql: transformed(transform, inner),
        params,
    }
}

/// INVARIANT: the fold stays in the inner statement so its placeholders bind
/// once; the projection reads only the `value` column the clamp guard repeats.
pub(super) fn transformed(transform: Option<&Transform>, inner: String) -> String {
    match transform {
        Some(transform) if !is_identity(transform) => {
            let value = transform_expr(transform, "value");
            format!("SELECT\n    * EXCEPT (value),\n    {value} AS value\nFROM (\n{inner}\n)")
        }
        Some(_) | None => inner,
    }
}

pub(super) fn is_identity(transform: &Transform) -> bool {
    transform.multiplier.is_none()
        && transform.offset.is_none()
        && transform.clamp_min.is_none()
        && transform.clamp_max.is_none()
}

/// INVARIANT: callers pass a column or alias reference, never an expression
/// carrying a placeholder — the clamp guard reads it twice.
pub(super) fn transform_in_place(transform: Option<&Transform>, expr: &str) -> String {
    match transform {
        Some(transform) if !is_identity(transform) => transform_expr(transform, expr),
        Some(_) | None => expr.to_owned(),
    }
}

// SAFETY: ClickHouse `least`/`greatest` ignore NULL arguments (24.12+), so the
// explicit guard keeps an unknown value unknown instead of a clamp bound.
fn transform_expr(transform: &Transform, expr: &str) -> String {
    let mut out = expr.to_owned();
    if let Some(multiplier) = transform.multiplier {
        out = format!("{multiplier:?} * ({out})");
    }
    if let Some(offset) = transform.offset {
        out = format!("({offset:?} + {out})");
    }
    if transform.clamp_min.is_none() && transform.clamp_max.is_none() {
        return out;
    }

    let mut clamped = out.clone();
    if let Some(clamp_min) = transform.clamp_min {
        clamped = format!("greatest({clamp_min:?}, {clamped})");
    }
    if let Some(clamp_max) = transform.clamp_max {
        clamped = format!("least({clamp_max:?}, {clamped})");
    }

    format!("if(({out}) IS NULL, NULL, {clamped})")
}
