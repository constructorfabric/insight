//! The definitions and requests the compiler tests compile, and the two
//! spellings the golden assertions read a statement in.
//!
//! Every fixture reads the catalogued test datasets, so a fixture cannot bind
//! a field the catalog would refuse.

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::domain::definitions::definition::{
    Aggregation, Computation, DimensionBinding, Direction, Format, MeasureDefinition,
    MetricDefinition, Transform,
};

use super::error::CompileError;
use super::metric::compile_metric_query;
use super::request::{Bucket, EntityScope, MetricQuery, ViewKind};
use super::sql::{CompiledMeasureQuery, QueryParam};
use super::test_catalog::catalog;

/// A count over the collapsing dataset, broken down by one unlabelled
/// dimension.
pub fn measure(key: &str, filter: Option<&str>) -> MeasureDefinition {
    MeasureDefinition {
        key: key.to_owned(),
        dataset: "git_pull_requests".to_owned(),
        description: None,
        filter: filter.map(|filter| serde_yaml::from_str(filter).expect("filter parses")),
        aggregation: Aggregation::Count,
        value_expr: None,
        subject_expr: None,
        event_time: "closed_on".to_owned(),
        entity: "author_email".to_owned(),
        dimensions: vec![DimensionBinding {
            key: "repository".to_owned(),
            value_field: "repo_slug".to_owned(),
            label_field: None,
        }],
    }
}

/// The same measure with a second dimension whose value key is not itself
/// presentable, so a label column answers for it.
pub fn labelled_measure(key: &str) -> MeasureDefinition {
    let mut measure = measure(key, None);
    measure.dimensions.push(DimensionBinding {
        key: "source".to_owned(),
        value_field: "data_source".to_owned(),
        label_field: Some("data_source_label".to_owned()),
    });
    measure
}

/// A measure folding a per-row value, which a percentile can rank.
pub fn sized_measure(key: &str) -> MeasureDefinition {
    MeasureDefinition {
        aggregation: Aggregation::Sum,
        value_expr: Some("lines_added".to_owned()),
        ..measure(key, None)
    }
}

pub fn measures(defined: &[MeasureDefinition]) -> BTreeMap<String, MeasureDefinition> {
    defined
        .iter()
        .map(|measure| (measure.key.clone(), measure.clone()))
        .collect()
}

pub fn metric(computation: Computation) -> MetricDefinition {
    MetricDefinition {
        key: "git.merge_rate".to_owned(),
        computation,
        transform: None,
        format: Format::Percent,
        direction: Direction::HigherIsBetter,
        entity_type: "person".to_owned(),
        cohort_key: None,
        label: None,
        description: None,
    }
}

pub fn direct(measure: &str) -> Computation {
    Computation::Direct {
        measure: measure.to_owned(),
    }
}

pub fn ratio(numerator: &str, denominator: &str) -> Computation {
    Computation::Ratio {
        numerator: numerator.to_owned(),
        denominator: denominator.to_owned(),
    }
}

pub fn percentile(measure: &str, quantile: f64) -> Computation {
    Computation::Percentile {
        measure: measure.to_owned(),
        quantile,
    }
}

pub fn percent_of_total() -> Transform {
    Transform {
        multiplier: Some(100.0),
        offset: None,
        clamp_min: Some(0.0),
        clamp_max: Some(100.0),
    }
}

pub fn query(view: ViewKind) -> MetricQuery {
    MetricQuery {
        tenant_id: "acme-tenant".to_owned(),
        entity_scope: EntityScope::Tenant,
        from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
        to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
        bucket: Bucket::Day,
        dimension_filters: Vec::new(),
        view,
        row_limit: 10_001,
    }
}

pub fn text(value: &str) -> QueryParam {
    QueryParam::Text(value.to_owned())
}

/// The statement a golden assertion expects, written one line per line.
pub fn lines(expected: &[&str]) -> String {
    expected.join("\n")
}

pub fn compile(
    metric: &MetricDefinition,
    defined: &[MeasureDefinition],
    query: &MetricQuery,
) -> CompiledMeasureQuery {
    compile_metric_query(&catalog(), metric, &measures(defined), query).expect("compiles")
}

pub fn compile_err(
    metric: &MetricDefinition,
    defined: &[MeasureDefinition],
    query: &MetricQuery,
) -> CompileError {
    compile_metric_query(&catalog(), metric, &measures(defined), query)
        .expect_err("expected a compile error")
}
