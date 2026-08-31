//! What each distribution compiles to, decided before anything is read: which
//! source identities its people are known by, and which of the two readings
//! the question asked for. The mapping is read once for the whole request.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::domain::compiler::request::{
    BinsView, Bucket, EntityScope, MetricQuery, QuantilesView, ResolvedPerson, ViewKind,
};
use crate::domain::compiler::sql::CompiledMeasureQuery;
use crate::domain::identity_binding::{IdentitySet, resolve_identities};

use super::super::catalog::MetricCatalog;
use super::super::error::QueryError;
use super::super::question::{ValidatedSubjects, query_row_limit};
use super::validation::{ValidatedDistribution, ValidatedDistributions};

// INVARIANT: a distribution reports the whole window at once, so neither read
// folds to a bucket and this field goes unread.
const UNREAD_BUCKET: Bucket = Bucket::Day;

/// The statements one question runs: one per reading it asked for. A question
/// the mapping knows no identity for anyone in runs neither — no row can carry
/// one of their observations, so both readings are assembled from nothing.
#[derive(Debug, PartialEq)]
pub(super) struct PlannedDistribution {
    pub histogram: Option<CompiledMeasureQuery>,
    pub quantiles: Option<CompiledMeasureQuery>,
}

/// The statements one request runs, in the order its questions were asked.
pub(super) async fn plan(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    batch: &ValidatedDistributions,
) -> Result<Vec<PlannedDistribution>, QueryError> {
    let identities = identities(clickhouse, tenant_id, batch).await?;

    batch
        .queries
        .iter()
        .map(|query| {
            let scope = entity_scope(&query.subjects, &identities);
            compile(catalog, tenant_id, query, &scope)
        })
        .collect()
}

fn compile(
    catalog: &MetricCatalog,
    tenant_id: Uuid,
    query: &ValidatedDistribution,
    scope: &EntityScope,
) -> Result<PlannedDistribution, QueryError> {
    // INVARIANT: validation refuses a metric the definitions do not carry, so
    // every planned question names one they do.
    let Some(metric) = catalog.metric(&query.metric_key) else {
        return Err(QueryError::UnknownMetric {
            metric: query.metric_key.clone(),
        });
    };
    if !scope.reaches_any_row() {
        return Ok(PlannedDistribution {
            histogram: None,
            quantiles: None,
        });
    }

    let read = |view: ViewKind| {
        catalog
            .compile(
                metric,
                &MetricQuery {
                    tenant_id: tenant_id.to_string(),
                    entity_scope: scope.clone(),
                    from: query.from,
                    to: query.to,
                    bucket: UNREAD_BUCKET,
                    dimension_filters: query.filters.clone(),
                    view,
                    row_limit: query_row_limit(),
                },
            )
            .map_err(QueryError::from)
    };

    let histogram = query
        .bins
        .map(|bins| read(ViewKind::Bins(BinsView { bins })))
        .transpose()?;
    let quantiles = query
        .quantiles
        .as_ref()
        .map(|quantiles| {
            read(ViewKind::Quantiles(QuantilesView {
                quantiles: quantiles.clone(),
            }))
        })
        .transpose()?;

    Ok(PlannedDistribution {
        histogram,
        quantiles,
    })
}

/// The identity values every person the request names is known by, in one read.
/// A mapping that cannot be read refuses rather than scoping a read to nobody.
async fn identities(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    batch: &ValidatedDistributions,
) -> Result<BTreeMap<Uuid, IdentitySet>, QueryError> {
    let people = batch.subject_ids();
    if people.is_empty() {
        return Ok(BTreeMap::new());
    }

    resolve_identities(clickhouse, tenant_id, &people)
        .await
        .map_err(|error| {
            tracing::error!(
                %error,
                "a distribution could not resolve the people it asks about"
            );
            QueryError::SubjectsUnresolved
        })
}

