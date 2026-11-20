#[cfg(test)]
pub(crate) mod memory;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bullmq::job_scheduler::RepeatOptions;
use bullmq::options::{JobOptions, RedisConnectionOptions};
use bullmq::types::{BackoffStrategy, RemoveOnFinish};
use bullmq::worker::CancellationToken as JobCancellation;
use bullmq::{Job, Queue, QueueOptions, Worker, WorkerOptions};
use uuid::Uuid;

use crate::domain::alerts::UnknownReason;
use crate::domain::alerts::delivery::{Attempted, Delivered, Deliverer, Deliveries, DeliveryJob};
use crate::domain::alerts::evaluation::{Evaluated, EvaluationJob, Evaluator};
use crate::domain::alerts::rule::Recorded;
use crate::domain::alerts::schedule::{AlertSchedule, ScheduleError, Scheduled};

const QUEUE: &str = "insight-v3-alerts-evaluate";
const JOB: &str = "evaluate";
const DELIVERY_QUEUE: &str = "insight-v3-alerts-deliver";
const DELIVERY_JOB: &str = "deliver";
const SCHEDULER_PREFIX: &str = "rule:";
const LISTING_BOUND: isize = 10_000;
const CLOSE_TIMEOUT_MS: u64 = 10_000;
const STALLED_INTERVAL_MS: u64 = 30_000;
const FINISHED_CHECKS_KEPT: usize = 1000;

fn scheduler_id(rule_id: Uuid) -> String {
    format!("{SCHEDULER_PREFIX}{}", rule_id.simple())
}

fn rule_of(scheduler_id: &str) -> Option<Uuid> {
    scheduler_id
        .strip_prefix(SCHEDULER_PREFIX)
        .and_then(|id| Uuid::parse_str(id).ok())
}

fn unreachable(error: impl std::error::Error + Send + Sync + 'static) -> ScheduleError {
    ScheduleError(Box::new(error))
}

pub(crate) struct RedisSchedule {
    queue: Queue,
}

impl RedisSchedule {
    pub(crate) async fn connect(redis_url: &str) -> Result<Self, ScheduleError> {
        let queue = Queue::with_options(
            QUEUE,
            QueueOptions {
                connection: connection(redis_url),
                ..Default::default()
            },
        )
        .await
        .map_err(unreachable)?;

        Ok(Self { queue })
    }
}

fn connection(redis_url: &str) -> RedisConnectionOptions {
    RedisConnectionOptions {
        url: redis_url.to_owned(),
        ..Default::default()
    }
}

#[async_trait]
impl AlertSchedule for RedisSchedule {
    async fn upsert(&self, scheduled: Scheduled) -> Result<(), ScheduleError> {
        let data = serde_json::to_value(scheduled.job).map_err(unreachable)?;
        self.queue
            .upsert_job_scheduler(
                &scheduler_id(scheduled.job.rule_id),
                RepeatOptions {
                    every: Some(u64::from(scheduled.every_secs) * 1000),
                    ..Default::default()
                },
                Some(JOB),
                Some(data),
                Some(JobOptions {
                    remove_on_complete: Some(RemoveOnFinish::Count(FINISHED_CHECKS_KEPT)),
                    remove_on_fail: Some(RemoveOnFinish::Count(FINISHED_CHECKS_KEPT)),
                    ..JobOptions::default()
                }),
            )
            .await
            .map_err(unreachable)?;

        Ok(())
    }

    async fn remove(&self, rule_id: Uuid) -> Result<(), ScheduleError> {
        self.queue
            .remove_job_scheduler(&scheduler_id(rule_id))
            .await
            .map_err(unreachable)?;

        Ok(())
    }

    async fn scheduled(&self) -> Result<Vec<Uuid>, ScheduleError> {
        let listed = self
            .queue
            .get_job_schedulers(0, LISTING_BOUND, true)
            .await
            .map_err(unreachable)?;

        Ok(listed
            .iter()
            .filter_map(|scheduler| rule_of(&scheduler.key))
            .collect())
    }
}

impl fmt::Debug for RedisSchedule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisSchedule")
            .field("queue", &QUEUE)
            .finish()
    }
}

