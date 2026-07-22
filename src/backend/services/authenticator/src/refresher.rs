//! IdP background token refresher (PRD 5.12, G5 — the decided design).
//!
//! One leader (Redis lock, DD-BFF-09) polls the `asm:idp_refresh_due` ZSET
//! every tick and spawns per-session refresh tasks behind a semaphore
//! (`idp.refresh_concurrency` — politeness toward the customer IdP, not our
//! capacity). Each task takes a per-session lock (refresh tokens are
//! one-time-use at most IdPs; racing a rotation burns the grant), runs the
//! grant, and:
//!
//! - **success** → store the rotated token + new expiry, re-schedule with
//!   write-time jitter;
//! - **`invalid_grant`** (definitive: revoked / expired / user disabled) →
//!   revoke the session that owns the grant through the standard pipeline —
//!   the user's other sessions each hold their own grant and die at their own
//!   next refresh, so IdP-side deactivation converges within roughly one IdP
//!   access-token lifetime;
//! - **transient** (network, 5xx, 429) → exponential backoff and retry,
//!   NEVER revoke — a five-minute IdP blip must not log out the installation.
//!
//! Metrics: `idp_refresh_total{result}`, an `idp_refresh_consecutive_failures`
//! gauge (alert before the mass logout, not after), and
//! `idp_refresh_invalid_grant_total`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::api::AppState;
use crate::oidc::RefreshOutcome;
use crate::session::SessionManager;

const LEADER_KEY: &str = "asm:leader:idp_refresher";
/// Per-session lock TTL — covers one grant round-trip with generous margin.
const SESSION_LOCK_TTL_MS: u64 = 30_000;
/// Backoff for transient failures: `min(base << failures, max)` seconds.
const BACKOFF_BASE_SECONDS: u64 = 15;
const BACKOFF_MAX_SECONDS: u64 = 300;
/// Schedule entries drained per tick (leader-side bound; the semaphore is the
/// real throttle).
const BATCH_LIMIT: usize = 512;

/// Instruments + the consecutive-transient-failure gauge state.
struct Metrics {
    refresh_total: Counter<u64>,
    invalid_grant_total: Counter<u64>,
    consecutive_failures: Arc<AtomicU64>,
}

impl Metrics {
    fn new() -> Self {
        let meter = opentelemetry::global::meter("authenticator.idp_refresher");
        let consecutive_failures = Arc::new(AtomicU64::new(0));
        let gauge_state = consecutive_failures.clone();
        meter
            .u64_observable_gauge("idp_refresh_consecutive_failures")
            .with_description(
                "Consecutive transient IdP refresh failures (rises before a mass logout)",
            )
            .with_callback(move |observer| {
                observer.observe(gauge_state.load(Ordering::Relaxed), &[]);
            })
            .build();
        Self {
            refresh_total: meter
                .u64_counter("idp_refresh_total")
                .with_description("IdP background refresh outcomes")
                .build(),
            invalid_grant_total: meter
                .u64_counter("idp_refresh_invalid_grant_total")
                .with_description("Definitive IdP refusals (each kills the owning session)")
                .build(),
            consecutive_failures,
        }
    }

    fn record(&self, result: &'static str) {
        self.refresh_total
            .add(1, &[KeyValue::new("result", result)]);
        match result {
            "transient" => {
                self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
            }
            _ => self.consecutive_failures.store(0, Ordering::Relaxed),
        }
        if result == "invalid_grant" {
            self.invalid_grant_total.add(1, &[]);
        }
    }
}

/// Spawn the refresher loop; returns immediately (the gear's `start` must be
/// prompt). The loop runs on every pod but only the elected leader drains the
/// schedule. Cancellation stops the loop at the next tick.
pub fn spawn(state: Arc<AppState>, cancel: CancellationToken) {
    if !state.cfg.idp.refresh_enabled {
        tracing::info!("idp refresher disabled by config (idp.refresh_enabled=false)");
        return;
    }
    tokio::spawn(run(state, cancel));
}

