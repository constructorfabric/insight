//! One check of one rule: run the metric as an on-demand run would, judge
//! the condition over its rows, record what was found.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::condition::{Condition, evaluate};
use super::rule::{AlertRule, AlertStore, AlertStoreError, Recorded, Recording};
use super::scalar::{Outcome, UnknownReason};
use crate::domain::definition::DefinitionId;
use crate::domain::metric_run::MetricRuns;
use crate::domain::query::metric_query::MetricRunError;
use crate::domain::query::time_window::WindowRequest;
use crate::domain::surfaces::CustomError;

/// The check a scheduler hands a worker: which rule, at which revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct EvaluationJob {
    pub(crate) rule_id: Uuid,
    pub(crate) revision: u32,
}

/// How a check ended.
#[derive(Debug)]
pub(crate) enum Evaluated {
    /// Recorded on the rule, with the notification it owes if any.
    Recorded(Recorded),
    /// Nothing to check: the rule is gone, off, or has moved on.
    Skipped(Skipped),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skipped {
    RuleMissing,
    RuleDisabled,
    RevisionReplaced,
}

impl Skipped {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RuleMissing => "rule_missing",
            Self::RuleDisabled => "rule_disabled",
            Self::RevisionReplaced => "revision_replaced",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Evaluator<'a> {
    store: &'a dyn AlertStore,
    runs: MetricRuns<'a>,
}

impl<'a> Evaluator<'a> {
    pub(crate) fn new(store: &'a dyn AlertStore, runs: MetricRuns<'a>) -> Self {
        Self { store, runs }
    }

    /// Runs the check the job asks for and records it.
    ///
    /// Only a store failure is an error: a metric that does not answer is a
    /// finding about the rule, recorded as unknown, not a failed job.
    pub(crate) async fn evaluate(&self, job: EvaluationJob) -> Result<Evaluated, AlertStoreError> {
        let Some(rule) = self.store.get(job.rule_id).await? else {
            return Ok(Evaluated::Skipped(Skipped::RuleMissing));
        };
        if !rule.enabled {
            return Ok(Evaluated::Skipped(Skipped::RuleDisabled));
        }
        if rule.revision != job.revision {
            return Ok(Evaluated::Skipped(Skipped::RevisionReplaced));
        }

        let evaluated_at = Utc::now();
        let observed = self.observe(&rule, evaluated_at).await;
        let recorded = self
            .store
            .record(Recording {
                rule_id: rule.id,
                revision: rule.revision,
                outcome: observed.outcome,
                metric_name: observed.metric_name,
                evaluated_at,
            })
            .await?;

        Ok(Evaluated::Recorded(recorded))
    }

    async fn observe(&self, rule: &AlertRule, evaluated_at: DateTime<Utc>) -> Observed {
        let label = rule.spec.name.as_str();
        let window = match rule.spec.window() {
            Ok(window) => window,
            Err(error) => {
                tracing::warn!(rule = %label, %error, "an alert's range no longer parses");
                return Observed {
                    outcome: Outcome::Unknown(UnknownReason::CompileFailed),
                    metric_name: None,
                };
            }
        };

        observe(
            &self.runs,
            label,
            &rule.spec.metric,
            &window,
            &rule.spec.condition,
            &rule.spec.context_columns,
            evaluated_at,
        )
        .await
    }
}

/// What one check found, and what the metric was called when it was read.
#[derive(Debug)]
pub(crate) struct Observed {
    pub(crate) outcome: Outcome,
    /// None when the metric could not be run.
    pub(crate) metric_name: Option<String>,
}

/// What a check of `metric` over `window` finds now: the run judged by the
/// condition, or the unknown a run that did not answer is. The groups, the
/// row bound and the name come from the same definition the rows did.
/// `label` names the rule, or what asked, in the log; `context` the columns
/// the witness keeps beside the condition's.
pub(crate) async fn observe(
    runs: &MetricRuns<'_>,
    label: &str,
    metric: &DefinitionId,
    window: &WindowRequest,
    condition: &Condition,
    context: &[String],
    evaluated_at: DateTime<Utc>,
) -> Observed {
    match runs.run_named(metric, window).await {
        Ok(run) => Observed {
            outcome: evaluate(
                &run.result,
                condition,
                &run.columns.groups,
                context,
                run.columns.imposed_limit,
                evaluated_at,
            ),
            metric_name: Some(run.name),
        },
        Err(error) => Observed {
            outcome: Outcome::Unknown(unknown_because(label, &error)),
            metric_name: None,
        },
    }
}

/// Which unknown a failed run is. What the warehouse said is logged here,
/// where the rule is known, and never stored.
fn unknown_because(rule: &str, error: &CustomError) -> UnknownReason {
    match error {
        CustomError::NotFound { .. } => UnknownReason::MetricMissing,
        CustomError::Run(MetricRunError::Timeout) => UnknownReason::Timeout,
        CustomError::Run(source) => {
            tracing::error!(rule = %rule, error = ?source, "an alert's metric did not run");
            UnknownReason::RunFailed
        }
        CustomError::Store(source) => {
            tracing::error!(rule = %rule, error = ?source, "an alert's metric could not be read");
            UnknownReason::RunFailed
        }
        CustomError::Datasets(source) => {
            tracing::error!(rule = %rule, error = ?source, "an alert's dataset could not be read");
            UnknownReason::RunFailed
        }
        CustomError::Body(_)
        | CustomError::Compile(_)
        | CustomError::Range(_)
        | CustomError::DatasetNotReady(_)
        | CustomError::Unanswerable(_)
        | CustomError::InUse { .. }
        | CustomError::NotEditor
        | CustomError::NotOwner
        | CustomError::Widget(_) => {
            tracing::warn!(rule = %rule, %error, "an alert's metric does not compile");
            UnknownReason::CompileFailed
        }
    }
}
