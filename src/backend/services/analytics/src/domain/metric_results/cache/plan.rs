use std::collections::{BTreeSet, HashMap};

use toolkit_canonical_errors::CanonicalError;

use super::super::builder::{build_peer_view, build_period_view};
use super::super::compiler::{PeerQueryRow, PeriodQueryRow};
use super::super::dto::MetricResultViewDto;
use super::super::validation::{
    ValidatedEntitySelection, ValidatedMetricRequest, ValidatedMetricResultsRequest,
    ValidatedMetricView,
};
use super::fragment::{self, PeerFragment, PeriodFragment};
use super::key::{CacheKey, PerEntityKind, ViewKeyPlan};

/// Which narrowed request a result came back from. The two carry different
/// entity sets, so a result cannot be remapped without knowing its origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Views needing every requested entity: whole-view misses and everything
    /// uncacheable.
    FullSet,
    /// Per-entity views, over the union of the entities they are missing.
    Subset,
}

/// What the executor produced for one requested view. Period and peer arrive as
/// rows because cached fragments contribute rows to the very same view.
#[derive(Debug)]
pub enum ViewOutcome {
    View(MetricResultViewDto),
    PeriodRows(Vec<PeriodQueryRow>),
    PeerRows(Vec<PeerQueryRow>),
}

#[derive(Debug)]
enum Slot {
    Pending,
    Done {
        view: MetricResultViewDto,
        fresh: bool,
    },
    Period {
        cached: Vec<PeriodQueryRow>,
        fresh: Vec<PeriodQueryRow>,
        absent: BTreeSet<String>,
    },
    Peer {
        cached: Vec<PeerQueryRow>,
        fresh: Vec<PeerQueryRow>,
        absent: BTreeSet<String>,
    },
}

#[derive(Debug)]
pub struct NarrowedRequest {
    pub req: ValidatedMetricResultsRequest,
    origins: Vec<Vec<(usize, usize)>>,
}

impl NarrowedRequest {
    fn origin(&self, metric_index: usize, view_index: usize) -> Option<(usize, usize)> {
        self.origins
            .get(metric_index)
            .and_then(|views| views.get(view_index))
            .copied()
    }
}

#[derive(Debug)]
pub struct CacheOutcome {
    pub views: Vec<Vec<Option<MetricResultViewDto>>>,
    pub writes: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CacheStats {
    pub hits: usize,
    pub misses: usize,
    pub uncacheable: usize,
}

#[derive(Debug)]
pub struct CachePlan {
    slots: Vec<Vec<Slot>>,
    keys: Vec<Vec<ViewKeyPlan>>,
    entity_ids: Vec<String>,
    full_set: Option<NarrowedRequest>,
    subset: Option<NarrowedRequest>,
    stats: CacheStats,
}

impl CachePlan {
    /// Splits the request into what the cache already answered and what still
    /// has to run. `cached` holds one entry per key in `keys`, in the order
    /// [`flat_keys`] produces.
    pub fn build(
        req: &ValidatedMetricResultsRequest,
        keys: Vec<Vec<ViewKeyPlan>>,
        cached: &[Option<Vec<u8>>],
    ) -> Self {
        let entity_ids = req.entity.entity_ids();
        let mut reader = CachedReader::new(cached);
        let mut stats = CacheStats::default();

        let mut slots: Vec<Vec<Slot>> = Vec::with_capacity(req.metrics.len());
        let mut full_set_views: Vec<Vec<usize>> = Vec::with_capacity(req.metrics.len());
        let mut subset_views: Vec<Vec<usize>> = Vec::with_capacity(req.metrics.len());
        let mut subset_ids: BTreeSet<String> = BTreeSet::new();

        for (metric_index, metric) in req.metrics.iter().enumerate() {
            let mut metric_slots = Vec::with_capacity(metric.views.len());
            let mut full_set = Vec::new();
            let mut subset = Vec::new();

            for (view_index, view) in metric.views.iter().enumerate() {
                let plan = &keys[metric_index][view_index];
                let read = read_slot(view, plan, &entity_ids, &mut reader, &mut stats);

                match &read.slot {
                    Slot::Done { .. } => {}
                    Slot::Pending => full_set.push(view_index),
                    Slot::Period { .. } | Slot::Peer { .. } => {
                        if !read.absent.is_empty() {
                            subset.push(view_index);
                            subset_ids.extend(read.absent);
                        }
                    }
                }

                metric_slots.push(read.slot);
            }

            slots.push(metric_slots);
            full_set_views.push(full_set);
            subset_views.push(subset);
        }

        let full_set = narrow(req, &full_set_views, None);
        let subset = narrow(req, &subset_views, Some(&subset_ids));

        Self {
            slots,
            keys,
            entity_ids,
            full_set,
            subset,
            stats,
        }
    }

    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    pub fn narrowed(&self) -> Vec<(Origin, &NarrowedRequest)> {
        [
            (Origin::FullSet, self.full_set.as_ref()),
            (Origin::Subset, self.subset.as_ref()),
        ]
        .into_iter()
        .filter_map(|(origin, narrowed)| narrowed.map(|narrowed| (origin, narrowed)))
        .collect()
    }

