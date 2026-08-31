//! What each question compiles to, decided before anything is read: which
//! source identities its people are known by, and which groups a capped split
//! keeps. Both are shared across the questions of one request, and both arrive
//! resolved at the compiler, which looks nothing up.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use crate::domain::compiler::request::{
    Bucket, CombinedSplitView, EntityScope, GroupLimit, GroupRankingQuery, MetricQuery,
    RankedGroup, ResolvedPerson, SubjectSeriesView, SubjectSplitView, ViewKind,
};
use crate::domain::compiler::sql::CompiledMeasureQuery;
use crate::domain::identity_binding::{IdentitySet, resolve_identities};

use super::catalog::MetricCatalog;
use super::dto::Grain;
use super::error::QueryError;
use super::group_cap::ranked_groups;
use super::validation::{
    QueryShape, ValidatedBatch, ValidatedQuery, ValidatedSplit, ValidatedSubjects, query_row_limit,
};

/// Everything a ranking read's answer depends on, beyond the request's own
/// window. Two questions that agree on all of it share one read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RankingKey {
    rank_metric_key: String,
    dimensions: Vec<String>,
    subjects: Vec<String>,
    top: u32,
}

/// The statements one request runs, in the order its questions were asked.
pub async fn plan(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    batch: &ValidatedBatch,
) -> Result<Vec<CompiledMeasureQuery>, QueryError> {
    let identities = identities(clickhouse, tenant_id, batch).await?;

    let mut rankings: BTreeMap<RankingKey, Vec<RankedGroup>> = BTreeMap::new();
    let mut compiled = Vec::with_capacity(batch.queries.len());
    for query in &batch.queries {
        let scope = entity_scope(&query.subjects, &identities);
        let limit = cap(catalog, clickhouse, tenant_id, &mut rankings, query, &scope).await?;
        compiled.push(compile(catalog, tenant_id, query, scope, limit)?);
    }
    Ok(compiled)
}

fn compile(
    catalog: &MetricCatalog,
    tenant_id: Uuid,
    query: &ValidatedQuery,
    scope: EntityScope,
    limit: Option<GroupLimit>,
) -> Result<CompiledMeasureQuery, QueryError> {
    // INVARIANT: validation refuses a metric the definitions do not carry, so
    // every planned question names one they do.
    let Some(metric) = catalog.metric(&query.metric_key) else {
        return Err(QueryError::UnknownMetric {
            metric: query.metric_key.clone(),
        });
    };

    let compiled = catalog.compile(
        metric,
        &MetricQuery {
            tenant_id: tenant_id.to_string(),
            entity_scope: scope,
            from: query.from,
            to: query.to,
            bucket: bucket(query.grain),
            dimension_filters: Vec::new(),
            view: view(query, limit),
            row_limit: query_row_limit(),
        },
    )?;
    Ok(compiled)
}

/// The compiler's view for one question's shape. A shape that reports no group
/// carries no dimensions, and only the two capped shapes carry a limit.
fn view(query: &ValidatedQuery, limit: Option<GroupLimit>) -> ViewKind {
    let dimensions = query.dimensions().to_vec();
    match query.shape {
        QueryShape::SubjectTotal => ViewKind::SubjectTotal,
        QueryShape::SubjectSplit => ViewKind::SubjectSplit(SubjectSplitView { dimensions }),
        QueryShape::CombinedSplit => ViewKind::CombinedSplit(CombinedSplitView {
            dimensions,
            group_limit: limit,
        }),
        QueryShape::SubjectSeries => ViewKind::SubjectSeries(SubjectSeriesView {
            dimensions,
            group_limit: limit,
        }),
    }
}

// INVARIANT: only a series read folds to a bucket. Every other shape leaves
// this unread, so the grain of a question that names none decides nothing.
fn bucket(grain: Grain) -> Bucket {
    match grain {
        Grain::Day | Grain::Total => Bucket::Day,
        Grain::Week => Bucket::Week,
        Grain::Month => Bucket::Month,
    }
}

