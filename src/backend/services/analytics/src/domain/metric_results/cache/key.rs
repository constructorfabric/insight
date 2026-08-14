use std::collections::BTreeSet;

use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::metric_definitions::{CohortSource, MetricDefinition, ObservationSource};

use super::super::compiler::{
    CompiledQuery, compile_breakdown_query, compile_histogram_query, compile_peer_batch_query,
    compile_period_batch_query, compile_timeseries_query,
};
use super::super::validation::{
    ValidatedDimensionFilter, ValidatedEntitySelection, ValidatedMetricRequest,
    ValidatedMetricResultsRequest, ValidatedMetricView,
};
use super::super::view::MetricResultViewKind;
use super::epoch::{RelationEpochs, RelationRef};

/// Namespaced apart from the session keys sharing this Redis. The hash tag is
/// per tenant, not global: a request only ever spans one tenant, so this keeps
/// its keys in one cluster slot for MGET while still spreading tenants across
/// shards instead of pointing every metric read at a single one.
const KEY_NAMESPACE: &str = "amrc";
const KEY_VERSION: u32 = 1;
/// Bump to strand every existing fragment after a shape or semantics change
/// that the compiled probe does not already capture.
const KEY_SCHEMA: u32 = 1;
/// A request asking for more keys than this skips the cache rather than issue
/// an unbounded MGET.
const MAX_KEYS_PER_REQUEST: usize = 10_000;
const PROBE_ENTITY: Uuid = Uuid::nil();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey(String);

impl CacheKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The row shape a per-entity entry restores to. Carried on the plan so the
/// assembler cannot re-derive it from the view and disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerEntityKind {
    Period,
    Peer,
}

/// How one requested view can be served from cache.
#[derive(Debug)]
pub enum ViewKeyPlan {
    /// Nothing to look up: custom observation SQL has no relation epoch to pin,
    /// and a group-limited timeseries cannot be keyed before its ranking runs.
    Uncacheable,
    /// One entry for the whole view, keyed by the full requested entity set.
    Whole(CacheKey),
    /// One entry per requested entity, in requested order. Only for views whose
    /// per-entity result does not depend on which other entities were asked
    /// for.
    PerEntity {
        kind: PerEntityKind,
        keys: Vec<CacheKey>,
    },
}

/// The shape of a request no key could be derived for — a disabled cache or an
/// epoch lookup that failed. Costs no probe compiles.
pub fn uncacheable_keys(req: &ValidatedMetricResultsRequest) -> Vec<Vec<ViewKeyPlan>> {
    req.metrics
        .iter()
        .map(|metric| {
            metric
                .views
                .iter()
                .map(|_| ViewKeyPlan::Uncacheable)
                .collect()
        })
        .collect()
}

/// Cache keys for every requested view, indexed as `[metric][view]`.
pub fn derive_view_keys(
    req: &ValidatedMetricResultsRequest,
    epochs: &RelationEpochs,
) -> Vec<Vec<ViewKeyPlan>> {
    let key_count = upper_bound_key_count(req);
    if key_count > MAX_KEYS_PER_REQUEST {
        tracing::debug!(key_count, "metric-results cache skipped: too many keys");
        return uncacheable_keys(req);
    }

    // INVARIANT: whole-view keys carry the requested entity ORDER, because a
    // whole-view hit replays a stored DTO verbatim and some builders render one
    // row per entity in the order asked for. Sorting here would let a request
    // inherit another request's ordering.
    let full_set = probe_request(req, requested_entity_ids(&req.entity));
    let single = probe_request(req, vec![PROBE_ENTITY]);

    req.metrics
        .iter()
        .map(|metric| {
            let filters = canonical_filters(&metric.filters);
            let resolved = metric_epochs(&metric.def, epochs);
            metric
                .views
                .iter()
                .map(|view| match &resolved {
                    None => ViewKeyPlan::Uncacheable,
                    Some(metric_epochs) => view_key_plan(
                        req,
                        metric,
                        view,
                        &filters,
                        metric_epochs,
                        epochs,
                        &full_set,
                        &single,
                    ),
                })
                .collect()
        })
        .collect()
}