async fn run(state: Arc<AppState>, cancel: CancellationToken) {
    let tick = Duration::from_secs(state.cfg.idp.refresher_tick_seconds.max(1));
    // Holder id: unique per process — pod name is not observable here, a UUID is.
    let holder = uuid::Uuid::now_v7().to_string();
    let semaphore = Arc::new(Semaphore::new(
        usize::try_from(state.cfg.idp.refresh_concurrency.max(1)).unwrap_or(128),
    ));
    let metrics = Arc::new(Metrics::new());
    tracing::info!(
        tick_seconds = tick.as_secs(),
        concurrency = state.cfg.idp.refresh_concurrency,
        "idp refresher started (leader-elected)"
    );

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                tracing::info!("idp refresher stopping");
                return;
            }
            () = tokio::time::sleep(tick) => {}
        }

        // Leader lock TTL = 3 ticks: a dead leader is replaced within ~2 ticks,
        // a live one renews every tick.
        let lease_ms = u64::try_from(tick.as_millis()).unwrap_or(5_000) * 3;
        let lead = state.sessions.try_lead(LEADER_KEY, &holder, lease_ms).await;
        match lead {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                tracing::warn!(error = %e, "idp refresher: leader election failed (skipping pass)");
                continue;
            }
        }

        let now = now_secs();
        let due = match state.sessions.due_refresh_sessions(now, BATCH_LIMIT).await {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!(error = %e, "idp refresher: schedule read failed");
                continue;
            }
        };
        for session_id in due {
            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                return; // semaphore closed — only on shutdown
            };
            let state = state.clone();
            let metrics = metrics.clone();
            tokio::spawn(async move {
                let _permit = permit;
                refresh_one(&state, &metrics, &session_id).await;
            });
        }
    }
}

/// Refresh a single due session under its rotation lock.
async fn refresh_one(state: &Arc<AppState>, metrics: &Metrics, session_id: &str) {
    let sessions = &state.sessions;
    match sessions
        .lock_session_refresh(session_id, SESSION_LOCK_TTL_MS)
        .await
    {
        Ok(true) => {}
        Ok(false) => return, // another worker is mid-rotation
        Err(e) => {
            tracing::warn!(error = %e, session_id, "refresh lock failed");
            return;
        }
    }

    let result = do_refresh(state, metrics, session_id).await;
    if let Err(e) = result {
        tracing::warn!(error = %e, session_id, "idp refresh: store error");
    }
    if let Err(e) = sessions.unlock_session_refresh(session_id).await {
        tracing::debug!(error = %e, session_id, "refresh unlock failed (lock TTL covers it)");
    }
}

// Linear per-session flow (load → grant → success/invalid_grant/transient),
// each arm with its own store + logging; splitting it would scatter the outcome
// handling without making it clearer.
#[allow(clippy::too_many_lines)]
async fn do_refresh(
    state: &Arc<AppState>,
    metrics: &Metrics,
    session_id: &str,
) -> anyhow::Result<()> {
    let sessions: &SessionManager = &state.sessions;
    let now = now_secs();

    // A vanished / expired session has nothing to refresh.
    let Some(record) = sessions.load_session(session_id).await? else {
        sessions.unschedule_refresh(session_id).await?;
        return Ok(());
    };
    if record.expires_at <= now || record.absolute_expires_at <= now {
        sessions.unschedule_refresh(session_id).await?;
        return Ok(());
    }
    let Some(refresh_token) = record.idp_refresh_token.as_deref() else {
        // Scheduled by mistake (no grant to refresh) — policy handled at login.
        sessions.unschedule_refresh(session_id).await?;
        return Ok(());
    };

    match state.oidc.refresh_grant(refresh_token).await {
        RefreshOutcome::Refreshed {
            new_refresh_token,
            expires_in,
        } => {
            metrics.record("ok");
            let access_expires_at = expires_in.map(|ttl| now + ttl);
            let next_due = next_due_at(
                now,
                expires_in,
                state.cfg.idp.refresh_safety_margin_seconds,
                state.cfg.idp.refresh_due_jitter_seconds,
            );
            // The IdP has ALREADY rotated the grant; the old token is spent. If
            // the store fails now, the next attempt would re-send the spent
            // token → invalid_grant → false logout (review M3). So retry the
            // store a few times before giving up; the guard returns false only
            // when the session was concurrently revoked (then just unschedule).
            let mut stored = false;
            for attempt in 0..3u32 {
                match sessions
                    .store_idp_refresh(
                        session_id,
                        new_refresh_token.as_deref(),
                        access_expires_at,
                        next_due,
                    )
                    .await
                {
                    Ok(true) => {
                        stored = true;
                        break;
                    }
                    Ok(false) => {
                        // Session revoked mid-flight — nothing to persist.
                        sessions.unschedule_refresh(session_id).await.ok();
                        stored = true;
                        break;
                    }
                    Err(e) if attempt == 2 => {
                        tracing::error!(error = %e, session_id, "idp refresh: store failed after retries — the rotated token is lost, session will be logged out on the next attempt");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, session_id, attempt, "idp refresh store failed, retrying");
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }
            }
            if stored {
                tracing::debug!(session_id, next_due, "idp refresh ok");
            }
        }
        RefreshOutcome::InvalidGrant(detail) => {
            metrics.record("invalid_grant");
            // Definitive verdict: the IdP no longer vouches for this grant.
            // Kill the owning session through the standard pipeline; the
            // user's other sessions die at their own next refresh.
            sessions.revoke_session(session_id).await?;
            tracing::warn!(
                target: "audit",
                event = "idp_refresh_invalid_grant",
                session_id,
                person_id = %record.person_id,
                detail = %detail,
                "IdP refused the refresh grant definitively: session revoked"
            );
            state.audit.emit(crate::audit::AuditEvent {
                action: "idp_refresh_invalid_grant",
                outcome: "success",
                tenant_id: record.tenant_id.clone(),
                actor_person_id: record.person_id.clone(),
                actor_ip: String::new(),
                actor_user_agent: String::new(),
                correlation_id: String::new(),
                resource_type: "session",
                resource_id: session_id.to_owned(),
                details: serde_json::json!({ "detail": detail }),
            });
        }
        RefreshOutcome::Transient(detail) => {
            metrics.record("transient");
            // A revoke mid-flight makes bump return None — don't resurrect a
            // counter-only zombie or reschedule a dead session.
            if let Some(failures) = sessions.bump_refresh_failures(session_id).await? {
                let retry_at = now + backoff_seconds(failures);
                sessions.reschedule_refresh(session_id, retry_at).await?;
                tracing::warn!(
                    session_id,
                    failures,
                    retry_at,
                    detail = %detail,
                    "idp refresh transient failure: backing off (never revoking)"
                );
            }
        }
    }
    Ok(())
}

