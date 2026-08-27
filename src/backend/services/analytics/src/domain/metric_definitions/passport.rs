//! Metric passports: a generated, human-readable derivation record for every
//! builtin metric — source, formula, and notes — rendered from the same
//! `registry.yaml` the reconciler seeds from. The rendered document is
//! committed as `passports.md` and pinned by [`tests::passports_md_is_in_sync`],
//! so a metric whose source, formula, or notes change without regenerating the
//! passport fails the build. Regenerate with `analytics passports > …/passports.md`.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::domain::metric_definitions::builtin::{
    MetricSeed, SeedComputation, builtin_metrics, builtin_sources,
};
use crate::domain::metric_definitions::definition::{MetricInputRole, ValueTransform};

const HEADER: &str = "\
# Metric passports

Generated from `registry.yaml` by `analytics passports`. Do not edit by hand —
regenerate and commit. A drift test (`metric_definitions::passport`) fails when
this file and the registry disagree.
";

/// Render the passport document from the builtin registry. Deterministic:
/// metrics appear in registry order, one section each.
pub fn render_passports() -> String {
    let source_refs: HashMap<&str, &str> = builtin_sources()
        .iter()
        .map(|source| {
            (
                source.source.key.as_str(),
                source.source.source_ref.as_str(),
            )
        })
        .collect();

    let mut out = String::from(HEADER);

    for metric in builtin_metrics() {
        let source_ref = source_refs
            .get(metric.source_key.as_str())
            .copied()
            .unwrap_or("?");
        let reads = metric
            .inputs
            .iter()
            .map(|input| input.measure_key.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let notes = metric
            .explanation
            .as_deref()
            .or(metric.description.as_deref())
            .unwrap_or("—");

        // Infallible: writing to a String never errors.
        let _ = write!(
            out,
            "\n## {key} — {label}\n\n\
             - Source: {source_key} ({source_ref})\n\
             - Reads: {reads}\n\
             - Formula: {formula}\n\
             - Shape: {shape}\n\
             - Notes: {notes}\n",
            key = metric.metric_key,
            label = metric.label,
            source_key = metric.source_key,
            formula = formula(metric),
            shape = shape(metric),
        );
    }

    out
}

fn formula(metric: &MetricSeed) -> String {
    let measure = |role: MetricInputRole| -> &str {
        metric
            .inputs
            .iter()
            .find(|input| input.input_role == role)
            .map_or("?", |input| input.measure_key.as_str())
    };

    let base = match metric.computation {
        SeedComputation::Sum => format!("sum({})", measure(MetricInputRole::Value)),
        SeedComputation::Median => format!("median({})", measure(MetricInputRole::Value)),
        SeedComputation::Percentile { q } => format!(
            "p{}({})",
            fmt_num(q * 100.0),
            measure(MetricInputRole::Value)
        ),
        SeedComputation::Stddev => format!("stddev({})", measure(MetricInputRole::Value)),
        SeedComputation::DistinctCount => {
            format!("distinct_count({})", measure(MetricInputRole::Value))
        }
        SeedComputation::Ratio {
            scale,
            denominator_aggregation,
        } => {
            let denominator = match denominator_aggregation {
                crate::domain::metric_definitions::definition::RatioDenominatorAggregation::Sum => {
                    measure(MetricInputRole::Denominator).to_owned()
                }
                crate::domain::metric_definitions::definition::RatioDenominatorAggregation::DistinctCount => {
                    format!(
                        "distinct_count({})",
                        measure(MetricInputRole::Denominator)
                    )
                }
            };
            let ratio = format!("{} / {}", measure(MetricInputRole::Numerator), denominator);
            if (scale - 1.0).abs() < f64::EPSILON {
                ratio
            } else {
                format!("{} * ({ratio})", fmt_num(scale))
            }
        }
    };

    match &metric.transform {
        Some(transform) if !transform.is_identity() => {
            format!("{base} -> {}", transform_expr(transform))
        }
        _ => base,
    }
}

fn transform_expr(transform: &ValueTransform) -> String {
    let mut expr = String::from("x");
    if let Some(multiplier) = transform.multiplier {
        expr = format!("{}*x", fmt_num(multiplier));
    }
    if let Some(offset) = transform.offset {
        expr = format!("{expr} + {}", fmt_num(offset));
    }

    let clamp = match (transform.clamp_min, transform.clamp_max) {
        (Some(lo), Some(hi)) => format!(", clamped to [{}, {}]", fmt_num(lo), fmt_num(hi)),
        (Some(lo), None) => format!(", clamped to >= {}", fmt_num(lo)),
        (None, Some(hi)) => format!(", clamped to <= {}", fmt_num(hi)),
        (None, None) => String::new(),
    };

    format!("{expr}{clamp}")
}

fn shape(metric: &MetricSeed) -> String {
    let mut shape = format!("{}, {}", metric.format.as_db(), metric.direction.as_db());
    if let Some(unit) = &metric.unit {
        let _ = write!(shape, ", unit {unit}");
    }
    shape
}

/// Format an `f64` registry constant without a spurious `.0` (100.0 -> "100",
/// 1.5 -> "1.5"), so the passport reads naturally and stays stable.
fn fmt_num(value: f64) -> String {
    format!("{value}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed passport document, embedded so the drift test compares
    /// against the exact bytes on disk.
    const PASSPORTS_MD: &str = include_str!("passports.md");

    #[test]
    fn passports_md_is_in_sync() {
        assert_eq!(
            render_passports(),
            PASSPORTS_MD,
            "passports.md is stale — regenerate: \
             (cd src/backend && cargo run -p analytics -- passports) \
             > src/backend/services/analytics/src/domain/metric_definitions/passports.md"
        );
    }

    #[test]
    fn every_builtin_metric_has_a_section() {
        let rendered = render_passports();
        for metric in builtin_metrics() {
            assert!(
                rendered.contains(&format!("\n## {} — ", metric.metric_key)),
                "no passport section for {}",
                metric.metric_key
            );
        }
    }

    #[test]
    fn ratio_scale_of_one_is_elided() {
        // A ratio with scale 1.0 renders as a bare division; a scaled ratio
        // carries its multiplier.
        let rendered = render_passports();
        assert!(rendered.contains("Formula: commit_count / distinct_count(commit_day)"));
        assert!(rendered.contains("Formula: 100 * (accepted_edit_actions / tool_use_offered)"));
    }

    #[test]
    fn transform_is_rendered_after_the_base_formula() {
        let rendered = render_passports();
        assert!(
            rendered.contains(
                "Formula: estimation_error_pct / estimation_samples \
                 -> -1*x + 100, clamped to [0, 100]"
            ),
            "estimation-accuracy passport should show the affine+clamp transform"
        );
    }
}