/// Counted from the request shape alone, before any probe is compiled or
/// hashed, so an oversized request is rejected without paying for the keys it
/// will never use.
fn upper_bound_key_count(req: &ValidatedMetricResultsRequest) -> usize {
    let entity_count = req.entity.len();
    req.metrics
        .iter()
        .flat_map(|metric| metric.views.iter())
        .map(|view| match view {
            ValidatedMetricView::Period | ValidatedMetricView::Peer { .. } => entity_count,
            ValidatedMetricView::Timeseries { .. }
            | ValidatedMetricView::Breakdown { .. }
            | ValidatedMetricView::Histogram => 1,
        })
        .sum()
}

#[expect(
    clippy::too_many_arguments,
    reason = "one pure fan-out over request parts"
)]
fn view_key_plan(
    req: &ValidatedMetricResultsRequest,
    metric: &ValidatedMetricRequest,
    view: &ValidatedMetricView,
    filters: &[ValidatedDimensionFilter],
    epochs: &[(String, String)],
    all_epochs: &RelationEpochs,
    full_set: &ValidatedMetricResultsRequest,
    single: &ValidatedMetricResultsRequest,
) -> ViewKeyPlan {
    let def = &metric.def;
    match view {
        ValidatedMetricView::Period => {
            let probe = compile_period_batch_query(&[def], single, filters);
            per_entity_keys(req, PerEntityKind::Period, &probe, epochs)
        }

        ValidatedMetricView::Peer { cohort_key } => {
            let Some(epochs) = with_cohort_epoch(all_epochs, epochs) else {
                return ViewKeyPlan::Uncacheable;
            };
            let probe = compile_peer_batch_query(&[def], single, cohort_key, filters);
            per_entity_keys(req, PerEntityKind::Peer, &probe, &epochs)
        }

        // The compiled SQL embeds the resolved top-N groups, which are only
        // known after the ranking query has run over the whole entity set.
        ValidatedMetricView::Timeseries {
            group_limit: Some(_),
            ..
        } => ViewKeyPlan::Uncacheable,

        ValidatedMetricView::Timeseries {
            bucket,
            dimensions,
            group_limit: None,
        } => {
            let probe = compile_timeseries_query(def, full_set, *bucket, dimensions, filters, None);
            whole_key(MetricResultViewKind::Timeseries, req, &probe, epochs)
        }

        ValidatedMetricView::Breakdown { dimensions } => {
            let probe = compile_breakdown_query(def, full_set, dimensions, filters);
            whole_key(MetricResultViewKind::Breakdown, req, &probe, epochs)
        }

        ValidatedMetricView::Histogram => {
            let probe = compile_histogram_query(def, full_set, filters);
            whole_key(MetricResultViewKind::Histogram, req, &probe, epochs)
        }
    }
}

fn whole_key(
    kind: MetricResultViewKind,
    req: &ValidatedMetricResultsRequest,
    probe: &CompiledQuery,
    epochs: &[(String, String)],
) -> ViewKeyPlan {
    let Some(digest) = view_digest(kind, req.tenant_id, probe, epochs) else {
        return ViewKeyPlan::Uncacheable;
    };
    ViewKeyPlan::Whole(cache_key(req.tenant_id, &digest, None))
}

/// The probe digest is per (metric, view); only the entity id varies, so it is
/// hashed once and mixed in per entity rather than re-serializing the SQL for
/// every requested id.
fn per_entity_keys(
    req: &ValidatedMetricResultsRequest,
    kind: PerEntityKind,
    probe: &CompiledQuery,
    epochs: &[(String, String)],
) -> ViewKeyPlan {
    let view_kind = match kind {
        PerEntityKind::Period => MetricResultViewKind::Period,
        PerEntityKind::Peer => MetricResultViewKind::Peer,
    };
    let Some(digest) = view_digest(view_kind, req.tenant_id, probe, epochs) else {
        return ViewKeyPlan::Uncacheable;
    };

    let keys = req
        .entity
        .entity_ids()
        .iter()
        .map(|entity_id| cache_key(req.tenant_id, &digest, Some(entity_id.as_str())))
        .collect();

    ViewKeyPlan::PerEntity { kind, keys }
}