/// The next schedule entry: `now + (expires_in − margin)`, jittered at write
/// (G5). An IdP that reports no lifetime is re-checked one margin from now.
/// Floored at `now + margin/2` (min 5 s) so an IdP issuing very short-lived
/// access tokens (`expires_in ≤ margin`) can't drive a refresh every tick
/// (review L3) — we'd hammer the IdP and never make progress.
fn next_due_at(now: u64, expires_in: Option<u64>, margin: u64, jitter_window: u64) -> u64 {
    let base = match expires_in {
        Some(ttl) => now + ttl.saturating_sub(margin),
        None => now + margin.max(60),
    };
    let floor = now + (margin / 2).max(5);
    base.max(floor).saturating_add_signed(jitter(jitter_window))
}

/// Exponential transient backoff: `min(15 << failures, 300)` seconds, jittered.
fn backoff_seconds(failures: u64) -> u64 {
    #[allow(clippy::cast_possible_truncation)]
    let shift = failures.min(8) as u32;
    let base = (BACKOFF_BASE_SECONDS << shift).min(BACKOFF_MAX_SECONDS);
    base.saturating_add_signed(jitter(base / 4))
}

/// Uniform jitter in `[-window, +window]` seconds.
fn jitter(window: u64) -> i64 {
    if window == 0 {
        return 0;
    }
    let w = i64::try_from(window).unwrap_or(0);
    rand::Rng::gen_range(&mut rand::thread_rng(), -w..=w)
}

fn now_secs() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_due_lands_margin_before_expiry_with_jitter() {
        // ttl 600, margin 60, jitter ±30 → due ∈ now + [510, 570].
        for _ in 0..100 {
            let due = next_due_at(1_000, Some(600), 60, 30);
            assert!((1_510..=1_570).contains(&due), "{due}");
        }
    }

    #[test]
    fn unknown_lifetime_rechecks_after_one_margin() {
        let due = next_due_at(1_000, None, 60, 0);
        assert_eq!(due, 1_060);
    }

    #[test]
    fn backoff_grows_and_caps() {
        // Deterministic core (jitter is ±base/4): failures 0 → ~15 s, large →
        // capped at ~300 s.
        for _ in 0..50 {
            let b0 = backoff_seconds(0);
            assert!((11..=19).contains(&b0), "{b0}");
            let b9 = backoff_seconds(9);
            assert!((225..=375).contains(&b9), "{b9}");
        }
    }
}
