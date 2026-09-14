use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use super::executor::{ReportRowSink, ReportSinkError, execute_report};
use super::executor_test_fixtures::*;
use super::planner::{ReportPlannerLimits, plan_report};
use super::query::{ReportMetricQuery, ReportQueryError, ReportQueryRunner, ReportQuerySubject};
use super::row::{ReportMetricValue, ReportMetricValues, ReportRow};

#[tokio::test]
async fn executes_people_batches_sequentially_and_emits_person_major_rows() {
    let profiles = [
        profile(2, "Second"),
        profile(1, "First"),
        profile(3, "Third"),
    ];
    let recipe = people_recipe(&profiles, 2);
    let plan = plan_report(
        &recipe,
        &profiles,
        ReportPlannerLimits {
            max_batch_cells: 4,
            max_total_cells: u64::MAX,
        },
    )
    .unwrap_or_else(|error| panic!("report should plan: {error}"));
    let runner = FakeRunner::new(HashMap::from([
        (
            0,
            vec![
                value(2, "2026-01-01", Some(20.0)),
                value(1, "2026-02-01", Some(10.0)),
            ],
        ),
        (1, vec![value(3, "2026-01-01", Some(30.0))]),
    ]));

    let rows = execute_report(
        &recipe,
        &plan,
        &profiles,
        context(),
        &runner,
        RecordingSink::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("report should execute: {error}"));

    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0][0], text("Second"));
    assert_eq!(rows[0][1], text("2026-01"));
    assert_eq!(rows[0][4], number(20.0));
    assert_eq!(rows[1][4], None);
    assert_eq!(rows[2][0], text("First"));
    assert_eq!(rows[3][4], number(10.0));
    assert_eq!(rows[4][0], text("Third"));
    assert_eq!(rows[4][5], number(30.0));
    assert_eq!(
        runner.person_batches(),
        vec![2, 2, 1, 1, 3, 3],
        "each two-query batch must finish before the next person starts"
    );
}

#[tokio::test]
async fn tenant_execution_has_no_profile_columns_and_orders_periods_chronologically() {
    let tenant_id = Uuid::from_u128(9);
    let recipe = tenant_recipe(tenant_id, 1);
    let plan = plan_report(
        &recipe,
        &[],
        ReportPlannerLimits {
            max_batch_cells: 1,
            max_total_cells: u64::MAX,
        },
    )
    .unwrap_or_else(|error| panic!("report should plan: {error}"));
    let runner = FakeRunner::new(HashMap::from([(
        0,
        vec![ReportMetricValue {
            entity_id: tenant_id,
            bucket_start: date("2026-02-01"),
            value: Some(4.0),
        }],
    )]));

    let rows = execute_report(
        &recipe,
        &plan,
        &[],
        context(),
        &runner,
        RecordingSink::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("report should execute: {error}"));

    assert_eq!(plan.columns.len(), 4);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        vec![
            text("2026-01"),
            text("2026-01-01"),
            text("2026-01-31"),
            None
        ]
        .into()
    );
    assert_eq!(
        rows[1],
        vec![
            text("2026-02"),
            text("2026-02-01"),
            text("2026-02-28"),
            number(4.0)
        ]
        .into()
    );
    assert_eq!(runner.tenant_queries(), 2);
}

#[tokio::test]
async fn keeps_more_than_fifty_metrics_in_recipe_order_with_four_queries_in_flight() {
    let tenant_id = Uuid::from_u128(9);
    let recipe = tenant_recipe(tenant_id, 64);
    let plan = plan_report(
        &recipe,
        &[],
        ReportPlannerLimits {
            max_batch_cells: 100_000,
            max_total_cells: u64::MAX,
        },
    )
    .unwrap_or_else(|error| panic!("report should plan: {error}"));
    let responses = (0..64)
        .map(|metric_index| {
            (
                metric_index,
                vec![ReportMetricValue {
                    entity_id: tenant_id,
                    bucket_start: date("2026-01-01"),
                    value: Some(metric_number(metric_index)),
                }],
            )
        })
        .collect();
    let runner = FakeRunner::new(responses);

    let rows = execute_report(
        &recipe,
        &plan,
        &[],
        context(),
        &runner,
        RecordingSink::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("report should execute: {error}"));

    assert_eq!(runner.max_active.load(Ordering::SeqCst), 4);
    assert_eq!(rows.len(), 2);
    for metric_index in 0..64 {
        assert_eq!(
            rows[0][metric_index + 3],
            number(metric_number(metric_index))
        );
    }
}