    /// A result that cannot be routed is a broken internal invariant, not a
    /// missing value: dropping it would cache an absence no query measured.
    pub fn record(
        &mut self,
        origin: Origin,
        metric_index: usize,
        view_index: usize,
        outcome: ViewOutcome,
    ) -> Result<(), CanonicalError> {
        let narrowed = match origin {
            Origin::FullSet => self.full_set.as_ref(),
            Origin::Subset => self.subset.as_ref(),
        };
        let Some((metric_index, view_index)) =
            narrowed.and_then(|narrowed| narrowed.origin(metric_index, view_index))
        else {
            tracing::error!(?origin, "metric-results cache result has no requested view");
            return Err(unroutable());
        };
        let Some(slot) = self
            .slots
            .get_mut(metric_index)
            .and_then(|views| views.get_mut(view_index))
        else {
            tracing::error!(
                metric_index,
                view_index,
                "metric-results cache slot missing"
            );
            return Err(unroutable());
        };

        match (&mut *slot, outcome) {
            (Slot::Pending, ViewOutcome::View(view)) => {
                *slot = Slot::Done { view, fresh: true };
                Ok(())
            }
            (Slot::Period { fresh, .. }, ViewOutcome::PeriodRows(rows)) => {
                *fresh = rows;
                Ok(())
            }
            (Slot::Peer { fresh, .. }, ViewOutcome::PeerRows(rows)) => {
                *fresh = rows;
                Ok(())
            }
            (slot, outcome) => {
                tracing::error!(
                    ?slot,
                    ?outcome,
                    "metric-results cache slot does not match the executed view"
                );
                Err(unroutable())
            }
        }
    }

    /// Assembles every view through the builders the uncached path uses, so a
    /// partially cached response is identical to a fully computed one.
    pub fn finish(
        self,
        req: &ValidatedMetricResultsRequest,
    ) -> Result<CacheOutcome, CanonicalError> {
        let Self {
            slots,
            keys,
            entity_ids,
            ..
        } = self;

        let mut writes = Vec::new();
        let mut views = Vec::with_capacity(slots.len());

        for (metric_index, metric_slots) in slots.into_iter().enumerate() {
            let mut metric_views = Vec::with_capacity(metric_slots.len());

            for (view_index, slot) in metric_slots.into_iter().enumerate() {
                let view_keys = &keys[metric_index][view_index];
                let view = match slot {
                    Slot::Pending => {
                        return Err(CanonicalError::internal("missing metric view result").create());
                    }

                    Slot::Done { view, fresh } => {
                        if let (true, ViewKeyPlan::Whole(key)) = (fresh, view_keys) {
                            push_write(&mut writes, key, &view);
                        }
                        view
                    }

                    Slot::Period {
                        cached,
                        fresh,
                        absent,
                    } => {
                        let rows = merge_rows(cached, fresh, &absent);
                        push_entity_writes(
                            &mut writes,
                            view_keys,
                            &entity_ids,
                            &absent,
                            &rows,
                            PeriodFragment::from_row,
                        );
                        build_period_view(req, rows)
                    }

                    Slot::Peer {
                        cached,
                        fresh,
                        absent,
                    } => {
                        let rows = merge_rows(cached, fresh, &absent);
                        push_entity_writes(
                            &mut writes,
                            view_keys,
                            &entity_ids,
                            &absent,
                            &rows,
                            PeerFragment::from_row,
                        );
                        build_peer_view(req, rows)
                    }
                };

                metric_views.push(Some(view));
            }

            views.push(metric_views);
        }

        Ok(CacheOutcome { views, writes })
    }
}

/// Only entities this view actually recomputed are written back. The rest came
/// out of the cache, so rewriting them would extend their TTL for free — and for
/// an entity the view never queried it would store an absence nothing measured.
fn push_entity_writes<Row: EntityRow, Frag: serde::Serialize>(
    writes: &mut Vec<(String, Vec<u8>)>,
    keys: &ViewKeyPlan,
    entity_ids: &[String],
    absent: &BTreeSet<String>,
    rows: &[Row],
    fragment_of: impl Fn(Option<&Row>) -> Frag,
) {
    let ViewKeyPlan::PerEntity { keys, .. } = keys else {
        return;
    };
    if absent.is_empty() {
        return;
    }

    let by_entity: HashMap<&str, &Row> = rows.iter().map(|row| (row.entity_id(), row)).collect();
    for (entity_id, key) in entity_ids.iter().zip(keys) {
        if !absent.contains(entity_id) {
            continue;
        }
        let row = by_entity.get(entity_id.as_str()).copied();
        if let Some(bytes) = fragment::encode(&fragment_of(row)) {
            writes.push((key.as_str().to_owned(), bytes));
        }
    }
}

fn unroutable() -> CanonicalError {
    CanonicalError::internal("metric view result could not be assembled").create()
}

/// Flattens the key grid in the order [`CachePlan::build`] consumes it.
pub fn flat_keys(keys: &[Vec<ViewKeyPlan>]) -> Vec<String> {
    keys.iter()
        .flatten()
        .flat_map(|plan| match plan {
            ViewKeyPlan::Uncacheable => Vec::new(),
            ViewKeyPlan::Whole(key) => vec![key.as_str().to_owned()],
            ViewKeyPlan::PerEntity { keys, .. } => {
                keys.iter().map(|key| key.as_str().to_owned()).collect()
            }
        })
        .collect()
}

trait EntityRow {
    fn entity_id(&self) -> &str;
}

impl EntityRow for PeriodQueryRow {
    fn entity_id(&self) -> &str {
        &self.entity_id
    }
}

impl EntityRow for PeerQueryRow {
    fn entity_id(&self) -> &str {
        &self.entity_id
    }
}

struct CachedReader<'a> {
    cached: &'a [Option<Vec<u8>>],
    next: usize,
}

