//! The definitions and requests the compiler tests compile. Every fixture
//! reads the catalogued test datasets, so none can bind a field the catalog
//! would refuse.

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use chrono::NaiveDate;

use crate::domain::definitions::definition::{
    Aggregation, Computation, DimensionBinding, Direction, Format, MeasureDefinition,
    MetricDefinition, Transform,
};

use super::error::CompileError;
use super::metric::compile_metric_query;
use super::request::{
    BinsView, Bucket, DrilldownQuery, EntityScope, MetricQuery, QuantilesView, ResolvedPerson,
    SubjectSeriesView, ViewKind,
};
use super::sql::{CompiledMeasureQuery, QueryParam};
use super::test_catalog::catalog;

/// A count over the collapsing dataset, with one unlabelled dimension.
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

/// A second dimension whose value key is not presentable, so a label answers.
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

pub fn stddev(measure: &str) -> Computation {
    Computation::Stddev {
        measure: measure.to_owned(),
    }
}

/// A derived computation over `(alias, measure)` pairs and an expression that
/// reads them.
pub fn derived(inputs: &[(&str, &str)], expr: &str) -> Computation {
    Computation::Derived {
        inputs: inputs
            .iter()
            .map(|(alias, measure)| ((*alias).to_owned(), (*measure).to_owned()))
            .collect(),
        expr: expr.to_owned(),
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

/// A subject series over no dimensions and no cap: one series per entity.
pub fn plain_subject_series() -> ViewKind {
    ViewKind::SubjectSeries(SubjectSeriesView {
        dimensions: Vec::new(),
        group_limit: None,
    })
}

/// A bins read cutting each entity's own range into `bins`.
pub fn bins_view(bins: u32) -> ViewKind {
    ViewKind::Bins(BinsView {
        bins: NonZeroU32::new(bins).expect("a bins read cuts at least one bin"),
    })
}

/// A quantile read over the named positions.
pub fn quantiles(positions: &[f64]) -> ViewKind {
    ViewKind::Quantiles(QuantilesView {
        quantiles: positions.to_vec(),
    })
}

/// Two people, one known by two identities, so a read must fold both into one
/// row group.
pub fn people() -> EntityScope {
    EntityScope::People(vec![
        ResolvedPerson {
            person_ref: "person-1".to_owned(),
            identities: vec![
                "one@example.com".to_owned(),
                "one.alt@example.com".to_owned(),
            ],
        },
        ResolvedPerson {
            person_ref: "person-2".to_owned(),
            identities: vec!["two@example.com".to_owned()],
        },
    ])
}

/// The parameters [`people`] binds, in the order the pool writes them.
pub fn people_params() -> Vec<QueryParam> {
    vec![
        text("person-1"),
        text("one@example.com"),
        text("person-1"),
        text("one.alt@example.com"),
        text("person-2"),
        text("two@example.com"),
    ]
}

pub fn pool_head() -> [&'static str; 6] {
    [
        "WITH pool AS (",
        "    SELECT",
        "        member.1 AS person_ref,",
        "        member.2 AS identity",
        "    FROM (SELECT arrayJoin([(?, ?), (?, ?), (?, ?)]) AS member)",
        ")",
    ]
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

/// The same tenancy, scope and window [`query`] reads under, asked row by row.
pub fn drilldown_query() -> DrilldownQuery {
    DrilldownQuery {
        tenant_id: "acme-tenant".to_owned(),
        entity_scope: EntityScope::Tenant,
        from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
        to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
        dimension_filters: Vec::new(),
        display_dimensions: Vec::new(),
        page_size: 50,
        sort: None,
        cursor: None,
    }
}

pub fn text(value: &str) -> QueryParam {
    QueryParam::Text(value.to_owned())
}

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