pub(crate) trait Checks: Send + Sync + 'static {
    fn evaluator(&self) -> Evaluator<'_>;

    fn deliveries(&self) -> &dyn Deliveries;
}

pub(crate) trait Sends: Send + Sync + 'static {
    fn deliverer(&self) -> Deliverer<'_>;
}

/// A finished job is removed at once, whichever way it ended: the outcome is
/// on the notification, and a job left behind would swallow the next
/// enqueue of the same id when reconciling queues an owed one again.
pub(crate) struct RedisDeliveries {
    queue: Queue,
    attempts: u32,
    backoff: Duration,
}

impl RedisDeliveries {
    pub(crate) async fn connect(
        redis_url: &str,
        attempts: u32,
        backoff: Duration,
    ) -> Result<Self, ScheduleError> {
        let queue = Queue::with_options(
            DELIVERY_QUEUE,
            QueueOptions {
                connection: connection(redis_url),
                ..Default::default()
            },
        )
        .await
        .map_err(unreachable)?;

        Ok(Self {
            queue,
            attempts,
            backoff,
        })
    }
}

#[async_trait]
impl Deliveries for RedisDeliveries {
    async fn enqueue(&self, notification_id: Uuid) -> Result<(), ScheduleError> {
        let job = DeliveryJob { notification_id };
        let data = serde_json::to_value(job).map_err(unreachable)?;
        let backoff_ms = u64::try_from(self.backoff.as_millis()).unwrap_or(u64::MAX);
        self.queue
            .add(DELIVERY_JOB, data)
            .job_id(notification_id.simple().to_string())
            .attempts(self.attempts)
            .backoff(BackoffStrategy::Exponential(backoff_ms))
            .remove_on_complete(RemoveOnFinish::Bool(true))
            .remove_on_fail(RemoveOnFinish::Bool(true))
            .await
            .map_err(unreachable)?;

        Ok(())
    }
}

impl fmt::Debug for RedisDeliveries {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisDeliveries")
            .field("queue", &DELIVERY_QUEUE)
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

pub(crate) struct DeliveryWorker {
    worker: Worker,
}

impl DeliveryWorker {
    pub(crate) async fn start(
        redis_url: &str,
        sends: Arc<dyn Sends>,
        concurrency: usize,
        timeout: Duration,
        max_attempts: u32,
    ) -> Result<Self, ScheduleError> {
        let options = WorkerOptions {
            connection: connection(redis_url),
            concurrency,
            lock_duration: u64::try_from(timeout.as_millis().saturating_mul(2)).unwrap_or(u64::MAX),
            stalled_interval: STALLED_INTERVAL_MS,
            ..Default::default()
        };
        let worker = Worker::with_options(
            DELIVERY_QUEUE,
            move |job: Job, _: JobCancellation| {
                let sends = Arc::clone(&sends);
                async move { send(&*sends, &job, max_attempts).await }
            },
            options,
        )
        .await
        .map_err(unreachable)?;

        Ok(Self { worker })
    }

    pub(crate) async fn stop(&self) {
        if let Err(error) = self.worker.close(CLOSE_TIMEOUT_MS).await {
            tracing::warn!(%error, "the delivery worker did not close cleanly");
        }
    }
}

impl fmt::Debug for DeliveryWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeliveryWorker")
            .field("worker", &self.worker.id())
            .finish()
    }
}

async fn send(
    sends: &dyn Sends,
    job: &Job,
    max_attempts: u32,
) -> Result<serde_json::Value, bullmq::Error> {
    let parsed: DeliveryJob = serde_json::from_value(job.data().clone())?;
    let attempt = job.attempts_made().saturating_add(1);

    let delivered = sends
        .deliverer()
        .deliver(parsed, attempt, max_attempts)
        .await
        .map_err(|error| {
            tracing::error!(notification_id = %parsed.notification_id, error = ?error, "a notification send could not be recorded");
            bullmq::Error::Unrecoverable("the alert store did not answer".to_owned())
        })?;

    match delivered {
        Delivered::Skipped => {
            tracing::info!(notification_id = %parsed.notification_id, attempt, "notification send skipped");
            Ok(serde_json::json!({ "skipped": true }))
        }
        Delivered::Attempted(Attempted::Sent(receipt)) => {
            tracing::info!(notification_id = %parsed.notification_id, attempt, receipt = receipt.0, "notification sent");
            Ok(serde_json::json!({ "sent": receipt.0 }))
        }
        Delivered::Attempted(Attempted::Retry(reason)) => {
            tracing::warn!(notification_id = %parsed.notification_id, attempt, max_attempts, reason, "notification send unconfirmed; will retry");
            Err(bullmq::Error::InvalidConfig(reason))
        }
        Delivered::Attempted(Attempted::Failed(reason)) => {
            tracing::warn!(notification_id = %parsed.notification_id, attempt, reason, "notification send failed");
            Ok(serde_json::json!({ "failed": reason }))
        }
    }
}

