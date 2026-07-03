//! Schema-validator (`cpt-metric-cat-component-schema-validator`).
//!
//! Verifies every `metric_catalog.metric_key` (in `table.column` form) resolves
//! to a real column in the ClickHouse analytics warehouse and persists the
//! result onto the row's `schema_*` columns so the read endpoint never pays a
//! ClickHouse round-trip per request.
//!
//! Refs #521. Spec: DESIGN §3.2 / §3.6 / §3.7,
//! `cpt-metric-cat-constraint-schema-validation`.
//!
//! ## Surface
//!
//! - [`SchemaValidator::validate_all`] — startup pass. Iterates every catalog
//!   row, owns the exponential-backoff retry loop on ClickHouse unreachable.
//!   Spawned post-readiness from `main.rs::run_server`.
//! - [`SchemaValidator::validate`] — per-write hook. Debounces against
//!   `schema_checked_at` < 60 s. Exposed as a function for admin-crud (#525)
//!   to call after every successful POST/PUT. Never blocks the write.
//!
//! ## What this module does NOT do
//!
//! - Speak HTTP. No handler routes through here.
//! - Touch any column on `metric_catalog` other than `schema_status` /
//!   `schema_checked_at` / `schema_error_code` (and explicitly NOT `updated_at`).
//! - Cache results in-process. Single source of truth is the row's `schema_status`.
//! - Block startup or readiness on ClickHouse availability — the validator
//!   is spawned only after the HTTP listener socket is bound, so a CH outage
//!   at boot cannot delay or fail the `/health` route.

pub mod backoff;
pub mod error_code;
pub mod parse;
pub mod probe;
pub mod repository;
pub mod status;

#[cfg(test)]
mod live_tests;

use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;
use tokio::time::sleep;

use crate::domain::schema_validator::backoff::BackoffPolicy;
use crate::domain::schema_validator::error_code::SchemaErrorCode;
use crate::domain::schema_validator::parse::parse_metric_key;
use crate::domain::schema_validator::probe::{ProbeError, ProbeOutcome, probe};
use crate::domain::schema_validator::repository::{
    CatalogRow, count_all, find_by_metric_key, mark_all_unchecked, stream_all,
    update_schema_columns,
};
use crate::domain::schema_validator::status::{SchemaState, SchemaStatus, should_log_transition};
use futures_util::StreamExt;

/// Default debounce window for the per-write hook (DESIGN §3.2 + #521 issue body).
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_mins(1);

/// Result of a single per-write `validate(metric_key)` call. The caller
/// (admin-crud) treats this as informational and never rolls back the
/// underlying threshold write — `validator failures NEVER block writes`
/// (DESIGN §3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    /// ClickHouse confirmed the column exists.
    Ok,
    /// Schema mismatch — `schema_error_code` is one of the canonical four.
    Error(SchemaErrorCode),
    /// `schema_checked_at` for this `metric_key` is within the debounce window;
    /// no ClickHouse round-trip was issued. Treated as a no-op by the caller.
    DebouncedSkipped,
    /// The `metric_key` has no row in `metric_catalog`. Distinct from
    /// `ValidatorError` so admin-crud can tell "your metric isn't in the
    /// catalog" (a real, persistent condition) from "the validator's own
    /// MariaDB call failed" (a transient infrastructure issue). Defensive:
    /// admin-crud validates `metric_key` existence before its write, so
    /// production callers should never see this in practice.
    MetricUnknown,
    /// The validator's own MariaDB read/write failed (catalog DB unreachable,
    /// query timeout, etc.). The admin-crud write that triggered this call
    /// has already committed; this signal exists so the caller can log the
    /// validator outage without inferring it from a `MetricUnknown` that
    /// actually means "the row exists, we just couldn't read it".
    ValidatorError,
}

/// Outcome counts from a single `validate_all` pass. Currently consumed only
/// by the startup task's summary log; returned for tests (and a future
/// metrics exporter) so the counters live in one place.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ValidateAllStats {
    pub ok: u64,
    pub error: u64,
    /// Rows for which the per-row ClickHouse probe failed at the transport
    /// layer (ClickHouse went down mid-pass). The row was marked `unchecked`.
    pub probe_failed: u64,
}

/// Schema-validator component. Cheap to clone (both `DatabaseConnection` and
/// `insight_clickhouse::Client` are `Clone` over shared handles), so one
/// instance is shared between `AppState` (per-write hook) and the spawned
/// startup task.
#[derive(Clone)]
#[allow(dead_code)] // `debounce` is read by the per-write hook (wired in #525)
pub struct SchemaValidator {
    db: DatabaseConnection,
    ch: insight_clickhouse::Client,
    debounce: Duration,
    backoff: BackoffPolicy,
}