/// INVARIANT: a person is carried with their own identities rather than merged
/// into one list, because the answer is keyed per person.
fn entity_scope(
    subjects: &ValidatedSubjects,
    identities: &BTreeMap<Uuid, IdentitySet>,
) -> EntityScope {
    match subjects {
        ValidatedSubjects::Tenant => EntityScope::Tenant,
        ValidatedSubjects::Persons(ids) => EntityScope::People(
            ids.iter()
                .map(|id| ResolvedPerson {
                    person_ref: id.to_string(),
                    identities: identities
                        .get(id)
                        .map(IdentitySet::values)
                        .unwrap_or_default(),
                })
                .collect(),
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::num::NonZeroU32;

    use chrono::NaiveDate;

    use crate::domain::compiler::sql::QueryParam;

    use super::super::super::catalog::product_metric_catalog;
    use super::super::super::fixtures::{
        SHIPPED_DISTRIBUTION_METRIC, offline_clickhouse, shipped_subjects, tenant,
    };
    use super::*;

    fn person() -> Uuid {
        Uuid::from_u128(1)
    }

    fn validated(bins: Option<u32>, quantiles: Option<Vec<f64>>) -> ValidatedDistribution {
        ValidatedDistribution {
            metric_key: SHIPPED_DISTRIBUTION_METRIC.to_owned(),
            subjects: ValidatedSubjects::Persons(vec![person()]),
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            filters: Vec::new(),
            bins: bins.and_then(NonZeroU32::new),
            quantiles,
        }
    }

    fn scope() -> EntityScope {
        EntityScope::People(vec![ResolvedPerson {
            person_ref: person().to_string(),
            identities: vec!["dev@example.com".to_owned()],
        }])
    }

    fn planned(bins: Option<u32>, quantiles: Option<Vec<f64>>) -> PlannedDistribution {
        compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &validated(bins, quantiles),
            &scope(),
        )
        .expect("a shipped distribution metric compiles")
    }

    #[test]
    fn a_question_compiles_one_read_per_reading_it_asked_for() {
        let cases = [
            (Some(10), None, true, false),
            (None, Some(vec![0.5]), false, true),
            (Some(4), Some(vec![0.5, 0.9]), true, true),
        ];

        for (bins, quantiles, reads_histogram, reads_quantiles) in cases {
            let named = format!("bins={bins:?} quantiles={quantiles:?}");
            let planned = planned(bins, quantiles);

            assert_eq!(planned.histogram.is_some(), reads_histogram, "{named}");
            assert_eq!(planned.quantiles.is_some(), reads_quantiles, "{named}");
        }
    }

    #[test]
    fn both_reads_reach_their_subjects_through_the_pool_and_key_rows_by_the_person() {
        let planned = planned(Some(4), Some(vec![0.5]));

        for read in [
            planned.histogram.expect("a histogram was asked for"),
            planned.quantiles.expect("quantiles were asked for"),
        ] {
            assert!(
                read.sql.contains("INNER JOIN pool ON pool.identity = "),
                "{}",
                read.sql
            );
            assert!(
                read.sql.contains("pool.person_ref AS entity_id,"),
                "{}",
                read.sql
            );
            assert_eq!(read.sql.matches('?').count(), read.params.len());
        }
    }

    #[test]
    fn the_bin_count_a_question_names_is_the_one_the_read_cuts() {
        let planned = planned(Some(4), None);
        let histogram = planned.histogram.expect("a histogram was asked for");

        assert!(
            histogram.sql.contains("toUInt32(least(3, toInt64(floor("),
            "{}",
            histogram.sql
        );
    }

    #[test]
    fn every_position_a_question_names_is_taken_in_one_read() {
        let planned = planned(None, Some(vec![0.25, 0.5, 0.75]));
        let quantiles = planned.quantiles.expect("quantiles were asked for");

        assert!(
            quantiles.sql.contains("quantilesExact(0.25, 0.5, 0.75)("),
            "{}",
            quantiles.sql
        );
    }

    /// A person the mapping knows nothing about reads as no data, not as a bad
    /// request: no reading is taken, and the answer is the empty distribution.
    #[test]
    fn a_question_no_identity_resolves_for_takes_neither_reading() {
        let query = validated(Some(10), Some(vec![0.5]));

        let planned = compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &query,
            &entity_scope(&query.subjects, &BTreeMap::new()),
        )
        .expect("a question nobody's identities reach is still answerable");

        assert_eq!(
            planned,
            PlannedDistribution {
                histogram: None,
                quantiles: None,
            }
        );
    }

    /// A set the mapping answers for in part is read for the part it answers
    /// for; the members it names nothing for contribute nothing.
    #[test]
    fn a_question_one_of_whose_people_resolves_is_read_for_that_person() {
        let unresolved = Uuid::from_u128(9);
        let query = ValidatedDistribution {
            subjects: ValidatedSubjects::Persons(vec![person(), unresolved]),
            ..validated(Some(10), None)
        };
        let known = BTreeMap::from([(
            person(),
            IdentitySet {
                emails: vec!["dev@example.com".to_owned()],
                account_ids: Vec::new(),
            },
        )]);

        let planned = compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &query,
            &entity_scope(&query.subjects, &known),
        )
        .expect("a question one of whose people resolves is read");

        let histogram = planned.histogram.expect("a histogram was asked for");
        assert!(
            histogram
                .params
                .contains(&QueryParam::Text("dev@example.com".to_owned())),
            "the resolved member's identity is bound"
        );
    }

    #[test]
    fn a_person_the_mapping_answers_nothing_for_is_scoped_to_no_identity_at_all() {
        let scope = entity_scope(
            &ValidatedSubjects::Persons(vec![person()]),
            &BTreeMap::new(),
        );

        assert_eq!(
            scope,
            EntityScope::People(vec![ResolvedPerson {
                person_ref: person().to_string(),
                identities: Vec::new(),
            }])
        );
    }

    #[tokio::test]
    async fn a_mapping_no_server_answers_for_refuses_the_whole_request() {
        let batch = ValidatedDistributions {
            queries: vec![validated(Some(10), None)],
        };

        let outcome = plan(
            product_metric_catalog().expect("loads"),
            &offline_clickhouse(),
            tenant(),
            &batch,
        )
        .await;

        assert!(matches!(
            outcome.expect_err("a closed port cannot answer"),
            QueryError::SubjectsUnresolved
        ));
    }

    /// The endpoint admits exactly the metrics the compiler's own rule does:
    /// those whose computation is taken over the measure's per-row values.
    #[test]
    fn every_shipped_metric_answers_a_distribution_only_where_its_computation_ranks_rows() {
        use crate::domain::definitions::definition::Computation;

        use super::super::super::error::QueryError;
        use super::super::dto::DistributionsRequest;
        use super::super::validation::validate_request;

        let catalog = product_metric_catalog().expect("loads");
        let definitions = crate::domain::definitions::seeds::product_definitions()
            .expect("the shipped definitions are valid");
        let mut admitted = 0_usize;

        for metric in &definitions.metrics {
            let request: DistributionsRequest = serde_json::from_value(serde_json::json!({
                "queries": [{
                    "metric": metric.key,
                    "subjects": shipped_subjects(&metric.key),
                    "time": { "from": "2026-01-01", "to": "2026-01-31" },
                    "bins": 10,
                    "quantiles": [0.5],
                }],
            }))
            .expect("the wire shape parses");

            let validated = validate_request(catalog, request);
            let ranks_rows = matches!(
                metric.computation,
                Computation::Percentile { .. } | Computation::Stddev { .. }
            );

            if !ranks_rows {
                assert!(
                    matches!(validated, Err(QueryError::NoDistribution(_))),
                    "metric `{}` has no distribution: {validated:?}",
                    metric.key
                );
                continue;
            }

            admitted += 1;
            let batch = validated
                .unwrap_or_else(|error| panic!("metric `{}` distributes: {error}", metric.key));
            let planned = compile(catalog, tenant(), &batch.queries[0], &scope())
                .unwrap_or_else(|error| panic!("metric `{}` compiles: {error}", metric.key));

            for read in [
                planned.histogram.expect("a histogram was asked for"),
                planned.quantiles.expect("quantiles were asked for"),
            ] {
                assert_eq!(
                    read.sql.matches('?').count(),
                    read.params.len(),
                    "metric `{}`",
                    metric.key
                );
            }
        }

        assert!(
            admitted > 0,
            "the shipped definitions carry a metric with a distribution"
        );
    }
}
