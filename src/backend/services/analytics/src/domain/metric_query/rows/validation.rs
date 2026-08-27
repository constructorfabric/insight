//! The boundary between what a caller wrote and what a page reasons about: one
//! metric, the people it is about, and exactly one input of its computation.
//!
//! INVARIANT: which input a page reads is decided here, so a request reaching
//! the compiler always names one the metric composes.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use uuid::Uuid;

use crate::domain::compiler::request::DimensionFilter;

use super::super::catalog::MetricCatalog;
use super::super::dto::Subjects;
use super::super::error::QueryError;
use super::super::question::{defined_metric, filters, person_ids, window};
use super::dto::RowsRequest;

/// The field a page names its people in.
const SUBJECTS_FIELD: &str = "subjects.ids";

const DEFAULT_PAGE_SIZE: u32 = 100;
const MAX_PAGE_SIZE: u32 = 250;
const MAX_DISPLAY_DIMENSIONS: usize = 10;

#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedRows {
    pub metric_key: String,
    pub subjects: Vec<Uuid>,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub filters: Vec<DimensionFilter>,
    /// The part of the metric's computation this page reads.
    pub input_role: String,
    /// Dimension keys to report beyond the measure's own, asked for once each.
    pub display_dimensions: Vec<String>,
    pub page_size: u32,
    /// Still encoded: reading a position needs the fingerprint of the question
    /// it was issued for, which only the whole validated request decides.
    pub cursor: Option<String>,
}

pub fn validate_request(
    catalog: &MetricCatalog,
    request: RowsRequest,
) -> Result<ValidatedRows, QueryError> {
    let metric_key = defined_metric(catalog, &request.metric)?;
    let subjects = subjects(request.subjects)?;
    let (from, to) = window(&request.time.from, &request.time.to)?;
    let filters = filters(catalog, &metric_key, request.filters)?;
    let display_dimensions = display_dimensions(catalog, &metric_key, request.display_dimensions)?;
    let input_role = input_role(catalog, &metric_key, request.input.as_deref())?;
    let page_size = page_size(request.page_size)?;

    Ok(ValidatedRows {
        metric_key,
        subjects,
        from,
        to,
        filters,
        input_role,
        display_dimensions,
        page_size,
        cursor: request.cursor,
    })
}

/// A page reports the events credited to a subject, and a dataset records no
/// event keyed by the tenant.
fn subjects(subjects: Subjects) -> Result<Vec<Uuid>, QueryError> {
    match subjects {
        Subjects::Persons { ids } => person_ids(SUBJECTS_FIELD, ids),
        Subjects::Tenant {} => Err(QueryError::Unanswerable {
            reason: "a page reports the events credited to a subject, and no event is recorded \
                     against the tenant itself",
        }),
    }
}

/// Which input the page reads. A metric composing one has an obvious answer; a
/// metric composing several has none, so the caller supplies it.
fn input_role(
    catalog: &MetricCatalog,
    metric_key: &str,
    asked: Option<&str>,
) -> Result<String, QueryError> {
    let Some(metric) = catalog.metric(metric_key) else {
        return Err(QueryError::UnknownMetric {
            metric: metric_key.to_owned(),
        });
    };
    let roles = catalog.input_roles(metric)?;

    let Some(asked) = asked.map(str::trim).filter(|asked| !asked.is_empty()) else {
        return match roles.as_slice() {
            [only] => Ok(only.clone()),
            _ => Err(QueryError::InputUnnamed {
                valid: named(&roles),
            }),
        };
    };

    if roles.iter().any(|role| role == asked) {
        return Ok(asked.to_owned());
    }
    Err(QueryError::UnknownInput {
        input: asked.to_owned(),
        valid: named(&roles),
    })
}

/// The dimensions a page reports beyond the measure's own. The compiler refuses
/// an undeclared one too; refusing here names the field the caller wrote it in.
fn display_dimensions(
    catalog: &MetricCatalog,
    metric_key: &str,
    asked: Vec<String>,
) -> Result<Vec<String>, QueryError> {
    if asked.len() > MAX_DISPLAY_DIMENSIONS {
        return Err(QueryError::TooManyDisplayDimensions {
            limit: MAX_DISPLAY_DIMENSIONS,
        });
    }

    let declared = catalog.dimension_keys(metric_key);
    let mut seen = BTreeSet::new();
    let mut kept = Vec::with_capacity(asked.len());
    for key in asked {
        let key = key.trim().to_owned();
        if !declared.iter().any(|declared| *declared == key) {
            return Err(QueryError::UnknownDisplayDimension { dimension: key });
        }
        if seen.insert(key.clone()) {
            kept.push(key);
        }
    }

    Ok(kept)
}

fn page_size(asked: Option<u32>) -> Result<u32, QueryError> {
    let size = asked.unwrap_or(DEFAULT_PAGE_SIZE);
    if size == 0 || size > MAX_PAGE_SIZE {
        return Err(QueryError::PageSizeOutOfRange {
            limit: MAX_PAGE_SIZE,
        });
    }
    Ok(size)
}