impl SchemaValidator {
    #[must_use]
    pub fn new(db: DatabaseConnection, ch: insight_clickhouse::Client) -> Self {
        Self {
            db,
            ch,
            debounce: DEFAULT_DEBOUNCE,
            backoff: BackoffPolicy::default_for_validator(),
        }
    }

    /// Override the debounce window. Test-only escape hatch.
    #[cfg(test)]
    #[must_use]
    pub fn with_debounce(mut self, d: Duration) -> Self {
        self.debounce = d;
        self
    }

    /// Per-write hook. Re-validates `metric_key` after a successful admin
    /// write, debouncing to skip the ClickHouse round-trip if
    /// `schema_checked_at` is within the last [`Self::debounce`] window
    /// (default 60 s).
    ///
    /// Never returns an HTTP error. The admin-crud caller logs and proceeds —
    /// validator failures are informational only (DESIGN §3.2).
    ///
    /// # Concurrency
    ///
    /// Two concurrent calls for the same `metric_key` can both pass
    /// `find_by_metric_key` + `should_debounce` before either persists, so the
    /// row is probed twice and the two `UPDATE`s race (last-write-wins on
    /// `checked_at`). Both writes are canonical and idempotent; the worst case
    /// is one extra ClickHouse round-trip and a redundant transition log if
    /// the two probes disagree. A row lock would add cross-replica coordination
    /// cost for a no-op race, so it is deliberately omitted.
    #[allow(dead_code)] // wired in #525; exposed as the per-write hook surface today
    pub async fn validate(&self, metric_key: &str) -> ValidationOutcome {
        let row = match find_by_metric_key(&self.db, metric_key).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                tracing::warn!(
                    metric_key = %metric_key,
                    "schema_validator.validate called with unknown metric_key (no catalog row)"
                );
                return ValidationOutcome::MetricUnknown;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    metric_key = %metric_key,
                    "schema_validator.validate failed to read catalog row"
                );
                return ValidationOutcome::ValidatorError;
            }
        };

        if self.should_debounce(&row) {
            tracing::debug!(
                metric_key = %metric_key,
                "schema_validator.validate debounced (schema_checked_at within window)"
            );
            return ValidationOutcome::DebouncedSkipped;
        }

        let now = Utc::now();
        let new_state = match self.probe_row(&row).await {
            Ok(state) => state,
            Err(_) => SchemaState::error(SchemaErrorCode::Unknown),
        };
        self.persist(&row, new_state, now).await;

        match new_state.status {
            SchemaStatus::Ok => ValidationOutcome::Ok,
            SchemaStatus::Error => {
                let code = new_state.error_code.unwrap_or(SchemaErrorCode::Unknown);
                ValidationOutcome::Error(code)
            }
            // `probe_row` returns Ok / Error only (parse failure → Error(Unknown),
            // network failure surfaces as Err and we already mapped that above).
            // Unchecked is reserved for the bulk degraded-mode mark in
            // `wait_for_clickhouse`, which the per-write path never enters.
            SchemaStatus::Unchecked => ValidationOutcome::ValidatorError,
        }
    }

    /// Startup pass. Iterates every catalog row and probes ClickHouse for each.
    ///
    /// Owns the exponential-backoff retry loop: if the first ClickHouse call
    /// fails (the warehouse is down at boot), marks every row as `unchecked`
    /// once, emits a degraded-mode summary log, and sleeps with backoff
    /// until ClickHouse comes back. Once reachable, walks the catalog via a
    /// DB cursor — one row in memory at a time — and persists per-row
    /// outcomes.
    ///
    /// Returns when the pass finishes. The caller (`main.rs`) doesn't await
    /// this — it's spawned as a `tokio::task`.
    pub async fn validate_all(&self) -> ValidateAllStats {
        // Empty-catalog gate: skip the CH wait + bulk-mark + cursor open if
        // the catalog has no rows. Without this, an empty catalog paired with
        // an unreachable ClickHouse would retry forever on a no-op mark.
        match count_all(&self.db).await {
            Ok(0) => {
                tracing::info!("schema_validator.validate_all: no catalog rows to probe");
                return ValidateAllStats::default();
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "schema_validator.validate_all failed to count catalog rows; \
                     aborting startup pass (read endpoint will continue serving \
                     existing schema_status values)"
                );
                return ValidateAllStats::default();
            }
        }

        // Wait for CH BEFORE opening the cursor so the MariaDB connection
        // backing the stream isn't held idle for the whole degraded-mode
        // window (which could be minutes — would starve the pool).
        self.wait_for_clickhouse().await;

        let mut stream = match stream_all(&self.db).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "schema_validator.validate_all failed to open catalog cursor; \
                     aborting startup pass"
                );
                return ValidateAllStats::default();
            }
        };

        let mut stats = ValidateAllStats::default();
        let mut total: u64 = 0;
        while let Some(row_result) = stream.next().await {
            let row = match row_result {
                Ok(r) => r,
                Err(e) => {
                    // Per-row decode/transport failure inside the cursor — keep
                    // going; the next row may be readable.
                    tracing::warn!(
                        error = %e,
                        "schema_validator: failed to read catalog row from cursor; skipping"
                    );
                    stats.probe_failed = stats.probe_failed.saturating_add(1);
                    continue;
                }
            };
            total = total.saturating_add(1);

            let now = Utc::now();
            let new_state = match self.probe_row(&row).await {
                Ok(probed) => probed,
                Err(ProbeError::Unreachable(_)) => {
                    // CH went down mid-pass — biconditional requires
                    // error_code=NULL on the wire when status != Error.
                    stats.probe_failed = stats.probe_failed.saturating_add(1);
                    SchemaState::unchecked()
                }
            };

            match new_state.status {
                SchemaStatus::Ok => stats.ok = stats.ok.saturating_add(1),
                SchemaStatus::Error => stats.error = stats.error.saturating_add(1),
                SchemaStatus::Unchecked => {}
            }

            self.persist(&row, new_state, now).await;
        }

        if stats.probe_failed > 0 {
            tracing::warn!(
                total,
                ok = stats.ok,
                error = stats.error,
                probe_failed = stats.probe_failed,
                "schema_validator.validate_all finished with some failures"
            );
        } else {
            tracing::info!(
                total,
                ok = stats.ok,
                error = stats.error,
                "schema_validator.validate_all finished"
            );
        }
        stats
    }

    fn should_debounce(&self, row: &CatalogRow) -> bool {
        let Some(last) = row.schema_checked_at else {
            return false;
        };
        let elapsed = Utc::now().signed_duration_since(last);
        let Ok(elapsed_std) = elapsed.to_std() else {
            // Negative duration (`checked_at` in the future) — clock skew or
            // a test rig. Don't debounce; re-validate.
            return false;
        };
        elapsed_std < self.debounce
    }

    /// Probe `row.metric_key`. Parse failure short-circuits to
    /// `Error(Unknown)` without a ClickHouse round-trip — the row's
    /// `metric_key` is malformed despite the DB CHECK, so we can't form a
    /// well-defined query.
    async fn probe_row(&self, row: &CatalogRow) -> Result<SchemaState, ProbeError> {
        let parsed = match parse_metric_key(&row.metric_key) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    metric_key = %row.metric_key,
                    error = %e,
                    "schema_validator: metric_key failed application-side parse \
                     (DB CHECK should have prevented this — investigate)"
                );
                return Ok(SchemaState::error(SchemaErrorCode::Unknown));
            }
        };

        let outcome = probe(
            &self.ch,
            self.ch.config().database.as_str(),
            parsed.table,
            parsed.column,
        )
        .await?;
        Ok(match outcome {
            ProbeOutcome::Ok => SchemaState::ok(),
            ProbeOutcome::Error(code) => SchemaState::error(code),
        })
    }

    /// Write the new state, emitting a transition log when the
    /// `(status, error_code)` pair has changed.
    async fn persist(&self, row: &CatalogRow, new_state: SchemaState, checked_at: DateTime<Utc>) {
        let from = row.current_state();
        if let Err(e) = update_schema_columns(&self.db, row.id, new_state, checked_at).await {
            tracing::error!(
                metric_key = %row.metric_key,
                error = %e,
                "schema_validator: failed to persist schema_* columns"
            );
            return;
        }

        if should_log_transition(from, new_state) {
            tracing::info!(
                event = "schema_validator.transition",
                metric_key = %row.metric_key,
                from_status = from.status.as_db_str(),
                to_status = new_state.status.as_db_str(),
                from_error_code = from.error_code.map_or("", SchemaErrorCode::as_db_str),
                to_error_code = new_state.error_code.map_or("", SchemaErrorCode::as_db_str),
                "schema_status transition"
            );
        }
    }

    /// Wait for ClickHouse to be reachable, in an exponential-backoff loop.
    /// On the FIRST consecutive failure, also flips every catalog row to
    /// `unchecked` so the read endpoint reflects degraded mode immediately.
    async fn wait_for_clickhouse(&self) {
        let mut attempt: u32 = 0;
        let mut marked_degraded = false;
        loop {
            match ping(&self.ch).await {
                Ok(()) => {
                    if marked_degraded {
                        tracing::info!(
                            event = "schema_validator.degraded_mode_recovered",
                            attempts = attempt,
                            "ClickHouse reachable; resuming startup validation pass"
                        );
                    }
                    return;
                }
                Err(e) => {
                    if marked_degraded {
                        tracing::debug!(
                            event = "schema_validator.retry_attempt",
                            attempt,
                            error = %e,
                            "ClickHouse still unreachable; backing off"
                        );
                    } else {
                        // Only declare degraded mode if we actually persisted
                        // the unchecked marks. Swallowing a `mark_all_unchecked`
                        // error and still flipping `marked_degraded = true` would
                        // strand the read endpoint serving stale `ok` values for
                        // the rest of the outage, since this branch is the only
                        // place that issues the bulk write.
                        let now = Utc::now();
                        match mark_all_unchecked(&self.db, now).await {
                            Ok(marked) => {
                                tracing::warn!(
                                    event = "schema_validator.degraded_mode_entered",
                                    marked_rows = marked,
                                    reason = SchemaErrorCode::ClickhouseUnreachable.as_db_str(),
                                    error = %e,
                                    "ClickHouse unreachable at startup; rows marked unchecked, \
                                     retrying with exponential backoff"
                                );
                                marked_degraded = true;
                            }
                            Err(db_err) => {
                                tracing::error!(
                                    event = "schema_validator.degraded_mode_mark_failed",
                                    db_error = %db_err,
                                    clickhouse_error = %e,
                                    "ClickHouse unreachable AND bulk-mark to MariaDB failed; \
                                     will retry both on next backoff tick"
                                );
                                // marked_degraded stays false so the next loop
                                // iteration retries the bulk mark.
                            }
                        }
                    }
                    let delay = self.backoff.next_delay(attempt);
                    sleep(delay).await;
                    attempt = attempt.saturating_add(1);
                }
            }
        }
    }
}

