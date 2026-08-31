//! What each question compiles to, decided before anything is read: which
//! source identities its people are known by, and which groups a capped split
//! keeps. Both are shared across the questions of one request, and both arrive
//! resolved at the compiler, which looks nothing up.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use uuid::Uuid;

use crate::domain::compiler::request::{
    Bucket, CombinedSplitView, DimensionFilter, EntityScope, GroupLimit, GroupRankingQuery,
    MetricQuery, RankedGroup, ResolvedPerson, SubjectSeriesView, SubjectSplitView, ViewKind,
};
use crate::domain::compiler::sql::CompiledMeasureQuery;
use crate::domain::identity_binding::{IdentitySet, resolve_all_identities, resolve_identities};

use super::super::catalog::MetricCatalog;
use super::super::error::QueryError;
use super::super::question::{query_row_limit, row_limit};
use super::dto::Grain;
use super::group_cap::ranked_groups;
use super::validation::{
    ComparedWindow, QueryShape, ValidatedBatch, ValidatedQuery, ValidatedSplit, ValidatedSubjects,
};

/// The statements one question runs: its own, and the compared window's when
/// it asked for one.
#[derive(Debug, PartialEq)]
pub(super) struct PlannedQuery {
    pub current: CompiledMeasureQuery,
    pub compared: Option<CompiledMeasureQuery>,
}

/// The window one compiled read covers.
#[derive(Debug, Clone, Copy)]
struct Window {
    from: NaiveDate,
    to: NaiveDate,
}

impl From<ComparedWindow> for Window {
    fn from(window: ComparedWindow) -> Self {
        Self {
            from: window.from,
            to: window.to,
        }
    }
}

/// Everything a ranking read's answer depends on, beyond the request's own
/// window. Two questions that agree on all of it share one read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RankingKey {
    rank_metric_key: String,
    dimensions: Vec<String>,
    subjects: Vec<String>,
    // INVARIANT: a narrowing changes which groups rank where, so two
    // differently narrowed questions never share one ranking.
    filters: Vec<DimensionFilter>,
    top: u32,
}

/// The statements one request runs, in the order its questions were asked.
pub(super) async fn plan(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    batch: &ValidatedBatch,
) -> Result<Vec<PlannedQuery>, QueryError> {
    let identities = identities(clickhouse, tenant_id, batch).await?;
    let tenant = tenant_pool(clickhouse, tenant_id, batch).await?;

    let mut rankings: BTreeMap<RankingKey, Vec<RankedGroup>> = BTreeMap::new();
    let mut compiled = Vec::with_capacity(batch.queries.len());
    for query in &batch.queries {
        let scope = entity_scope(&query.subjects, &identities, &tenant);
        let limit = cap(catalog, clickhouse, tenant_id, &mut rankings, query, &scope).await?;
        let window = Window {
            from: query.from,
            to: query.to,
        };

        // INVARIANT: both reads keep the groups the current window ranked, so
        // a series compares against the same group it reports.
        let current = compile(catalog, tenant_id, query, &scope, limit.clone(), window)?;
        let compared = query
            .compare
            .map(|compared| {
                compile(
                    catalog,
                    tenant_id,
                    query,
                    &scope,
                    limit.clone(),
                    compared.into(),
                )
            })
            .transpose()?;
        compiled.push(PlannedQuery { current, compared });
    }
    Ok(compiled)
}