/// The inputs a metric composes, as a refusal quotes them.
fn named(roles: &[String]) -> String {
    roles
        .iter()
        .map(|role| format!("`{role}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::super::catalog::product_metric_catalog;
    use super::super::super::fixtures::{
        SHIPPED_METRIC, SHIPPED_RATIO_METRIC, shipped_input_roles,
    };
    use super::*;

    fn catalog() -> &'static MetricCatalog {
        product_metric_catalog().expect("the shipped definitions load")
    }

    fn person() -> Uuid {
        Uuid::from_u128(1)
    }

    fn request(overrides: &serde_json::Value) -> serde_json::Value {
        let mut request = serde_json::json!({
            "metric": SHIPPED_METRIC,
            "subjects": { "type": "persons", "ids": [person().to_string()] },
            "time": { "from": "2026-01-01", "to": "2026-01-31" },
        });
        let base = request.as_object_mut().expect("an object");
        for (key, value) in overrides.as_object().expect("an object") {
            base.insert(key.clone(), value.clone());
        }
        request
    }

    fn validate(overrides: &serde_json::Value) -> Result<ValidatedRows, QueryError> {
        let parsed: RowsRequest =
            serde_json::from_value(request(overrides)).expect("the wire shape parses");
        validate_request(catalog(), parsed)
    }

    #[test]
    fn a_metric_composing_one_input_pages_it_without_being_told_which() {
        let validated = validate(&serde_json::json!({})).expect("a shipped metric pages");

        assert_eq!(validated.input_role, "value");
    }

    #[test]
    fn a_metric_composing_several_inputs_is_paged_only_once_one_is_named() {
        let refused = validate(&serde_json::json!({ "metric": SHIPPED_RATIO_METRIC }))
            .expect_err("a composed metric has no obvious input");

        assert!(matches!(refused, QueryError::InputUnnamed { .. }));
        assert!(
            refused.to_string().contains("`numerator`")
                && refused.to_string().contains("`denominator`"),
            "the refusal names what may be asked for: {refused}"
        );

        let validated = validate(&serde_json::json!({
            "metric": SHIPPED_RATIO_METRIC,
            "input": "denominator",
        }))
        .expect("a named half pages");
        assert_eq!(validated.input_role, "denominator");
    }

    #[test]
    fn an_input_the_metric_does_not_compose_is_refused_and_told_which_ones_it_does() {
        let refused = validate(&serde_json::json!({ "input": "numerator" }))
            .expect_err("a direct metric composes no numerator");

        assert!(matches!(refused, QueryError::UnknownInput { .. }));
        assert!(
            refused.to_string().contains("`value`"),
            "the refusal names what may be asked for: {refused}"
        );
    }

    #[test]
    fn a_page_reports_between_one_row_and_the_ceiling() {
        for (size, named) in [
            (serde_json::json!(0), "a page of no rows"),
            (
                serde_json::json!(MAX_PAGE_SIZE + 1),
                "one row past the ceiling",
            ),
        ] {
            let refused = validate(&serde_json::json!({ "page_size": size }));

            assert!(
                matches!(refused, Err(QueryError::PageSizeOutOfRange { .. })),
                "should refuse: {named}"
            );
        }

        assert_eq!(
            validate(&serde_json::json!({}))
                .map(|validated| validated.page_size)
                .ok(),
            Some(DEFAULT_PAGE_SIZE)
        );
        assert!(validate(&serde_json::json!({ "page_size": MAX_PAGE_SIZE })).is_ok());
    }

    #[test]
    fn a_tenant_has_no_events_of_its_own_to_page() {
        let refused = validate(&serde_json::json!({ "subjects": { "type": "tenant" } }))
            .expect_err("nothing keys an event by the tenant");

        assert!(matches!(refused, QueryError::Unanswerable { .. }));
    }

    #[test]
    fn a_dimension_the_metric_does_not_declare_cannot_be_reported() {
        let refused = validate(&serde_json::json!({
            "display_dimensions": ["not_a_dimension"],
        }))
        .expect_err("an undeclared dimension names no column");

        assert!(matches!(
            refused,
            QueryError::UnknownDisplayDimension { .. }
        ));
    }

    #[test]
    fn a_dimension_named_twice_is_reported_once() {
        let validated = validate(&serde_json::json!({
            "display_dimensions": ["repository", "repository"],
        }))
        .expect("a repeated dimension is one dimension");

        assert_eq!(validated.display_dimensions, ["repository"]);
    }

    #[test]
    fn the_same_subject_named_twice_is_read_once() {
        let validated = validate(&serde_json::json!({
            "subjects": { "type": "persons", "ids": [person().to_string(), person().to_string()] },
        }))
        .expect("a repeated subject is one subject");

        assert_eq!(validated.subjects, vec![person()]);
    }

    #[test]
    fn a_subject_list_is_refused_when_it_is_empty_or_unreadable() {
        for ids in [serde_json::json!([]), serde_json::json!(["nobody"])] {
            let named = ids.to_string();

            let outcome = validate(&serde_json::json!({
                "subjects": { "type": "persons", "ids": ids },
            }));

            assert!(outcome.is_err(), "should refuse: {named}");
        }
    }

    #[test]
    fn every_shipped_metric_pages_each_input_it_composes() {
        for (metric_key, roles) in shipped_input_roles() {
            let unnamed = validate(&serde_json::json!({ "metric": metric_key }));
            if roles.len() == 1 {
                assert_eq!(
                    unnamed.map(|validated| validated.input_role).ok().as_ref(),
                    roles.first(),
                    "{metric_key} pages its only input unasked"
                );
            } else {
                assert!(
                    matches!(unnamed, Err(QueryError::InputUnnamed { .. })),
                    "{metric_key} composes {roles:?} and names none of them by default"
                );
            }

            for role in &roles {
                let validated = validate(&serde_json::json!({
                    "metric": metric_key,
                    "input": role,
                }));

                assert_eq!(
                    validated.map(|validated| validated.input_role).ok(),
                    Some(role.clone()),
                    "{metric_key} pages its `{role}`"
                );
            }
        }
    }
}
