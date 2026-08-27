//! What a page compiles to, decided before anything is read: the identities its
//! people are known by, the marker of the mapping that named them, and the one
//! input of the metric's computation the page reads.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::domain::compiler::drilldown::CompiledDrilldown;
use crate::domain::compiler::request::{
    DrilldownCursor, DrilldownQuery, EntityScope, ResolvedPerson,
};
use crate::domain::identity_binding::{IdentitySet, identity_epoch, resolve_identities};

use super::super::catalog::MetricCatalog;
use super::super::error::QueryError;
use super::cursor::Resume;
use super::validation::ValidatedRows;

/// The one statement a page runs, and the mapping marker its rows were
/// attributed through.
#[derive(Debug)]
pub(super) struct PlannedPage {
    pub compiled: CompiledDrilldown,
    pub identity_epoch: u64,
}

pub(super) async fn plan(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    request: &ValidatedRows,
    resume: Option<&Resume>,
) -> Result<PlannedPage, QueryError> {
    // INVARIANT: the marker is read beside the mapping it describes, so it is
    // the epoch this page's pool was actually built from.
    let (identities, epoch) = tokio::join!(
        resolve_identities(clickhouse, tenant_id, &request.subjects),
        identity_epoch(clickhouse, tenant_id)
    );
    let identities = identities.map_err(|error| {
        tracing::error!(%error, "a page could not resolve the people it asks about");
        QueryError::SubjectsUnresolved
    })?;
    let identity_epoch = epoch.map_err(|error| {
        tracing::error!(%error, "a page could not mark the identity mapping it read");
        QueryError::SubjectsUnresolved
    })?;

    let compiled = compile(catalog, tenant_id, request, &identities, resume)?;
    Ok(PlannedPage {
        compiled,
        identity_epoch,
    })
}

fn compile(
    catalog: &MetricCatalog,
    tenant_id: Uuid,
    request: &ValidatedRows,
    identities: &BTreeMap<Uuid, IdentitySet>,
    resume: Option<&Resume>,
) -> Result<CompiledDrilldown, QueryError> {
    // INVARIANT: validation refuses a metric the definitions do not carry, so
    // every planned page names one they do.
    let Some(metric) = catalog.metric(&request.metric_key) else {
        return Err(QueryError::UnknownMetric {
            metric: request.metric_key.clone(),
        });
    };

    let query = DrilldownQuery {
        tenant_id: tenant_id.to_string(),
        entity_scope: entity_scope(&request.subjects, identities),
        from: request.from,
        to: request.to,
        dimension_filters: request.filters.clone(),
        display_dimensions: request.display_dimensions.clone(),
        page_size: u64::from(request.page_size),
        cursor: resume.map(|resume| DrilldownCursor {
            sort_values: resume.sort_values.clone(),
        }),
    };

    let mut pages = catalog.compile_drilldown(metric, &query)?;
    let Some(asked) = pages
        .iter()
        .position(|page| page.input_role == request.input_role)
    else {
        return Err(QueryError::UnknownInput {
            input: request.input_role.clone(),
            valid: pages
                .iter()
                .map(|page| format!("`{}`", page.input_role))
                .collect::<Vec<_>>()
                .join(", "),
        });
    };
    Ok(pages.swap_remove(asked))
}

