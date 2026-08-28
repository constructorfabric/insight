//! A metric's computation resolved against the measures it names, and the
//! post-aggregation transform every view projects over the folded value.

use std::collections::BTreeMap;

use crate::domain::definitions::definition::{
    Computation, MeasureDefinition, MetricDefinition, Transform,
};
use crate::domain::definitions::expr::{ScalarExpr, validate_scalar_expr};
use crate::domain::definitions::filter::FilterTree;
use crate::domain::field_catalog::model::{CatalogDataset, FieldCatalog};

use super::drilldown::{ROLE_DENOMINATOR, ROLE_NUMERATOR, ROLE_VALUE};
use super::error::CompileError;
use super::pool::{Pool, joined_entity, only_cte, scan_clause};
use super::sql::{
    CompiledMeasureQuery, EmptyFold, QueryParam, ReadScope, aggregate_expr,
    conditional_aggregate_expr, from_clause, read_predicates, render_filter,
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
    /// A composed metric folds every input in one scan, and the first input
    /// owns the grain all of them are read at.
    pub grain: &'a MeasureDefinition,
    /// Absent for a composed metric, which keeps each input's filter in its
    /// own fold.
    pub where_filter: Option<&'a FilterTree>,
    kind: FoldKind<'a>,
}

pub(super) enum FoldKind<'a> {
    Aggregate(&'a MeasureDefinition),
    Ratio {
        numerator: &'a MeasureDefinition,
        denominator: &'a MeasureDefinition,
    },
    Quantile {
        measure: &'a MeasureDefinition,
        quantile: f64,
    },
    Deviation {
        measure: &'a MeasureDefinition,
    },
    Derived {
        /// Each input under the alias the expression names it by, in the order
        /// the metric declares them.
        inputs: Vec<(&'a str, &'a MeasureDefinition)>,
        expr: Box<ScalarExpr>,
    },
}

