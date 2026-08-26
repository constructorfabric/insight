//! Turns a definition plus a query request into ClickHouse SQL.
//!
//! The compiler generates statements and nothing else: it returns the SQL text
//! and the parameters to bind against it, and never touches a connection.
//! Execution, row decoding, and view assembly are layers above.
//!
//! A measure read emits the observation row shape the metric-result builders
//! already consume — `entity_id`, `metric_date`, `value`, and, when a
//! breakdown is asked for, `dimension_value` / `dimension_label`. A metric
//! read emits the result row shape of the view it serves. Either way a view
//! assembles from either executor's rows.

#![allow(dead_code)] // tests are this module's only callers in the crate

pub mod error;
pub mod measure;
pub mod metric;
pub mod request;
mod sql;
#[cfg(test)]
mod test_catalog;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod product_tests {
    use std::collections::BTreeMap;

    use chrono::NaiveDate;

    use super::metric::compile_metric_query;
    use super::request::{Bucket, EntityScope, MetricQuery, ViewKind};
    use crate::domain::definitions::seeds::product_definitions;
    use crate::domain::field_catalog::product_catalog;

    #[test]
    fn every_shipped_metric_compiles_in_both_view_kinds() {
        let catalog = product_catalog().expect("catalog loads");
        let definitions = product_definitions().expect("definitions are valid");
        let measures: BTreeMap<_, _> = definitions
            .measures
            .iter()
            .map(|measure| (measure.key.clone(), measure.clone()))
            .collect();

        for metric in &definitions.metrics {
            for view in [ViewKind::Period, ViewKind::Timeseries] {
                let query = MetricQuery {
                    tenant_id: "tenant".to_owned(),
                    entity_scope: EntityScope::Tenant,
                    from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                    to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
                    bucket: Bucket::Week,
                    dimension_filters: Vec::new(),
                    view,
                    row_limit: 10_000,
                };
                let compiled = compile_metric_query(catalog, metric, &measures, &query);
                assert!(
                    compiled.is_ok(),
                    "metric `{}` must compile for {view:?}: {}",
                    metric.key,
                    compiled.err().map(|e| e.to_string()).unwrap_or_default()
                );
            }
        }
    }
}