impl<'a> CachedReader<'a> {
    fn new(cached: &'a [Option<Vec<u8>>]) -> Self {
        Self { cached, next: 0 }
    }

    fn take(&mut self) -> Option<&'a [u8]> {
        let entry = self.cached.get(self.next)?;
        self.next += 1;
        entry.as_deref()
    }
}

struct SlotRead {
    slot: Slot,
    absent: BTreeSet<String>,
}

/// INVARIANT: consumes exactly as many cached entries as `flat_keys` emitted for
/// this plan — a short read here would hand the next view someone else's bytes.
fn read_slot(
    view: &ValidatedMetricView,
    plan: &ViewKeyPlan,
    entity_ids: &[String],
    reader: &mut CachedReader<'_>,
    stats: &mut CacheStats,
) -> SlotRead {
    match plan {
        ViewKeyPlan::Uncacheable => {
            stats.uncacheable += 1;
            let absent: BTreeSet<String> = match view {
                ValidatedMetricView::Period | ValidatedMetricView::Peer { .. } => {
                    entity_ids.iter().cloned().collect()
                }
                ValidatedMetricView::Timeseries { .. }
                | ValidatedMetricView::Breakdown { .. }
                | ValidatedMetricView::Histogram => BTreeSet::new(),
            };
            SlotRead {
                slot: empty_slot(view, absent.clone()),
                absent,
            }
        }

        ViewKeyPlan::Whole(_) => {
            let cached = reader.take().and_then(fragment::decode);
            let slot = if let Some(view) = cached {
                stats.hits += 1;
                Slot::Done { view, fresh: false }
            } else {
                stats.misses += 1;
                Slot::Pending
            };

            SlotRead {
                slot,
                absent: BTreeSet::new(),
            }
        }

        ViewKeyPlan::PerEntity { kind, keys } => {
            let mut absent = BTreeSet::new();
            let mut period = Vec::new();
            let mut peer = Vec::new();

            for index in 0..keys.len() {
                let bytes = reader.take();
                let Some(entity_id) = entity_ids.get(index) else {
                    continue;
                };

                match kind {
                    PerEntityKind::Period => {
                        if let Some(found) = bytes.and_then(fragment::decode::<PeriodFragment>) {
                            stats.hits += 1;
                            period.push(found.into_row(entity_id.clone()));
                        } else {
                            stats.misses += 1;
                            absent.insert(entity_id.clone());
                        }
                    }

                    PerEntityKind::Peer => {
                        if let Some(found) = bytes.and_then(fragment::decode::<PeerFragment>) {
                            stats.hits += 1;
                            peer.extend(found.into_row(entity_id.clone()));
                        } else {
                            stats.misses += 1;
                            absent.insert(entity_id.clone());
                        }
                    }
                }
            }

            let slot = match kind {
                PerEntityKind::Period => Slot::Period {
                    cached: period,
                    fresh: Vec::new(),
                    absent: absent.clone(),
                },
                PerEntityKind::Peer => Slot::Peer {
                    cached: peer,
                    fresh: Vec::new(),
                    absent: absent.clone(),
                },
            };

            SlotRead { slot, absent }
        }
    }
}