/// The compiled probe query IS the fingerprint of everything that shapes the
/// result — measure keys, transform, dates, tenant predicate, filters,
/// dimensions — so a definition edit or a compiler change lands on a new key
/// without anything here enumerating what changed.
#[derive(Serialize)]
struct ViewKeyMaterial<'a> {
    schema: u32,
    tenant_id: Uuid,
    view_kind: MetricResultViewKind,
    sql: &'a str,
    params: &'a [String],
    epochs: &'a [(String, String)],
}

fn view_digest(
    kind: MetricResultViewKind,
    tenant_id: Uuid,
    probe: &CompiledQuery,
    epochs: &[(String, String)],
) -> Option<[u8; 32]> {
    let material = ViewKeyMaterial {
        schema: KEY_SCHEMA,
        tenant_id,
        view_kind: kind,
        sql: &probe.sql,
        params: &probe.params,
        epochs,
    };
    let bytes = serde_json::to_vec(&material).ok()?;

    Some(Sha256::digest(bytes).into())
}

fn cache_key(tenant_id: Uuid, digest: &[u8; 32], entity_id: Option<&str>) -> CacheKey {
    let mut hasher = Sha256::new();
    hasher.update(digest);
    if let Some(entity_id) = entity_id {
        hasher.update(b"\0");
        hasher.update(entity_id.as_bytes());
    }

    CacheKey(format!(
        "{{{KEY_NAMESPACE}:{tenant_id}}}:v{KEY_VERSION}:{}",
        hex::encode(hasher.finalize())
    ))
}

/// `None` when any input reads custom SQL or its relation has no epoch — either
/// way there is nothing to invalidate against.
fn metric_epochs(def: &MetricDefinition, epochs: &RelationEpochs) -> Option<Vec<(String, String)>> {
    let mut resolved = BTreeSet::new();
    for input in def.inputs() {
        let ObservationSource::Managed(relation) = &input.observation else {
            return None;
        };
        let (database, table) = relation.table_ref();
        let relation = RelationRef {
            database,
            table: table.to_owned(),
        };
        let epoch = epochs.get(&relation)?;
        resolved.insert((format!("{database}.{table}"), epoch.to_owned()));
    }
    Some(resolved.into_iter().collect())
}

/// Peer keys additionally pin the cohort relation: a cohort rebuild changes
/// every pool without touching an observation table.
fn with_cohort_epoch(
    epochs: &RelationEpochs,
    metric_epochs: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    let (database, table) = CohortSource::MetricEntityCohortsCurrent.table_ref();
    let relation = RelationRef {
        database,
        table: table.to_owned(),
    };
    let epoch = epochs.get(&relation)?;

    let mut resolved: BTreeSet<(String, String)> = metric_epochs.iter().cloned().collect();
    resolved.insert((format!("{database}.{table}"), epoch.to_owned()));
    Some(resolved.into_iter().collect())
}

fn probe_request(
    req: &ValidatedMetricResultsRequest,
    ids: Vec<Uuid>,
) -> ValidatedMetricResultsRequest {
    ValidatedMetricResultsRequest {
        tenant_id: req.tenant_id,
        entity: match &req.entity {
            ValidatedEntitySelection::Person { .. } => ValidatedEntitySelection::Person { ids },
        },
        from: req.from,
        to: req.to,
        metrics: Vec::new(),
        enforce_tenant_scope: req.enforce_tenant_scope,
    }
}