/// The identity values every person the request names is known by, in one read.
/// A mapping that cannot be read refuses rather than scoping a read to nobody.
async fn identities(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    batch: &ValidatedBatch,
) -> Result<BTreeMap<Uuid, IdentitySet>, QueryError> {
    let people: Vec<Uuid> = batch
        .queries
        .iter()
        .filter_map(|query| match &query.subjects {
            ValidatedSubjects::Persons(ids) => Some(ids.iter().copied()),
            ValidatedSubjects::Tenant => None,
        })
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if people.is_empty() {
        return Ok(BTreeMap::new());
    }

    resolve_identities(clickhouse, tenant_id, &people)
        .await
        .map_err(|error| {
            tracing::error!(%error, "a values question could not resolve the people it asks about");
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
                    identities: resolved_identities(identities.get(id)),
                })
                .collect(),
        ),
    }
}

/// One person's identity values, deduplicated: an identity a source recorded
/// under both an address and an account id must not attribute its rows twice.
fn resolved_identities(set: Option<&IdentitySet>) -> Vec<String> {
    let Some(set) = set else {
        return Vec::new();
    };

    let mut values: BTreeSet<&String> = set.emails.iter().collect();
    values.extend(set.account_ids.iter());
    values.into_iter().cloned().collect()
}

async fn cap(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    rankings: &mut BTreeMap<RankingKey, Vec<RankedGroup>>,
    query: &ValidatedQuery,
    scope: &EntityScope,
) -> Result<Option<GroupLimit>, QueryError> {
    let Some(ValidatedSplit {
        dimensions,
        limit: Some(limit),
    }) = query.split.as_ref()
    else {
        return Ok(None);
    };

    let key = RankingKey {
        rank_metric_key: limit.rank_by.clone(),
        dimensions: dimensions.clone(),
        subjects: scoped_refs(scope),
        top: limit.top,
    };
    if let Some(groups) = rankings.get(&key) {
        return Ok(Some(GroupLimit {
            groups: groups.clone(),
            include_remainder: limit.remainder,
        }));
    }

    let ranking = GroupRankingQuery {
        tenant_id: tenant_id.to_string(),
        entity_scope: scope.clone(),
        from: query.from,
        to: query.to,
        dimension_filters: Vec::new(),
        dimensions: dimensions.clone(),
        count: u64::from(limit.top),
    };
    let groups = ranked_groups(catalog, clickhouse, &limit.rank_by, &ranking).await?;
    rankings.insert(key, groups.clone());

    Ok(Some(GroupLimit {
        groups,
        include_remainder: limit.remainder,
    }))
}