fn compile(
    catalog: &MetricCatalog,
    tenant_id: Uuid,
    query: &ValidatedQuery,
    scope: &EntityScope,
    limit: Option<GroupLimit>,
    window: Window,
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
            entity_scope: scope.clone(),
            from: window.from,
            to: window.to,
            bucket: bucket(query.grain),
            dimension_filters: query.filters.clone(),
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

/// Everyone the tenant's identity mapping knows, read once for the whole
/// request and only when a question asks about the tenant.
///
/// INVARIANT: no dataset records a row keyed by the tenant, so a tenant-wide
/// value is its people's rows folded together. Reading them through the
/// mapping is what makes the fold count people rather than source identities,
/// and what leaves out the identities the mapping resolves nobody for.
async fn tenant_pool(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    batch: &ValidatedBatch,
) -> Result<Vec<ResolvedPerson>, QueryError> {
    if !batch.asks_about_the_tenant() {
        return Ok(Vec::new());
    }

    let identities = resolve_all_identities(clickhouse, tenant_id, query_row_limit())
        .await
        .map_err(|error| {
            tracing::error!(%error, "a tenant-wide question could not enumerate the people it folds");
            QueryError::SubjectsUnresolved
        })?;
    if identities.len() > row_limit() {
        return Err(QueryError::PopulationTooLarge { limit: row_limit() });
    }

    Ok(identities
        .into_iter()
        .map(|(person_id, known_by)| ResolvedPerson {
            person_ref: person_id.to_string(),
            identities: known_by.values(),
        })
        .collect())
}

/// INVARIANT: a person is carried with their own identities rather than merged
/// into one list, because the answer is keyed per person. A tenant-wide
/// question carries the whole tenant's people for the same reason: what it
/// reports as having contributed is people, not the identities they are
/// recorded under.
fn entity_scope(
    subjects: &ValidatedSubjects,
    identities: &BTreeMap<Uuid, IdentitySet>,
    tenant: &[ResolvedPerson],
) -> EntityScope {
    match subjects {
        ValidatedSubjects::Tenant => EntityScope::People(tenant.to_vec()),
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
        filters: query.filters.clone(),
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
        dimension_filters: query.filters.clone(),
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
    use super::super::super::catalog::product_metric_catalog;
    use super::super::super::fixtures::{SHIPPED_METRIC, offline_clickhouse};
    use super::super::fixtures::validated;
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

    /// The tenant's mapping as one pooled person, so a tenant-wide read has
    /// somebody to fold.
    fn tenant_people() -> Vec<ResolvedPerson> {
        vec![ResolvedPerson {
            person_ref: person().to_string(),
            identities: vec!["dev@example.com".to_owned()],
        }]
    }

    fn window(query: &ValidatedQuery) -> Window {
        Window {
            from: query.from,
            to: query.to,
        }
    }

    fn compiled(query: &ValidatedQuery, scope: &EntityScope) -> CompiledMeasureQuery {
        compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            query,
            scope,
            None,
            window(query),
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
            let sql = compiled(&query, &people_scope()).sql;

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

            let sql = compiled(&query, &people_scope()).sql;

            assert!(sql.contains(marker), "{grain:?}: {sql}");
        }
    }

    #[test]
    fn a_question_about_people_reaches_its_rows_through_the_pool_that_keys_them() {
        let query = validated(QueryShape::SubjectTotal, Grain::Total, &[]);

        let compiled = compiled(&query, &people_scope());

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
    fn a_tenant_question_folds_the_tenants_people_and_counts_them_rather_than_their_identities() {
        let query = validated(QueryShape::CombinedSplit, Grain::Total, &["repository"]);
        let scope = entity_scope(
            &ValidatedSubjects::Tenant,
            &BTreeMap::new(),
            &tenant_people(),
        );

        let compiled = compiled(&query, &scope);

        assert!(
            compiled.sql.contains("INNER JOIN pool ON pool.identity = "),
            "{}",
            compiled.sql
        );
        assert!(
            compiled
                .sql
                .contains("uniqExact(pool.person_ref) AS contributing_entity_count"),
            "{}",
            compiled.sql
        );
        assert!(
            compiled
                .params
                .contains(&QueryParam::Text("dev@example.com".to_owned())),
            "every mapped identity in the tenant is bound"
        );
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn a_tenant_question_reaches_only_the_people_the_mapping_resolves() {
        let scope = entity_scope(
            &ValidatedSubjects::Tenant,
            &BTreeMap::new(),
            &tenant_people(),
        );

        assert_eq!(scope, EntityScope::People(tenant_people()));
    }

    #[tokio::test]
    async fn a_tenant_question_whose_mapping_cannot_be_enumerated_refuses_the_request() {
        let batch = ValidatedBatch {
            queries: vec![ValidatedQuery {
                subjects: ValidatedSubjects::Tenant,
                ..validated(QueryShape::CombinedSplit, Grain::Total, &["repository"])
            }],
        };

        let error = tenant_pool(&offline_clickhouse(), tenant(), &batch)
            .await
            .expect_err("a closed port cannot answer");

        assert!(matches!(error, QueryError::SubjectsUnresolved));
    }

    #[tokio::test]
    async fn a_request_about_people_alone_never_enumerates_the_tenants_mapping() {
        let batch = ValidatedBatch {
            queries: vec![validated(QueryShape::SubjectTotal, Grain::Total, &[])],
        };

        let pooled = tenant_pool(&offline_clickhouse(), tenant(), &batch)
            .await
            .expect("no question asks about the tenant");

        assert!(pooled.is_empty());
    }

    /// Validation refuses an undeclared dimension before anything is planned;
    /// this pins the compiler's own refusal, which is the backstop behind it.
    #[test]
    fn a_dimension_the_definitions_do_not_declare_does_not_compile() {
        let query = validated(QueryShape::SubjectSplit, Grain::Total, &["not_a_dimension"]);

        let error = compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &query,
            &people_scope(),
            None,
            window(&query),
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

        let scope = entity_scope(
            &ValidatedSubjects::Persons(vec![alice, bob]),
            &resolved,
            &[],
        );

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

        let scope = entity_scope(&ValidatedSubjects::Persons(vec![person()]), &resolved, &[]);

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
            &[],
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
                filters: Vec::new(),
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

    #[tokio::test]
    async fn a_differently_narrowed_question_does_not_reuse_another_questions_ranking() {
        let mut rankings = BTreeMap::from([(
            RankingKey {
                rank_metric_key: SHIPPED_METRIC.to_owned(),
                dimensions: vec!["repository".to_owned()],
                subjects: vec![person().to_string()],
                filters: Vec::new(),
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
        let mut query = validated(QueryShape::CombinedSplit, Grain::Total, &["repository"]);
        query.split = Some(ValidatedSplit {
            dimensions: vec!["repository".to_owned()],
            limit: Some(super::super::validation::ValidatedSplitLimit {
                top: 5,
                rank_by: SHIPPED_METRIC.to_owned(),
                remainder: true,
            }),
        });
        query.filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec!["example/app".to_owned()],
        }];

        let outcome = cap(
            product_metric_catalog().expect("loads"),
            &offline_clickhouse(),
            tenant(),
            &mut rankings,
            &query,
            &people_scope(),
        )
        .await;

        assert!(
            matches!(outcome, Err(QueryError::SplitUnranked)),
            "the narrowed question ranks its own groups rather than reusing the unnarrowed ones"
        );
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
            &EntityScope::Tenant,
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
            window(&query),
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

    /// A ratio metric, so both halves are read in one scan and a narrowing
    /// that reached only one of them would be visible.
    const COMPOSED_METRIC: &str = "git.merge_rate";

    #[test]
    fn a_filter_narrows_the_scan_every_input_of_the_metric_is_read_from() {
        let mut query = validated(QueryShape::SubjectTotal, Grain::Total, &[]);
        query.metric_key = COMPOSED_METRIC.to_owned();
        query.filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec!["example/app".to_owned()],
        }];

        let compiled = compiled(&query, &people_scope());

        assert_eq!(
            compiled.sql.matches("repository IN (?)").count(),
            1,
            "one scan carries the narrowing both halves fold over: {}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("IfOrNull(") && compiled.sql.contains("nullIf("),
            "the read folds two inputs: {}",
            compiled.sql
        );
        assert!(
            compiled
                .params
                .contains(&QueryParam::Text("example/app".to_owned())),
            "the filter binds its value rather than writing it into the statement"
        );
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn a_compared_question_compiles_the_same_read_over_the_shifted_window() {
        let mut query = validated(QueryShape::SubjectTotal, Grain::Total, &[]);
        query.compare = Some(ComparedWindow {
            from: NaiveDate::from_ymd_opt(2025, 12, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2025, 12, 31).expect("valid date"),
        });

        let current = compiled(&query, &people_scope());
        let compared = compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &query,
            &people_scope(),
            None,
            query.compare.expect("the question compares").into(),
        )
        .expect("the compared window compiles");

        assert_eq!(
            compared.sql, current.sql,
            "only the bound window differs between the two reads"
        );
        for bound in ["2025-12-01", "2025-12-31"] {
            assert!(
                compared
                    .params
                    .contains(&QueryParam::Text(bound.to_owned())),
                "the compared read binds {bound}"
            );
        }
        for bound in ["2026-01-01", "2026-01-31"] {
            assert!(
                !compared
                    .params
                    .contains(&QueryParam::Text(bound.to_owned())),
                "the compared read leaves the current window behind"
            );
        }
    }

    #[test]
    fn the_window_a_question_names_is_the_window_it_is_read_over() {
        let mut query = validated(QueryShape::SubjectTotal, Grain::Total, &[]);
        query.from = NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date");
        query.to = NaiveDate::from_ymd_opt(2026, 3, 31).expect("valid date");

        let compiled = compiled(&query, &people_scope());

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