fn requested_entity_ids(entity: &ValidatedEntitySelection) -> Vec<Uuid> {
    match entity {
        ValidatedEntitySelection::Person { ids } => ids.clone(),
    }
}

fn canonical_filters(filters: &[ValidatedDimensionFilter]) -> Vec<ValidatedDimensionFilter> {
    let mut filters: Vec<ValidatedDimensionFilter> = filters
        .iter()
        .map(|filter| {
            let mut values = filter.values.clone();
            values.sort_unstable();
            values.dedup();
            ValidatedDimensionFilter {
                dimension: filter.dimension.clone(),
                values,
            }
        })
        .collect();
    filters.sort();
    filters
}

#[cfg(test)]
mod tests {
    use super::super::epoch::{RelationEpochs, relation_epochs_for_test, required_relations};
    use super::super::test_support::{
        TENANT, custom_sql_metric, entity, filtered_metric_request, metric_request, request,
        split_source_ratio_metric, sum_metric,
    };
    use super::*;
    use crate::domain::metric_definitions::definition::ValueTransform;
    use crate::domain::metric_results::validation::ValidatedGroupLimit;
    use crate::domain::metric_results::view::Bucket;

    fn epochs_for(req: &ValidatedMetricResultsRequest) -> RelationEpochs {
        relation_epochs_for_test(&required_relations(&req.metrics), "epoch-1")
    }

    fn keys_of(req: &ValidatedMetricResultsRequest) -> Vec<Vec<ViewKeyPlan>> {
        derive_view_keys(req, &epochs_for(req))
    }

    fn whole(plan: &ViewKeyPlan) -> &str {
        match plan {
            ViewKeyPlan::Whole(key) => key.as_str(),
            other => panic!("expected a whole-view key, got {other:?}"),
        }
    }

    fn per_entity(plan: &ViewKeyPlan) -> Vec<&str> {
        match plan {
            ViewKeyPlan::PerEntity { keys, .. } => keys.iter().map(CacheKey::as_str).collect(),
            other => panic!("expected per-entity keys, got {other:?}"),
        }
    }

    fn timeseries(group_limit: Option<ValidatedGroupLimit>) -> ValidatedMetricView {
        ValidatedMetricView::Timeseries {
            bucket: Bucket::Day,
            dimensions: Vec::new(),
            group_limit,
        }
    }

    #[test]
    fn identical_requests_derive_identical_keys() {
        let views = vec![ValidatedMetricView::Period, timeseries(None)];
        let ids = vec![entity(1), entity(2)];

        let left = keys_of(&request(
            ids.clone(),
            vec![metric_request(sum_metric(), views.clone())],
        ));
        let right = keys_of(&request(ids, vec![metric_request(sum_metric(), views)]));

        assert_eq!(per_entity(&left[0][0]), per_entity(&right[0][0]));
        assert_eq!(whole(&left[0][1]), whole(&right[0][1]));
    }