pub(crate) struct AlertWorker {
    worker: Worker,
}

impl AlertWorker {
    pub(crate) async fn start(
        redis_url: &str,
        checks: Arc<dyn Checks>,
        concurrency: usize,
        lock: Duration,
    ) -> Result<Self, ScheduleError> {
        let options = WorkerOptions {
            connection: connection(redis_url),
            concurrency,
            lock_duration: u64::try_from(lock.as_millis()).unwrap_or(u64::MAX),
            stalled_interval: STALLED_INTERVAL_MS,
            ..Default::default()
        };
        let worker = Worker::with_options(
            QUEUE,
            move |job: Job, _: JobCancellation| {
                let checks = Arc::clone(&checks);
                async move { run(&*checks, &job).await }
            },
            options,
        )
        .await
        .map_err(unreachable)?;

        Ok(Self { worker })
    }

    pub(crate) async fn stop(&self) {
        if let Err(error) = self.worker.close(CLOSE_TIMEOUT_MS).await {
            tracing::warn!(%error, "the alert worker did not close cleanly");
        }
    }
}

impl fmt::Debug for AlertWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AlertWorker")
            .field("worker", &self.worker.id())
            .finish()
    }
}

async fn run(checks: &dyn Checks, job: &Job) -> Result<serde_json::Value, bullmq::Error> {
    let parsed: EvaluationJob = serde_json::from_value(job.data().clone())?;
    let started = std::time::Instant::now();

    let evaluated = checks
        .evaluator()
        .evaluate(parsed)
        .await
        .map_err(|error| {
            tracing::error!(rule_id = %parsed.rule_id, error = ?error, "an alert check could not be recorded");
            bullmq::Error::Unrecoverable("the alert store did not answer".to_owned())
        })?;

    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let summary = match &evaluated {
        Evaluated::Skipped(skipped) => {
            tracing::info!(rule_id = %parsed.rule_id, revision = parsed.revision, skipped = skipped.as_str(), elapsed_ms, "alert check skipped");
            serde_json::json!({ "skipped": skipped.as_str() })
        }
        Evaluated::Recorded(Recorded::Stale) => {
            tracing::info!(rule_id = %parsed.rule_id, revision = parsed.revision, elapsed_ms, "alert check arrived after its revision");
            serde_json::json!({ "skipped": "stale" })
        }
        Evaluated::Recorded(Recorded::Accepted(accepted)) => {
            let rule = &accepted.rule;
            let notification = &accepted.notification;
            if let Some(owed) = notification {
                checks.deliveries().enqueue(owed.id).await.map_err(|error| {
                    tracing::error!(rule_id = %parsed.rule_id, notification_id = %owed.id, error = ?error, "an owed notification could not be queued");
                    bullmq::Error::Unrecoverable("the delivery queue did not answer".to_owned())
                })?;
            }
            let outcome = rule
                .state
                .last_outcome
                .as_ref()
                .map_or("none", |outcome| outcome.as_str());
            tracing::info!(
                rule_id = %parsed.rule_id,
                rule = rule.spec.name.as_str(),
                revision = parsed.revision,
                outcome,
                reason = rule.state.last_reason().map(UnknownReason::as_str),
                notified = notification.is_some(),
                elapsed_ms,
                "alert check recorded"
            );
            serde_json::json!({ "outcome": outcome, "notified": notification.is_some() })
        }
    };

    Ok(summary)
}

#[cfg(test)]
mod tests;