fn empty_slot(view: &ValidatedMetricView, absent: BTreeSet<String>) -> Slot {
    match view {
        ValidatedMetricView::Period => Slot::Period {
            cached: Vec::new(),
            fresh: Vec::new(),
            absent,
        },
        ValidatedMetricView::Peer { .. } => Slot::Peer {
            cached: Vec::new(),
            fresh: Vec::new(),
            absent,
        },
        ValidatedMetricView::Timeseries { .. }
        | ValidatedMetricView::Breakdown { .. }
        | ValidatedMetricView::Histogram => Slot::Pending,
    }
}

/// Each side contributes exactly the entities it owns: cached rows for the
/// entities this view had, fresh rows for the ones it was missing. The subset
/// query covers the union across views, so a fresh row for an entity this view
/// already had is another view's work and is dropped rather than churning a TTL.
fn merge_rows<Row: EntityRow>(
    cached: Vec<Row>,
    fresh: Vec<Row>,
    absent: &BTreeSet<String>,
) -> Vec<Row> {
    let mut rows: Vec<Row> = cached
        .into_iter()
        .filter(|row| !absent.contains(row.entity_id()))
        .collect();
    rows.extend(
        fresh
            .into_iter()
            .filter(|row| absent.contains(row.entity_id())),
    );
    rows
}

fn narrow(
    req: &ValidatedMetricResultsRequest,
    wanted: &[Vec<usize>],
    entity_ids: Option<&BTreeSet<String>>,
) -> Option<NarrowedRequest> {
    let entity = match entity_ids {
        None => req.entity.clone(),
        Some(wanted) => {
            let entity = retain_entities(&req.entity, wanted);
            if entity.len() == 0 {
                return None;
            }
            entity
        }
    };

    let mut metrics = Vec::new();
    let mut origins = Vec::new();
    for (metric_index, view_indexes) in wanted.iter().enumerate() {
        if view_indexes.is_empty() {
            continue;
        }

        let metric = &req.metrics[metric_index];
        metrics.push(ValidatedMetricRequest {
            def: metric.def.clone(),
            filters: metric.filters.clone(),
            views: view_indexes
                .iter()
                .map(|view_index| metric.views[*view_index].clone())
                .collect(),
        });
        origins.push(
            view_indexes
                .iter()
                .map(|view_index| (metric_index, *view_index))
                .collect(),
        );
    }

    if metrics.is_empty() {
        return None;
    }

    Some(NarrowedRequest {
        req: ValidatedMetricResultsRequest {
            tenant_id: req.tenant_id,
            entity,
            from: req.from,
            to: req.to,
            metrics,
            enforce_tenant_scope: req.enforce_tenant_scope,
        },
        origins,
    })
}

fn retain_entities(
    entity: &ValidatedEntitySelection,
    wanted: &BTreeSet<String>,
) -> ValidatedEntitySelection {
    match entity {
        ValidatedEntitySelection::Person { ids } => ValidatedEntitySelection::Person {
            ids: ids
                .iter()
                .filter(|id| wanted.contains(&id.to_string()))
                .copied()
                .collect(),
        },
    }
}