    /// A whole-view hit replays a stored DTO verbatim, and the histogram builder
    /// emits one value per entity in the order asked for — so a permuted request
    /// must not be able to collect the earlier request's ordering.
    #[test]
    fn entity_order_changes_whole_view_keys() {
        let forward = keys_of(&request(
            vec![entity(1), entity(2)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Histogram],
            )],
        ));
        let reversed = keys_of(&request(
            vec![entity(2), entity(1)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Histogram],
            )],
        ));

        assert_ne!(whole(&forward[0][0]), whole(&reversed[0][0]));
    }

    #[test]
    fn per_entity_keys_ignore_the_rest_of_the_entity_set() {
        let alone = keys_of(&request(
            vec![entity(1)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period],
            )],
        ));
        let crowded = keys_of(&request(
            vec![entity(9), entity(1), entity(4)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period],
            )],
        ));

        assert_eq!(per_entity(&alone[0][0])[0], per_entity(&crowded[0][0])[1]);
    }

    #[test]
    fn per_entity_keys_follow_requested_order_and_differ_per_entity() {
        let keys = keys_of(&request(
            vec![entity(1), entity(2)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period],
            )],
        ));

        let keys = per_entity(&keys[0][0]);
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1]);
    }

    #[test]
    fn filter_value_order_does_not_change_keys() {
        let filter = |values: Vec<&str>| {
            vec![ValidatedDimensionFilter {
                dimension: "tool".to_owned(),
                values: values.into_iter().map(str::to_owned).collect(),
            }]
        };
        let of = |values: Vec<&str>| {
            let req = request(
                vec![entity(1)],
                vec![filtered_metric_request(
                    sum_metric(),
                    filter(values),
                    vec![timeseries(None)],
                )],
            );
            whole(&keys_of(&req)[0][0]).to_owned()
        };

        assert_eq!(of(vec!["a", "b"]), of(vec!["b", "a"]));
    }

    #[test]
    fn a_different_filter_changes_the_key() {
        let of = |values: Vec<&str>| {
            let filters = vec![ValidatedDimensionFilter {
                dimension: "tool".to_owned(),
                values: values.into_iter().map(str::to_owned).collect(),
            }];
            let req = request(
                vec![entity(1)],
                vec![filtered_metric_request(
                    sum_metric(),
                    filters,
                    vec![timeseries(None)],
                )],
            );
            whole(&keys_of(&req)[0][0]).to_owned()
        };

        assert_ne!(of(vec!["a"]), of(vec!["a", "b"]));
    }

    #[test]
    fn request_shape_changes_that_change_results_change_keys() {
        let baseline = request(
            vec![entity(1)],
            vec![metric_request(sum_metric(), vec![timeseries(None)])],
        );
        let key = whole(&keys_of(&baseline)[0][0]).to_owned();

        let mut other_tenant = baseline.clone();
        other_tenant.tenant_id = Uuid::from_u128(0xdead);
        assert_ne!(key, whole(&keys_of(&other_tenant)[0][0]), "tenant");

        let mut other_period = baseline.clone();
        other_period.to = super::super::test_support::date("2026-02-28");
        assert_ne!(key, whole(&keys_of(&other_period)[0][0]), "period");

        let mut unscoped = baseline.clone();
        unscoped.enforce_tenant_scope = false;
        assert_ne!(key, whole(&keys_of(&unscoped)[0][0]), "tenant scope");

        let mut other_bucket = baseline.clone();
        other_bucket.metrics[0].views[0] = ValidatedMetricView::Timeseries {
            bucket: Bucket::Week,
            dimensions: Vec::new(),
            group_limit: None,
        };
        assert_ne!(key, whole(&keys_of(&other_bucket)[0][0]), "bucket");

        let mut dimensioned = baseline.clone();
        dimensioned.metrics[0].views[0] = ValidatedMetricView::Timeseries {
            bucket: Bucket::Day,
            dimensions: vec!["tool".to_owned()],
            group_limit: None,
        };
        assert_ne!(key, whole(&keys_of(&dimensioned)[0][0]), "dimensions");

        let mut transformed = baseline.clone();
        transformed.metrics[0].def.transform = Some(ValueTransform {
            multiplier: Some(2.0),
            offset: None,
            clamp_min: None,
            clamp_max: None,
        });
        assert_ne!(key, whole(&keys_of(&transformed)[0][0]), "transform");
    }

    #[test]
    fn a_rebuilt_relation_changes_the_key() {
        let req = request(
            vec![entity(1)],
            vec![metric_request(sum_metric(), vec![timeseries(None)])],
        );
        let relations = required_relations(&req.metrics);

        let before = derive_view_keys(&req, &relation_epochs_for_test(&relations, "epoch-1"));
        let after = derive_view_keys(&req, &relation_epochs_for_test(&relations, "epoch-2"));

        assert_ne!(whole(&before[0][0]), whole(&after[0][0]));
    }

    #[test]
    fn a_rebuilt_cohort_relation_changes_peer_keys_only() {
        let req = request(
            vec![entity(1)],
            vec![metric_request(
                sum_metric(),
                vec![
                    ValidatedMetricView::Peer {
                        cohort_key: "org_unit".to_owned(),
                    },
                    timeseries(None),
                ],
            )],
        );
        let observations = RelationRef {
            database: "insight",
            table: "ai_metric_observations".to_owned(),
        };
        let cohorts = RelationRef {
            database: "insight",
            table: "metric_entity_cohorts_current".to_owned(),
        };

        let before = derive_view_keys(
            &req,
            &RelationEpochs::from_pairs(vec![
                (observations.clone(), "obs-1".to_owned()),
                (cohorts.clone(), "cohort-1".to_owned()),
            ]),
        );
        let after = derive_view_keys(
            &req,
            &RelationEpochs::from_pairs(vec![
                (observations, "obs-1".to_owned()),
                (cohorts, "cohort-2".to_owned()),
            ]),
        );

        assert_ne!(per_entity(&before[0][0]), per_entity(&after[0][0]), "peer");
        assert_eq!(whole(&before[0][1]), whole(&after[0][1]), "timeseries");
    }

    #[test]
    fn peer_view_without_a_cohort_epoch_is_uncacheable() {
        let req = request(
            vec![entity(1)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Peer {
                    cohort_key: "org_unit".to_owned(),
                }],
            )],
        );
        let observations_only = RelationEpochs::from_pairs(vec![(
            RelationRef {
                database: "insight",
                table: "ai_metric_observations".to_owned(),
            },
            "obs-1".to_owned(),
        )]);

        let keys = derive_view_keys(&req, &observations_only);

        assert!(matches!(keys[0][0], ViewKeyPlan::Uncacheable));
    }

    #[test]
    fn custom_observation_sql_is_never_cached() {
        for def in [custom_sql_metric(), split_source_ratio_metric()] {
            let req = request(
                vec![entity(1)],
                vec![metric_request(
                    def,
                    vec![ValidatedMetricView::Period, timeseries(None)],
                )],
            );

            let keys = keys_of(&req);

            assert!(matches!(keys[0][0], ViewKeyPlan::Uncacheable));
            assert!(matches!(keys[0][1], ViewKeyPlan::Uncacheable));
        }
    }

    #[test]
    fn group_limited_timeseries_is_uncacheable() {
        let req = request(
            vec![entity(1)],
            vec![metric_request(
                sum_metric(),
                vec![timeseries(Some(ValidatedGroupLimit {
                    count: 5,
                    rank_by: Box::new(sum_metric()),
                    include_remainder: true,
                }))],
            )],
        );

        assert!(matches!(keys_of(&req)[0][0], ViewKeyPlan::Uncacheable));
    }

    #[test]
    fn an_oversized_key_set_skips_the_cache_entirely() {
        let ids: Vec<Uuid> = (0..=MAX_KEYS_PER_REQUEST).map(entity).collect();
        let req = request(
            ids,
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period],
            )],
        );

        assert!(matches!(keys_of(&req)[0][0], ViewKeyPlan::Uncacheable));
    }

    /// One slot per tenant: a request stays MGET-able, but metric reads do not
    /// all pile onto the shard a single global tag would select.
    #[test]
    fn keys_are_hash_tagged_per_tenant() {
        let req = request(
            vec![entity(1)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period, timeseries(None)],
            )],
        );
        let expected = format!("{{{KEY_NAMESPACE}:{TENANT}}}:v{KEY_VERSION}:");

        let keys = keys_of(&req);

        assert!(per_entity(&keys[0][0])[0].starts_with(&expected));
        assert!(whole(&keys[0][1]).starts_with(&expected));

        let mut other_tenant = req.clone();
        other_tenant.tenant_id = Uuid::from_u128(0xfeed);
        assert!(
            !whole(&keys_of(&other_tenant)[0][1]).starts_with(&expected),
            "a different tenant must land in a different slot"
        );
    }
}