/// Lightweight liveness check: `SELECT 1` against ClickHouse. We don't reuse
/// the [`probe`] query here because we want to distinguish "ClickHouse is
/// down" from "ClickHouse is up but the analytics database doesn't exist".
async fn ping(ch: &insight_clickhouse::Client) -> Result<(), clickhouse::error::Error> {
    ch.query("SELECT 1").execute().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_debounce_is_60s() {
        assert_eq!(DEFAULT_DEBOUNCE, Duration::from_mins(1));
    }

    #[test]
    fn validation_outcome_variants_are_distinct() {
        // Pin the public enum shape so a refactor that collapses two variants
        // (e.g., merging `MetricUnknown` into `Error(Unknown)`) trips this test
        // and forces a deliberate decision — admin-crud distinguishes
        // "your metric doesn't exist" (`MetricUnknown`) from "the validator's
        // own DB failed" (`ValidatorError`) from "the metric exists but its
        // CH column is missing" (`Error(_)`), and surfaces them differently
        // to the operator.
        let v = [
            ValidationOutcome::Ok,
            ValidationOutcome::Error(SchemaErrorCode::ColumnNotFound),
            ValidationOutcome::DebouncedSkipped,
            ValidationOutcome::MetricUnknown,
            ValidationOutcome::ValidatorError,
        ];
        for (i, a) in v.iter().enumerate() {
            for (j, b) in v.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "variants {i} and {j} compared equal");
                }
            }
        }
    }

    #[test]
    fn should_debounce_returns_false_for_future_checked_at() {
        // Clock skew (or a test rig setting `schema_checked_at` ahead of `Utc::now()`)
        // produces a negative elapsed duration. `Duration::to_std()` rejects
        // negatives; the validator must NOT treat that as "within the window"
        // (which would silently freeze the row's schema state until clocks resync).
        let v = SchemaValidator::new(
            // Mock DatabaseConnection / ClickHouse client — we never touch them,
            // we only call the synchronous `should_debounce` predicate.
            sea_orm::DatabaseConnection::Disconnected,
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                "http://localhost:1",
                "x",
            )),
        );
        let row = crate::domain::schema_validator::repository::CatalogRow {
            id: uuid::Uuid::nil(),
            metric_key: "a.b".to_owned(),
            schema_status: "ok".to_owned(),
            schema_checked_at: Some(Utc::now() + chrono::Duration::hours(1)),
            schema_error_code: None,
        };
        assert!(!v.should_debounce(&row));
    }
}