fn push_write(writes: &mut Vec<(String, Vec<u8>)>, key: &CacheKey, view: &MetricResultViewDto) {
    if let Some(bytes) = fragment::encode_readable(view) {
        writes.push((key.as_str().to_owned(), bytes));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::epoch::{relation_epochs_for_test, required_relations};
    use super::super::key::{derive_view_keys, uncacheable_keys};
    use super::super::test_support::{
        custom_sql_metric, entity, metric_request, request, sum_metric,
    };
    use super::*;
    use crate::domain::metric_results::batch::{PlannedQuery, plan_queries};
    use crate::domain::metric_results::builder::build_histogram_view;
    use crate::domain::metric_results::dto::{PeerValueDto, PeriodValueDto};
    use crate::domain::metric_results::view::Bucket;

    fn keys(req: &ValidatedMetricResultsRequest) -> Vec<Vec<ViewKeyPlan>> {
        let epochs = relation_epochs_for_test(&required_relations(&req.metrics), "epoch-1");
        derive_view_keys(req, &epochs)
    }

    fn period_row(entity_id: &str, value: Option<f64>) -> PeriodQueryRow {
        PeriodQueryRow {
            entity_id: entity_id.to_owned(),
            value,
        }
    }

    fn peer_row(entity_id: &str, target: f64) -> PeerQueryRow {
        PeerQueryRow {
            entity_id: entity_id.to_owned(),
            target_value: Some(target),
            p25: None,
            median: None,
            p75: None,
            min: None,
            max: None,
            n: Some(3),
        }
    }

    fn record(
        plan: &mut CachePlan,
        origin: Origin,
        metric_index: usize,
        view_index: usize,
        outcome: ViewOutcome,
    ) {
        plan.record(origin, metric_index, view_index, outcome)
            .unwrap_or_else(|error| panic!("result must route: {error}"));
    }

    fn view_at(outcome: &CacheOutcome, metric: usize, view: usize) -> &MetricResultViewDto {
        match outcome.views[metric][view].as_ref() {
            Some(view) => view,
            None => panic!("view [{metric}][{view}] was never filled"),
        }
    }

    fn period_values(view: &MetricResultViewDto) -> &[PeriodValueDto] {
        match view {
            MetricResultViewDto::Period { values } => values,
            other => panic!("expected a period view, got {other:?}"),
        }
    }

    fn peer_values(view: &MetricResultViewDto) -> &[PeerValueDto] {
        match view {
            MetricResultViewDto::Peer { values } => values,
            other => panic!("expected a peer view, got {other:?}"),
        }
    }

    fn cached_period(value: Option<f64>) -> Option<Vec<u8>> {
        fragment::encode(&PeriodFragment::from_row(Some(&period_row("x", value))))
    }

    /// Every requested entity is queried and nothing is written back when no key
    /// could be derived — the shape the disabled cache produces.
    #[test]
    fn an_uncacheable_request_queries_everything_and_writes_nothing() {
        let req = request(
            vec![entity(1), entity(2)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period, ValidatedMetricView::Histogram],
            )],
        );
        let mut plan = CachePlan::build(&req, uncacheable_keys(&req), &[]);

        let entity_counts: Vec<(Origin, usize)> = plan
            .narrowed()
            .iter()
            .map(|(origin, narrowed)| (*origin, narrowed.req.entity.len()))
            .collect();
        assert_eq!(
            entity_counts,
            vec![(Origin::FullSet, 2), (Origin::Subset, 2)],
            "both narrowed requests keep every requested entity"
        );

        record(
            &mut plan,
            Origin::Subset,
            0,
            0,
            ViewOutcome::PeriodRows(vec![period_row(&entity(1).to_string(), Some(4.0))]),
        );
        record(
            &mut plan,
            Origin::FullSet,
            0,
            0,
            ViewOutcome::View(MetricResultViewDto::Histogram { values: Vec::new() }),
        );

        let outcome = plan
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        assert!(outcome.writes.is_empty(), "nothing is cacheable");
        assert_eq!(period_values(view_at(&outcome, 0, 0)).len(), 2);
    }

    #[test]
    fn a_fully_cached_request_runs_no_query() {
        let req = request(
            vec![entity(1), entity(2)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period],
            )],
        );
        let cached = vec![cached_period(Some(1.0)), cached_period(Some(2.0))];

        let plan = CachePlan::build(&req, keys(&req), &cached);

        assert!(plan.narrowed().is_empty());
        assert_eq!(plan.stats().hits, 2);
        assert_eq!(plan.stats().misses, 0);

        let outcome = plan
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        let values = period_values(view_at(&outcome, 0, 0));
        assert_eq!(values[0].value, Some(1.0));
        assert_eq!(values[1].value, Some(2.0));
        assert!(outcome.writes.is_empty(), "a hit must not refresh its TTL");
    }

    #[test]
    fn a_partial_hit_queries_only_the_missing_entities() {
        let req = request(
            vec![entity(1), entity(2), entity(3)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period],
            )],
        );
        let cached = vec![cached_period(Some(1.0)), None, cached_period(Some(3.0))];

        let mut plan = CachePlan::build(&req, keys(&req), &cached);

        let narrowed = plan.narrowed();
        assert_eq!(narrowed.len(), 1);
        let (origin, subset) = narrowed[0];
        assert_eq!(origin, Origin::Subset);
        assert_eq!(subset.req.entity.entity_ids(), vec![entity(2).to_string()]);

        record(
            &mut plan,
            Origin::Subset,
            0,
            0,
            ViewOutcome::PeriodRows(vec![period_row(&entity(2).to_string(), Some(2.0))]),
        );

        let outcome = plan
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        let values = period_values(view_at(&outcome, 0, 0));
        assert_eq!(
            values.iter().map(|value| value.value).collect::<Vec<_>>(),
            vec![Some(1.0), Some(2.0), Some(3.0)],
            "cached and fresh values merge in requested order"
        );
        assert_eq!(
            outcome.writes.len(),
            1,
            "only the recomputed entity is stored"
        );
    }

    /// An entity the subset query returned no row for is stored as a known-absent
    /// value, so the next request does not re-query it.
    #[test]
    fn an_entity_with_no_observations_is_cached_as_unknown() {
        let req = request(
            vec![entity(1)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period],
            )],
        );
        let mut plan = CachePlan::build(&req, keys(&req), &[None]);

        record(
            &mut plan,
            Origin::Subset,
            0,
            0,
            ViewOutcome::PeriodRows(Vec::new()),
        );

        let outcome = plan
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        assert_eq!(period_values(view_at(&outcome, 0, 0))[0].value, None);
        assert_eq!(outcome.writes.len(), 1);
    }

    #[test]
    fn a_cached_peer_entity_without_a_pool_stays_omitted() {
        let req = request(
            vec![entity(1), entity(2)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Peer {
                    cohort_key: "org_unit".to_owned(),
                }],
            )],
        );
        let absent = fragment::encode(&PeerFragment::from_row(None));
        let present = fragment::encode(&PeerFragment::from_row(Some(&peer_row("x", 9.0))));
        let cached = vec![absent, present];

        let plan = CachePlan::build(&req, keys(&req), &cached);

        assert!(plan.narrowed().is_empty());

        let outcome = plan
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        let values = peer_values(view_at(&outcome, 0, 0));
        assert_eq!(values.len(), 1, "the entity with no pool stays omitted");
        assert_eq!(values[0].entity_id, entity(2).to_string());
        assert_eq!(values[0].target_value, Some(9.0));
    }

    /// The subset query covers the union across views, so each view takes fresh
    /// rows only for the entities it was itself missing and keeps the rest.
    #[test]
    fn each_view_takes_fresh_rows_only_for_the_entities_it_was_missing() {
        let req = request(
            vec![entity(1), entity(2)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period, ValidatedMetricView::Period],
            )],
        );
        // View 0 misses entity 1, view 1 misses entity 2, so the subset covers
        // both entities and each view sees fresh rows for both.
        let cached = vec![
            None,
            cached_period(Some(20.0)),
            cached_period(Some(1.0)),
            None,
        ];

        let mut plan = CachePlan::build(&req, keys(&req), &cached);

        let fresh = || {
            vec![
                period_row(&entity(1).to_string(), Some(99.0)),
                period_row(&entity(2).to_string(), Some(88.0)),
            ]
        };
        record(
            &mut plan,
            Origin::Subset,
            0,
            0,
            ViewOutcome::PeriodRows(fresh()),
        );
        record(
            &mut plan,
            Origin::Subset,
            0,
            1,
            ViewOutcome::PeriodRows(fresh()),
        );

        let outcome = plan
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        assert_eq!(
            period_values(view_at(&outcome, 0, 0))
                .iter()
                .map(|value| value.value)
                .collect::<Vec<_>>(),
            vec![Some(99.0), Some(20.0)],
            "view 0 takes fresh entity 1 and keeps its cached entity 2"
        );
        assert_eq!(
            period_values(view_at(&outcome, 0, 1))
                .iter()
                .map(|value| value.value)
                .collect::<Vec<_>>(),
            vec![Some(1.0), Some(88.0)],
            "view 1 keeps its cached entity 1 and takes fresh entity 2"
        );
        assert_eq!(
            outcome.writes.len(),
            2,
            "each view writes back only the entity it was missing"
        );
    }

    /// The subset is a union across views, so a view that never ran must keep
    /// the rows it had for entities another view happened to be missing.
    #[test]
    fn a_view_that_did_not_run_keeps_its_cached_rows() {
        let req = request(
            vec![entity(1), entity(2)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Period, ValidatedMetricView::Period],
            )],
        );
        // View 0 is fully cached; view 1 misses entity 1 and drives the subset.
        let cached = vec![
            cached_period(Some(1.0)),
            cached_period(Some(2.0)),
            None,
            cached_period(Some(20.0)),
        ];

        let mut plan = CachePlan::build(&req, keys(&req), &cached);

        record(
            &mut plan,
            Origin::Subset,
            0,
            0,
            ViewOutcome::PeriodRows(vec![period_row(&entity(1).to_string(), Some(99.0))]),
        );

        let outcome = plan
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        let untouched = period_values(view_at(&outcome, 0, 0));
        assert_eq!(
            untouched
                .iter()
                .map(|value| value.value)
                .collect::<Vec<_>>(),
            vec![Some(1.0), Some(2.0)],
            "a view outside the subset keeps every cached row"
        );
        assert!(
            outcome.writes.len() == 1,
            "only the view that ran writes back"
        );
    }

    /// The narrowed requests carry their own metric and view indexes; every
    /// result has to land back on the coordinates the caller asked about.
    #[test]
    fn planned_queries_remap_onto_the_requested_coordinates() {
        let req = request(
            vec![entity(1)],
            vec![
                metric_request(
                    sum_metric(),
                    vec![ValidatedMetricView::Histogram, ValidatedMetricView::Period],
                ),
                metric_request(
                    sum_metric(),
                    vec![ValidatedMetricView::Timeseries {
                        bucket: Bucket::Day,
                        dimensions: Vec::new(),
                        group_limit: None,
                    }],
                ),
            ],
        );
        let mut plan = CachePlan::build(&req, uncacheable_keys(&req), &[]);

        let mut recorded = Vec::new();
        for (origin, narrowed) in plan.narrowed() {
            let planned = plan_queries(&narrowed.req, &BTreeMap::new())
                .unwrap_or_else(|error| panic!("planning must succeed: {error}"));
            for query in planned {
                match query {
                    PlannedQuery::PeriodBatch { items, .. }
                    | PlannedQuery::PeerBatch { items, .. } => {
                        for item in items {
                            recorded.push((origin, item.metric_index, item.view_index));
                        }
                    }
                    PlannedQuery::Single {
                        metric_index,
                        view_index,
                        ..
                    } => recorded.push((origin, metric_index, view_index)),
                }
            }
        }

        for (origin, metric_index, view_index) in recorded {
            let outcome = match origin {
                Origin::Subset => ViewOutcome::PeriodRows(Vec::new()),
                Origin::FullSet => {
                    ViewOutcome::View(MetricResultViewDto::Histogram { values: Vec::new() })
                }
            };
            record(&mut plan, origin, metric_index, view_index, outcome);
        }

        let outcome = plan
            .finish(&req)
            .unwrap_or_else(|error| panic!("every requested view must be filled: {error}"));

        assert_eq!(outcome.views.len(), 2);
        assert_eq!(outcome.views[0].len(), 2);
        assert_eq!(outcome.views[1].len(), 1);
        assert!(outcome.views.iter().flatten().all(Option::is_some));
    }

    /// The whole point of assembling through the builders: whether a value came
    /// from ClickHouse or from Redis must not be visible on the wire.
    #[test]
    fn a_cached_response_is_byte_identical_to_a_computed_one() {
        let req = request(
            vec![entity(1), entity(2), entity(3)],
            vec![metric_request(
                sum_metric(),
                vec![
                    ValidatedMetricView::Period,
                    ValidatedMetricView::Peer {
                        cohort_key: "org_unit".to_owned(),
                    },
                ],
            )],
        );
        // Entity 2 has no observations and no cohort row.
        let period_rows = || {
            vec![
                period_row(&entity(1).to_string(), Some(1.0)),
                period_row(&entity(3).to_string(), None),
            ]
        };
        let peer_rows = || {
            vec![
                peer_row(&entity(3).to_string(), 30.0),
                peer_row(&entity(1).to_string(), 10.0),
            ]
        };

        let mut computed =
            CachePlan::build(&req, keys(&req), &[None, None, None, None, None, None]);
        record(
            &mut computed,
            Origin::Subset,
            0,
            0,
            ViewOutcome::PeriodRows(period_rows()),
        );
        record(
            &mut computed,
            Origin::Subset,
            0,
            1,
            ViewOutcome::PeerRows(peer_rows()),
        );
        let computed = computed
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        // Replay the writes the computed pass produced as the next request's hits.
        let by_key: BTreeMap<&str, &Vec<u8>> = computed
            .writes
            .iter()
            .map(|(key, bytes)| (key.as_str(), bytes))
            .collect();
        let replayed: Vec<Option<Vec<u8>>> = flat_keys(&keys(&req))
            .iter()
            .map(|key| by_key.get(key.as_str()).map(|bytes| (*bytes).clone()))
            .collect();
        assert!(
            replayed.iter().all(Option::is_some),
            "every entity of a fully computed pass must be written back"
        );

        let cached = CachePlan::build(&req, keys(&req), &replayed);
        assert!(cached.narrowed().is_empty(), "nothing left to query");
        let cached = cached
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        for (index, (left, right)) in computed.views[0].iter().zip(&cached.views[0]).enumerate() {
            let left = serde_json::to_value(left).unwrap_or_else(|error| panic!("{error}"));
            let right = serde_json::to_value(right).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(left, right, "view {index} must render identically");
        }
    }

    /// A whole-view hit replays a stored DTO verbatim, so a request that asks
    /// for the same entities in another order must not collect the first
    /// request's ordering.
    #[test]
    fn a_permuted_request_does_not_inherit_a_cached_entity_order() {
        let histogram = || {
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Histogram],
            )]
        };
        let forward = request(vec![entity(1), entity(2)], histogram());
        let reversed = request(vec![entity(2), entity(1)], histogram());

        let mut computed = CachePlan::build(&forward, keys(&forward), &[None]);
        record(
            &mut computed,
            Origin::FullSet,
            0,
            0,
            ViewOutcome::View(build_histogram_view(&forward, Vec::new())),
        );
        let computed = computed
            .finish(&forward)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));
        assert_eq!(computed.writes.len(), 1, "the computed view is stored");

        // Offer that stored view to the permuted request under its own keys.
        let stored: Vec<Option<Vec<u8>>> = flat_keys(&keys(&reversed))
            .iter()
            .map(|key| {
                computed
                    .writes
                    .iter()
                    .find(|(written, _)| written == key)
                    .map(|(_, bytes)| bytes.clone())
            })
            .collect();

        assert!(
            stored.iter().all(Option::is_none),
            "a permuted request must not match the stored key"
        );

        let mut permuted = CachePlan::build(&reversed, keys(&reversed), &stored);
        record(
            &mut permuted,
            Origin::FullSet,
            0,
            0,
            ViewOutcome::View(build_histogram_view(&reversed, Vec::new())),
        );
        let permuted = permuted
            .finish(&reversed)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        let MetricResultViewDto::Histogram { values } = view_at(&permuted, 0, 0) else {
            panic!("expected a histogram view");
        };
        assert_eq!(
            values
                .iter()
                .map(|v| v.entity_id.clone())
                .collect::<Vec<_>>(),
            vec![entity(2).to_string(), entity(1).to_string()],
            "the response follows the order this request asked for"
        );
    }

    /// The whole-view path: a stored DTO is served back without querying, and a
    /// hit is not rewritten.
    #[test]
    fn a_whole_view_hit_is_served_from_cache_and_not_rewritten() {
        let req = request(
            vec![entity(1), entity(2)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Histogram],
            )],
        );

        let mut computed = CachePlan::build(&req, keys(&req), &[None]);
        record(
            &mut computed,
            Origin::FullSet,
            0,
            0,
            ViewOutcome::View(build_histogram_view(&req, Vec::new())),
        );
        let computed = computed
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));
        assert_eq!(computed.writes.len(), 1, "a computed whole view is stored");

        let stored = vec![Some(computed.writes[0].1.clone())];
        let cached = CachePlan::build(&req, keys(&req), &stored);

        assert!(cached.narrowed().is_empty(), "nothing left to query");
        assert_eq!(cached.stats().hits, 1);

        let cached = cached
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        assert!(
            cached.writes.is_empty(),
            "a whole-view hit must not rewrite its entry"
        );
        let MetricResultViewDto::Histogram { values } = view_at(&cached, 0, 0) else {
            panic!("expected the histogram view back, not another variant");
        };
        assert_eq!(values.len(), 2, "every requested entity is still listed");
    }

    /// The subset is a union across views, so an uncacheable view must not drag
    /// a partially-cached view into re-querying and rewriting everything.
    #[test]
    fn an_uncacheable_view_does_not_invalidate_another_views_cached_rows() {
        let req = request(
            vec![entity(1), entity(2)],
            vec![
                metric_request(custom_sql_metric(), vec![ValidatedMetricView::Period]),
                metric_request(sum_metric(), vec![ValidatedMetricView::Period]),
            ],
        );
        // The custom-SQL metric is uncacheable, so it contributes no keys; the
        // managed metric has entity 1 cached and entity 2 missing.
        let cached = vec![cached_period(Some(1.0)), None];

        let mut plan = CachePlan::build(&req, keys(&req), &cached);

        record(
            &mut plan,
            Origin::Subset,
            0,
            0,
            ViewOutcome::PeriodRows(vec![
                period_row(&entity(1).to_string(), Some(11.0)),
                period_row(&entity(2).to_string(), Some(22.0)),
            ]),
        );
        record(
            &mut plan,
            Origin::Subset,
            1,
            0,
            ViewOutcome::PeriodRows(vec![
                period_row(&entity(1).to_string(), Some(99.0)),
                period_row(&entity(2).to_string(), Some(2.0)),
            ]),
        );

        let outcome = plan
            .finish(&req)
            .unwrap_or_else(|error| panic!("plan must finish: {error}"));

        let managed = period_values(view_at(&outcome, 1, 0));
        assert_eq!(
            managed.iter().map(|value| value.value).collect::<Vec<_>>(),
            vec![Some(1.0), Some(2.0)],
            "the cached row survives an unrelated view's re-query"
        );
        assert_eq!(
            outcome.writes.len(),
            1,
            "only the entity this view was missing is written back"
        );
    }

    #[test]
    fn a_view_that_was_never_produced_is_an_internal_error() {
        let req = request(
            vec![entity(1)],
            vec![metric_request(
                sum_metric(),
                vec![ValidatedMetricView::Histogram],
            )],
        );

        let plan = CachePlan::build(&req, uncacheable_keys(&req), &[]);

        assert!(plan.finish(&req).is_err());
    }
}