#[tokio::test]
async fn query_failure_writes_no_batch_and_never_finishes_sink() {
    let tenant_id = Uuid::from_u128(9);
    let recipe = tenant_recipe(tenant_id, 5);
    let plan = plan_report(
        &recipe,
        &[],
        ReportPlannerLimits {
            max_batch_cells: 100,
            max_total_cells: u64::MAX,
        },
    )
    .unwrap_or_else(|error| panic!("report should plan: {error}"));
    let runner = FakeRunner::new(HashMap::new()).failing(2);
    let sink = RecordingSink::default();
    let writes = Arc::clone(&sink.writes);
    let finishes = Arc::clone(&sink.finishes);

    let result = execute_report(&recipe, &plan, &[], context(), &runner, sink).await;
    assert!(result.is_err());
    assert!(
        writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
    assert_eq!(finishes.load(Ordering::SeqCst), 0);
}

struct FakeRunner {
    responses: HashMap<usize, Vec<ReportMetricValue>>,
    fail_metric: Option<usize>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    subjects: Mutex<Vec<ReportQuerySubject>>,
}

impl FakeRunner {
    fn new(responses: HashMap<usize, Vec<ReportMetricValue>>) -> Self {
        Self {
            responses,
            fail_metric: None,
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            subjects: Mutex::new(Vec::new()),
        }
    }
    fn failing(mut self, metric_index: usize) -> Self {
        self.fail_metric = Some(metric_index);
        self
    }
    fn person_batches(&self) -> Vec<u128> {
        self.subjects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|subject| match subject {
                ReportQuerySubject::People(ids) => ids.first().map(Uuid::as_u128),
                ReportQuerySubject::Tenant(_) => None,
            })
            .collect()
    }
    fn tenant_queries(&self) -> usize {
        self.subjects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|subject| matches!(subject, ReportQuerySubject::Tenant(_)))
            .count()
    }
}

#[async_trait]
impl ReportQueryRunner for FakeRunner {
    async fn run(&self, query: ReportMetricQuery) -> Result<ReportMetricValues, ReportQueryError> {
        self.subjects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(query.subject.clone());
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(2)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.fail_metric == Some(query.metric_index) {
            return Err(ReportQueryError::InvalidResultShape);
        }
        let selected_ids = match &query.subject {
            ReportQuerySubject::People(ids) => ids.clone(),
            ReportQuerySubject::Tenant(id) => vec![*id],
        };
        let values = self
            .responses
            .get(&query.metric_index)
            .into_iter()
            .flatten()
            .filter(|value| selected_ids.contains(&value.entity_id))
            .filter(|value| {
                value.bucket_start >= query.first_bucket_start
                    && value.bucket_start <= query.last_bucket_start
            })
            .cloned()
            .collect();
        Ok(ReportMetricValues {
            metric_index: query.metric_index,
            values,
        })
    }
}

#[derive(Default)]
struct RecordingSink {
    writes: Arc<Mutex<Vec<ReportRow>>>,
    finishes: Arc<AtomicUsize>,
}

#[async_trait]
impl ReportRowSink for RecordingSink {
    type Output = Vec<ReportRow>;
    async fn write_rows(&mut self, rows: &[ReportRow]) -> Result<(), ReportSinkError> {
        self.writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(rows);
        Ok(())
    }

    async fn finish(self) -> Result<Self::Output, ReportSinkError> {
        self.finishes.fetch_add(1, Ordering::SeqCst);
        let rows = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Ok(rows)
    }
}