/// The scope as a ranking key can compare it. Two questions ranked over
/// different people rank different groups.
fn scoped_refs(scope: &EntityScope) -> Vec<String> {
    match scope {
        EntityScope::Tenant => Vec::new(),
        EntityScope::Identities(values) => values.clone(),
        EntityScope::People(people) => people
            .iter()
            .map(|person| person.person_ref.clone())
            .collect(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use super::super::catalog::product_metric_catalog;
    use super::super::fixtures::{SHIPPED_METRIC, offline_clickhouse, validated};
    use super::*;
    use crate::domain::compiler::sql::QueryParam;

    fn person() -> Uuid {
        Uuid::from_u128(1)
    }

    fn tenant() -> Uuid {
        Uuid::from_u128(0x7e_11a7)
    }

    fn people_scope() -> EntityScope {
        EntityScope::People(vec![ResolvedPerson {
            person_ref: person().to_string(),
            identities: vec!["dev@example.com".to_owned()],
        }])
    }

    fn compiled(query: &ValidatedQuery, scope: EntityScope) -> CompiledMeasureQuery {
        compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            query,
            scope,
            None,
        )
        .unwrap_or_else(|error| panic!("{:?} compiles: {error}", query.shape))
    }

    #[test]
    fn each_shape_compiles_the_read_that_answers_it() {
        let cases = [
            (
                validated(QueryShape::SubjectTotal, Grain::Total, &[]),
                "GROUP BY entity_id",
            ),
            (
                validated(QueryShape::SubjectSplit, Grain::Total, &["repository"]),
                "GROUP BY entity_id, dim_0_value, dim_0_label",
            ),
            (
                validated(QueryShape::CombinedSplit, Grain::Total, &["repository"]),
                "contributing_entity_count",
            ),
            (
                validated(QueryShape::SubjectSeries, Grain::Week, &[]),
                "GROUP BY GROUPING SETS",
            ),
        ];

        for (query, marker) in cases {
            let sql = compiled(&query, people_scope()).sql;

            assert!(sql.contains(marker), "{:?}: {sql}", query.shape);
        }
    }

    #[test]
    fn a_series_read_folds_to_the_grain_the_question_named() {
        for (grain, marker) in [
            (Grain::Day, "toDate("),
            (Grain::Week, "toStartOfWeek("),
            (Grain::Month, "toStartOfMonth("),
        ] {
            let query = validated(QueryShape::SubjectSeries, grain, &[]);

            let sql = compiled(&query, people_scope()).sql;

            assert!(sql.contains(marker), "{grain:?}: {sql}");
        }
    }

    #[test]
    fn a_question_about_people_reaches_its_rows_through_the_pool_that_keys_them() {
        let query = validated(QueryShape::SubjectTotal, Grain::Total, &[]);

        let compiled = compiled(&query, people_scope());

        assert!(
            compiled.sql.contains("INNER JOIN pool ON pool.identity = "),
            "{}",
            compiled.sql
        );
        assert!(
            compiled
                .params
                .contains(&QueryParam::Text("dev@example.com".to_owned())),
            "every resolved identity is bound"
        );
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn a_tenant_question_reads_without_an_entity_predicate() {
        let query = validated(QueryShape::CombinedSplit, Grain::Total, &["repository"]);

        let compiled = compiled(&query, EntityScope::Tenant);

        assert!(
            !compiled.sql.contains("INNER JOIN pool"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_dimension_the_definitions_do_not_declare_does_not_compile() {
        let query = validated(QueryShape::SubjectSplit, Grain::Total, &["not_a_dimension"]);

        let error = compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &query,
            people_scope(),
            None,
        )
        .expect_err("an undeclared dimension names no column");

        assert!(matches!(error, QueryError::Uncompilable(_)), "{error}");
    }

    #[test]
    fn each_person_keeps_their_own_identities_rather_than_sharing_one_list() {
        let alice = Uuid::from_u128(1);
        let bob = Uuid::from_u128(2);
        let resolved = BTreeMap::from([
            (
                alice,
                IdentitySet {
                    emails: vec!["alice@example.com".to_owned()],
                    account_ids: vec!["acct-1".to_owned()],
                },
            ),
            (
                bob,
                IdentitySet {
                    emails: vec!["bob@example.com".to_owned()],
                    account_ids: Vec::new(),
                },
            ),
        ]);

        let scope = entity_scope(&ValidatedSubjects::Persons(vec![alice, bob]), &resolved);

        assert_eq!(
            scope,
            EntityScope::People(vec![
                ResolvedPerson {
                    person_ref: alice.to_string(),
                    identities: vec!["acct-1".to_owned(), "alice@example.com".to_owned()],
                },
                ResolvedPerson {
                    person_ref: bob.to_string(),
                    identities: vec!["bob@example.com".to_owned()],
                },
            ])
        );
    }

    #[test]
    fn an_identity_recorded_as_both_an_address_and_an_account_is_carried_once() {
        let resolved = BTreeMap::from([(
            person(),
            IdentitySet {
                emails: vec!["dev@example.com".to_owned()],
                account_ids: vec!["dev@example.com".to_owned()],
            },
        )]);

        let scope = entity_scope(&ValidatedSubjects::Persons(vec![person()]), &resolved);

        assert_eq!(
            scope,
            EntityScope::People(vec![ResolvedPerson {
                person_ref: person().to_string(),
                identities: vec!["dev@example.com".to_owned()],
            }])
        );
    }

    #[test]
    fn a_person_the_mapping_answers_nothing_for_is_named_with_no_identity_at_all() {
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
    async fn a_request_that_names_nobody_asks_the_identity_mapping_nothing() {
        let batch = ValidatedBatch {
            queries: vec![ValidatedQuery {
                subjects: ValidatedSubjects::Tenant,
                ..validated(QueryShape::CombinedSplit, Grain::Total, &["repository"])
            }],
        };

        let resolved = identities(&offline_clickhouse(), tenant(), &batch)
            .await
            .expect("a tenant question resolves nobody");

        assert!(resolved.is_empty());
    }

    #[tokio::test]
    async fn a_mapping_that_does_not_answer_refuses_the_request() {
        let batch = ValidatedBatch {
            queries: vec![validated(QueryShape::SubjectTotal, Grain::Total, &[])],
        };

        let error = identities(&offline_clickhouse(), tenant(), &batch)
            .await
            .expect_err("a closed port cannot answer");

        assert!(matches!(error, QueryError::SubjectsUnresolved));
    }

    #[tokio::test]
    async fn one_cap_policy_is_ranked_once_however_many_questions_share_it() {
        let mut rankings = BTreeMap::from([(
            RankingKey {
                rank_metric_key: SHIPPED_METRIC.to_owned(),
                dimensions: vec!["repository".to_owned()],
                subjects: vec![person().to_string()],
                top: 5,
            },
            vec![RankedGroup {
                rank: 1,
                dimensions: vec![crate::domain::compiler::request::RankedDimension {
                    value: "example/app".to_owned(),
                    label: None,
                }],
            }],
        )]);
        let catalog = product_metric_catalog().expect("loads");

        for shape in [QueryShape::SubjectSeries, QueryShape::CombinedSplit] {
            let grain = match shape {
                QueryShape::SubjectSeries => Grain::Day,
                QueryShape::SubjectTotal | QueryShape::SubjectSplit | QueryShape::CombinedSplit => {
                    Grain::Total
                }
            };
            let mut query = validated(shape, grain, &["repository"]);
            query.split = Some(ValidatedSplit {
                dimensions: vec!["repository".to_owned()],
                limit: Some(super::super::validation::ValidatedSplitLimit {
                    top: 5,
                    rank_by: SHIPPED_METRIC.to_owned(),
                    remainder: true,
                }),
            });

            let limit = cap(
                catalog,
                &offline_clickhouse(),
                tenant(),
                &mut rankings,
                &query,
                &people_scope(),
            )
            .await
            .unwrap_or_else(|error| panic!("{shape:?} reuses the ranked groups: {error}"));

            assert_eq!(
                limit.map(|limit| limit.groups.len()),
                Some(1),
                "{shape:?} keeps the groups the ranking already named"
            );
        }
        assert_eq!(rankings.len(), 1);
    }

    #[test]
    fn a_cap_binds_its_groups_into_the_read_it_shapes() {
        let mut query = validated(QueryShape::CombinedSplit, Grain::Total, &["repository"]);
        query.split = Some(ValidatedSplit {
            dimensions: vec!["repository".to_owned()],
            limit: Some(super::super::validation::ValidatedSplitLimit {
                top: 2,
                rank_by: SHIPPED_METRIC.to_owned(),
                remainder: true,
            }),
        });

        let compiled = compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &query,
            EntityScope::Tenant,
            Some(GroupLimit {
                groups: vec![RankedGroup {
                    rank: 1,
                    dimensions: vec![crate::domain::compiler::request::RankedDimension {
                        value: "example/app".to_owned(),
                        label: Some("Example App".to_owned()),
                    }],
                }],
                include_remainder: true,
            }),
        )
        .expect("a capped rollup compiles once its groups are ranked");

        assert!(compiled.sql.contains("AS group_rank"), "{}", compiled.sql);
        for value in ["example/app", "Example App"] {
            assert!(
                compiled
                    .params
                    .contains(&QueryParam::Text(value.to_owned())),
                "the cap binds `{value}` rather than writing it into the statement"
            );
        }
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn the_window_a_question_names_is_the_window_it_is_read_over() {
        let mut query = validated(QueryShape::SubjectTotal, Grain::Total, &[]);
        query.from = NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date");
        query.to = NaiveDate::from_ymd_opt(2026, 3, 31).expect("valid date");

        let compiled = compiled(&query, people_scope());

        for bound in ["2026-03-01", "2026-03-31"] {
            assert!(
                compiled
                    .params
                    .contains(&QueryParam::Text(bound.to_owned())),
                "the window binds {bound}"
            );
        }
    }
}
