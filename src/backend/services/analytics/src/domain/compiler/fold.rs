//! A metric's computation resolved against the measures it names, and the
//! post-aggregation transform every view projects over the folded value.
//!
//! Every view kind reads the same fold over the same dataset; they differ only
//! in what they group by and what columns they report, so resolving the fold
//! is done once here and each view renders it.

use std::collections::BTreeMap;

use crate::domain::definitions::definition::{
    Computation, MeasureDefinition, MetricDefinition, Transform,
};
use crate::domain::definitions::filter::FilterTree;
use crate::domain::field_catalog::model::{CatalogDataset, FieldCatalog};

use super::error::CompileError;
use super::sql::{
    CompiledMeasureQuery, EmptyFold, QueryParam, ReadScope, aggregate_expr,
    conditional_aggregate_expr, read_predicates, render_filter,
};

/// The value column and scope predicates of a read that writes its fold before
/// its `WHERE`, with the parameters both bind, in binding order.
pub(super) struct ScopedRead {
    pub value: String,
    pub predicates: Vec<String>,
    pub params: Vec<QueryParam>,
}

pub(super) struct Fold<'a> {
    /// The measure whose entity, event time, and dimensions the read is
    /// grained by. A ratio folds two measures in one scan, and the numerator
    /// owns the grain both halves are read at.
    pub grain: &'a MeasureDefinition,
    /// The stored filter to scope the scan by, when one measure owns the whole
    /// read. A ratio keeps both filters in its folds instead.
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

    // INVARIANT: a placeholder binds by position, and the value column is
    // written before the predicates are, so the fold's values bind first.
    pub fn scoped_read(
        &self,
        dataset: &CatalogDataset,
        metric: &MetricDefinition,
        scope: &ReadScope<'_>,
    ) -> Result<ScopedRead, CompileError> {
        let mut params = Vec::new();
        let value = self.value_expr(metric, &mut params)?;
        let predicates =
            read_predicates(dataset, self.grain, self.where_filter, scope, &mut params)?;

        Ok(ScopedRead {
            value,
            predicates,
            params,
        })
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
                // A numerator that matches no row is an unknown split, not a
                // zero; a zero denominator is an undefined ratio. Both read
                // NULL, and the builders never fill a NULL back in.
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
                // The quantile is taken over the measure's per-row values: a
                // quantile of pre-folded aggregates is not that quantile.
                let value = self.row_value_expr(metric)?;
                format!("quantileExact({quantile})({value})")
            }
        };

        Ok(format!("toFloat64({value})"))
    }

    /// The per-row value the fold ranks, for a view that reads the observations
    /// themselves rather than their fold. Only a distribution over per-row
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

/// The rows one half of a ratio folds over, as an aggregate-function
/// condition. A measure with no stored filter folds over every scanned row.
fn fold_condition(
    measure: &MeasureDefinition,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    match &measure.filter {
        Some(filter) => render_filter(measure, filter, params),
        None => Ok("1".to_owned()),
    }
}

/// One scan can be grained one way only, so the two halves of a ratio must
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

/// Binds the row ceiling last and projects the transform over a rendered read
/// — the closing two steps of every read that folds one value column.
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

/// Projects the transform over the aggregated value. The fold stays in the
/// inner statement so its placeholders bind once; the projection reads only
/// the `value` column, which the clamp guard references more than once.
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

/// The transform applied in place to an expression a statement may repeat.
/// Callers pass a column or alias reference, never an expression carrying a
/// placeholder: the clamp guard reads it twice.
pub(super) fn transform_in_place(transform: Option<&Transform>, expr: &str) -> String {
    match transform {
        Some(transform) if !is_identity(transform) => transform_expr(transform, expr),
        Some(_) | None => expr.to_owned(),
    }
}

// SAFETY: ClickHouse `least`/`greatest` ignore NULL arguments (24.12+), so an
// unguarded clamp would resurrect an honest NULL as the clamp bound. The
// explicit guard keeps an unknown value unknown.
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
