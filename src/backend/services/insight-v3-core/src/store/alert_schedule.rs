//! The check schedule as `BullMQ` holds it in Redis: one repeating job per
//! enabled rule, and the worker that runs them.
//!
//! `BullMQ` owns what a job queue owns — locks, stalled-job recovery, one
//! iteration at a time per scheduler — so the rules here are only about
//! naming: a scheduler is keyed by the rule, and its job carries the
//! revision the store will accept a check for.

#[cfg(test)]
pub(crate) mod memory;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bullmq::job_scheduler::RepeatOptions;
use bullmq::options::RedisConnectionOptions;
use bullmq::worker::CancellationToken as JobCancellation;
use bullmq::{Job, Queue, QueueOptions, Worker, WorkerOptions};
use uuid::Uuid;

use bullmq::types::BackoffStrategy;

use crate::domain::alerts::UnknownReason;
use crate::domain::alerts::delivery::{Attempted, Delivered, Deliverer, Deliveries, DeliveryJob};
use crate::domain::alerts::evaluation::{Evaluated, EvaluationJob, Evaluator};
use crate::domain::alerts::rule::Recorded;
use crate::domain::alerts::schedule::{AlertSchedule, ScheduleError, Scheduled};

/// The queue the checks go through. One per installation; rules are told
/// apart by their scheduler key.
const QUEUE: &str = "insight-v3-alerts-evaluate";
const JOB: &str = "evaluate";
/// The queue owed notifications go through, one job per notification,
/// keyed by it so a second enqueue is the same job.
const DELIVERY_QUEUE: &str = "insight-v3-alerts-deliver";
const DELIVERY_JOB: &str = "deliver";
const SCHEDULER_PREFIX: &str = "rule:";
/// How many schedulers one listing reads. Bounded by the rule limit, which
/// an installation sets far below this.
const LISTING_BOUND: isize = 10_000;
/// How long a shutdown waits for running checks.
const CLOSE_TIMEOUT_MS: u64 = 10_000;
const STALLED_INTERVAL_MS: u64 = 30_000;

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

/// The queue side: what rule writes reach.
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
                None,
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

/// What a running check needs, given to the worker once.
pub(crate) trait Checks: Send + Sync + 'static {
    fn evaluator(&self) -> Evaluator<'_>;

    /// Where a notification the check owes is queued to be sent.
    fn deliveries(&self) -> &dyn Deliveries;
}

/// What a running send needs, given to the worker once.
pub(crate) trait Sends: Send + Sync + 'static {
    fn deliverer(&self) -> Deliverer<'_>;
}

/// The queue side of delivery: what a recorded notification reaches.
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

/// The worker side of delivery: takes sends off the queue and runs them.
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

/// One send, as the queue hands it over. An unconfirmed send that has
/// attempts left fails the job so `BullMQ` retries it after its backoff;
/// everything else is final and the job completes.
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

/// The worker side: takes checks off the queue and runs them until told to
/// stop.
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

    /// Stops taking checks and waits, briefly, for the running ones.
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

/// One check, as the queue hands it over. Only a store that did not answer
/// fails the job; the iteration is then dropped and the next one runs at its
/// time. Everything else is a finding recorded on the rule.
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