/// INVARIANT: a person is carried with their own identities rather than merged
/// into one list, because every row is keyed by the person it is credited to.
fn entity_scope(subjects: &[Uuid], identities: &BTreeMap<Uuid, IdentitySet>) -> EntityScope {
    EntityScope::People(
        subjects
            .iter()
            .map(|id| ResolvedPerson {
                person_ref: id.to_string(),
                identities: identities
                    .get(id)
                    .map(IdentitySet::values)
                    .unwrap_or_default(),
            })
            .collect(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use crate::domain::compiler::drilldown::DrilldownColumnKind;
    use crate::domain::compiler::sql::QueryParam;

    use super::super::super::catalog::product_metric_catalog;
    use super::super::super::fixtures::{
        SHIPPED_METRIC, SHIPPED_RATIO_METRIC, offline_clickhouse, shipped_input_roles, tenant,
    };
    use super::super::cursor::Anchor;
    use super::*;

    fn person() -> Uuid {
        Uuid::from_u128(1)
    }

    fn catalog() -> &'static MetricCatalog {
        product_metric_catalog().expect("the shipped definitions load")
    }

    fn request(metric_key: &str, input_role: &str) -> ValidatedRows {
        ValidatedRows {
            metric_key: metric_key.to_owned(),
            subjects: vec![person()],
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            filters: Vec::new(),
            input_role: input_role.to_owned(),
            display_dimensions: Vec::new(),
            page_size: 100,
            cursor: None,
        }
    }

    fn resolved() -> BTreeMap<Uuid, IdentitySet> {
        BTreeMap::from([(
            person(),
            IdentitySet {
                emails: vec!["dev@example.com".to_owned()],
                account_ids: Vec::new(),
            },
        )])
    }

    fn compiled(request: &ValidatedRows, resume: Option<&Resume>) -> CompiledDrilldown {
        compile(catalog(), tenant(), request, &resolved(), resume)
            .unwrap_or_else(|error| panic!("`{}` compiles: {error}", request.metric_key))
    }

    #[test]
    fn a_page_reaches_its_rows_through_the_pool_that_keys_them_by_person() {
        let compiled = compiled(&request(SHIPPED_METRIC, "value"), None);

        assert!(
            compiled.sql.contains("INNER JOIN pool ON pool.identity = "),
            "{}",
            compiled.sql
        );
        assert!(compiled.sql.contains("    pool.person_ref AS entity_id,"));
        assert!(
            compiled
                .params
                .contains(&QueryParam::Text("dev@example.com".to_owned())),
            "every resolved identity is bound"
        );
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn a_composed_metric_runs_only_the_half_the_question_named() {
        let numerator = compiled(&request(SHIPPED_RATIO_METRIC, "numerator"), None);
        let denominator = compiled(&request(SHIPPED_RATIO_METRIC, "denominator"), None);

        assert_eq!(numerator.input_role, "numerator");
        assert_eq!(denominator.input_role, "denominator");
        assert_ne!(numerator.sql, denominator.sql);
    }

    #[test]
    fn a_page_reads_one_row_beyond_its_size_so_a_further_page_is_detectable() {
        let asked = ValidatedRows {
            page_size: 25,
            ..request(SHIPPED_METRIC, "value")
        };

        let compiled = compiled(&asked, None);

        assert_eq!(compiled.params.last(), Some(&QueryParam::UInt(26)));
    }

    #[test]
    fn a_resumed_page_binds_the_position_it_carries_rather_than_writing_it_in() {
        let arity = compiled(&request(SHIPPED_METRIC, "value"), None)
            .columns
            .iter()
            .filter(|column| matches!(column.kind, DrilldownColumnKind::SortKey(_)))
            .count();
        let resume = Resume {
            anchor: Anchor {
                snapshot_id: "dataset-uuid".to_owned(),
                identity_epoch: 1,
            },
            sort_values: vec!["'; DROP TABLE x; --".to_owned(); arity],
        };

        let compiled = compiled(&request(SHIPPED_METRIC, "value"), Some(&resume));

        assert!(!compiled.sql.contains("DROP TABLE"), "{}", compiled.sql);
        assert!(compiled.sql.contains("> tuple("));
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    /// The compiler is the backstop behind validation, which refuses an input
    /// the metric does not compose before anything is planned.
    #[test]
    fn an_input_the_metric_does_not_compose_names_no_page() {
        let error = compile(
            catalog(),
            tenant(),
            &request(SHIPPED_METRIC, "numerator"),
            &resolved(),
            None,
        )
        .expect_err("a direct metric composes no numerator");

        assert!(matches!(error, QueryError::UnknownInput { .. }), "{error}");
    }

    #[test]
    fn every_shipped_metric_compiles_a_page_for_every_input_it_composes() {
        for (metric_key, roles) in shipped_input_roles() {
            for role in roles {
                let compiled = compiled(&request(metric_key, &role), None);

                assert_eq!(
                    compiled.sql.matches('?').count(),
                    compiled.params.len(),
                    "`{metric_key}` binds one parameter per placeholder in its `{role}` page"
                );
                assert!(
                    compiled
                        .columns
                        .iter()
                        .any(|column| matches!(column.kind, DrilldownColumnKind::SortKey(_))),
                    "`{metric_key}` orders its `{role}` page totally"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_mapping_that_does_not_answer_refuses_the_page_rather_than_widening_it() {
        let error = plan(
            catalog(),
            &offline_clickhouse(),
            tenant(),
            &request(SHIPPED_METRIC, "value"),
            None,
        )
        .await
        .expect_err("a closed port cannot answer");

        assert!(matches!(error, QueryError::SubjectsUnresolved));
    }
}