impl<'a> Fold<'a> {
    pub fn resolve(
        metric: &'a MetricDefinition,
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
                agree_on(metric, &[numerator, denominator])?;
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
            Computation::Stddev { measure } => {
                let measure = input(measure)?;
                Ok(Self {
                    grain: measure,
                    where_filter: measure.filter.as_ref(),
                    kind: FoldKind::Deviation { measure },
                })
            }
            Computation::Derived { inputs, expr } => {
                let resolved = inputs
                    .iter()
                    .map(|(alias, key)| Ok((alias.as_str(), input(key)?)))
                    .collect::<Result<Vec<_>, CompileError>>()?;
                let measures: Vec<&MeasureDefinition> =
                    resolved.iter().map(|(_, measure)| *measure).collect();
                let Some(grain) = measures.first().copied() else {
                    return Err(CompileError::NoInputs {
                        metric: metric.key.clone(),
                    });
                };
                agree_on(metric, &measures)?;

                Ok(Self {
                    grain,
                    where_filter: None,
                    kind: FoldKind::Derived {
                        inputs: resolved,
                        expr: Box::new(validate_scalar_expr(expr).map_err(|source| {
                            CompileError::MalformedExpr {
                                metric: metric.key.clone(),
                                source,
                            }
                        })?),
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
            scan: scan_clause(from_clause(dataset), pool, &self.grain.entity, ""),
            entity: joined_entity(pool, &self.grain.entity).to_owned(),
            value,
            predicates,
            params,
        })
    }

    /// The computation this fold renders, with the measures it resolved.
    pub fn kind(&self) -> &FoldKind<'a> {
        &self.kind
    }

    /// The measures the value is computed from, each tagged with the part it
    /// plays, so a composed metric's inputs stay distinguishable. A derived
    /// input plays the part its own alias names.
    pub fn inputs(&self) -> Vec<(&'a str, &'a MeasureDefinition)> {
        match &self.kind {
            FoldKind::Aggregate(measure)
            | FoldKind::Quantile { measure, .. }
            | FoldKind::Deviation { measure } => {
                vec![(ROLE_VALUE, measure)]
            }
            FoldKind::Ratio {
                numerator,
                denominator,
            } => vec![(ROLE_NUMERATOR, numerator), (ROLE_DENOMINATOR, denominator)],
            FoldKind::Derived { inputs, .. } => inputs.clone(),
        }
    }

    /// The metric's served value as one aggregate expression over the scan.
    pub fn value_expr(
        &self,
        metric: &MetricDefinition,
        params: &mut Vec<QueryParam>,
    ) -> Result<String, CompileError> {
        let value = match &self.kind {
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
            FoldKind::Quantile { measure, quantile } => {
                // INVARIANT: a quantile of pre-folded aggregates is not that
                // quantile, so this ranks the measure's per-row values.
                let value = row_value(metric, measure)?;
                format!("quantileExact({quantile})({value})")
            }
            FoldKind::Deviation { measure } => {
                // INVARIANT: this ranks the measure's own per-row values, and
                // a spread nothing was observed for reads NULL, never zero.
                let value = row_value(metric, measure)?;
                format!("stddevSampIfOrNull({value}, {value} IS NOT NULL)")
            }
            FoldKind::Derived { inputs, expr } => {
                // INVARIANT: an input that matched no row folds to NULL and
                // arithmetic propagates it, so the value stays unknown.
                let mut folded = Vec::with_capacity(expr.references.len());
                for alias in &expr.references {
                    let (_, measure) =
                        inputs
                            .iter()
                            .find(|(name, _)| name == alias)
                            .ok_or_else(|| CompileError::UnknownDerivedInput {
                                metric: metric.key.clone(),
                                alias: alias.clone(),
                            })?;
                    folded.push(conditional_aggregate_expr(
                        measure,
                        &fold_condition(measure, params)?,
                        EmptyFold::Null,
                    )?);
                }

                expr.render(&folded)
                    .map_err(|source| CompileError::MalformedExpr {
                        metric: metric.key.clone(),
                        source,
                    })?
            }
        };

        Ok(format!("toFloat64({value})"))
    }

    /// The per-row value the named view ranks; only a distribution over
    /// per-row values has one.
    pub fn row_value_expr(
        &self,
        metric: &MetricDefinition,
        view: &'static str,
    ) -> Result<&'a str, CompileError> {
        match &self.kind {
            FoldKind::Quantile { measure, .. } | FoldKind::Deviation { measure } => {
                row_value(metric, measure)
            }
            FoldKind::Aggregate(_) | FoldKind::Ratio { .. } | FoldKind::Derived { .. } => {
                Err(CompileError::UnsupportedView {
                    metric: metric.key.clone(),
                    view,
                    reason: DISTRIBUTION_RULE,
                })
            }
        }
    }
}

/// The rule deciding which metrics have a distribution at all, stated once so
/// every view that needs one refuses in the same words.
pub(super) const DISTRIBUTION_RULE: &str = "it needs a percentile or stddev computation, the only ones taken over the measure's own per-row values";

fn row_value<'a>(
    metric: &MetricDefinition,
    measure: &'a MeasureDefinition,
) -> Result<&'a str, CompileError> {
    measure
        .value_expr
        .as_deref()
        .ok_or_else(|| CompileError::DistributionWithoutValue {
            metric: metric.key.clone(),
            measure: measure.key.clone(),
        })
}

/// The rows one input folds over, as an aggregate-function condition.
fn fold_condition(
    measure: &MeasureDefinition,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    match &measure.filter {
        Some(filter) => render_filter(measure, filter, params),
        None => Ok("1".to_owned()),
    }
}

/// INVARIANT: one scan is grained one way only, so every input of a composed
/// metric must read the same rows about the same subject at the same time.
fn agree_on(metric: &MetricDefinition, inputs: &[&MeasureDefinition]) -> Result<(), CompileError> {
    let Some((first, rest)) = inputs.split_first() else {
        return Ok(());
    };

    for other in rest {
        let disagreement = if first.dataset != other.dataset {
            Some("the dataset they read")
        } else if first.entity != other.entity {
            Some("the field they identify an entity by")
        } else if first.event_time != other.event_time {
            Some("the field they take an event time from")
        } else {
            None
        };

        if let Some(aspect) = disagreement {
            return Err(CompileError::InputsDisagree {
                metric: metric.key.clone(),
                first: first.key.clone(),
                other: other.key.clone(),
                aspect,
            });
        }
    }

    Ok(())
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
