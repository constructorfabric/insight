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

const LEADER_KEY: &str = "{asm}:leader:idp_refresher";
const SESSION_LOCK_TTL_MS: u64 = 30_000;
const LOCK_HOLD_BUDGET_MS: u64 = SESSION_LOCK_TTL_MS - 5_000;
const STORE_RETRY_ATTEMPTS: u32 = 6;
const BACKOFF_BASE_SECONDS: u64 = 15;
const BACKOFF_MAX_SECONDS: u64 = 300;
const BATCH_LIMIT: usize = 512;

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

pub fn spawn(state: Arc<AppState>, cancel: CancellationToken) {
    if !state.cfg.idp.refresh_enabled {
        tracing::info!("idp refresher disabled by config (idp.refresh_enabled=false)");
        return;
    }
    tokio::spawn(run(state, cancel));
}

async fn run(state: Arc<AppState>, cancel: CancellationToken) {
    let tick = Duration::from_secs(state.cfg.idp.refresher_tick_seconds.max(1));
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
                return;
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

async fn refresh_one(state: &Arc<AppState>, metrics: &Metrics, session_id: &str) {
    let sessions = &state.sessions;
    let owner_token = match sessions
        .lock_session_refresh(session_id, SESSION_LOCK_TTL_MS)
        .await
    {
        Ok(Some(token)) => token,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, session_id, "refresh lock failed");
            return;
        }
    };

    match tokio::time::timeout(
        Duration::from_millis(LOCK_HOLD_BUDGET_MS),
        do_refresh(state, metrics, session_id),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, session_id, "idp refresh: store error"),
        Err(_) => tracing::error!(
            session_id,
            budget_ms = LOCK_HOLD_BUDGET_MS,
            "idp refresh exceeded the lock budget; aborting so the lock is not outlived"
        ),
    }
    if let Err(e) = sessions
        .unlock_session_refresh(session_id, &owner_token)
        .await
    {
        tracing::debug!(error = %e, session_id, "refresh unlock failed (lock TTL covers it)");
    }
}

// Pedantic length lint: one linear load → grant → outcome flow.
#[allow(clippy::too_many_lines)]
async fn do_refresh(
    state: &Arc<AppState>,
    metrics: &Metrics,
    session_id: &str,
) -> anyhow::Result<()> {
    let sessions: &SessionManager = &state.sessions;
    let now = now_secs();

    let Some(record) = sessions.load_session(session_id).await? else {
        sessions.unschedule_refresh(session_id).await?;
        return Ok(());
    };
    if record.expires_at <= now || record.absolute_expires_at <= now {
        sessions.unschedule_refresh(session_id).await?;
        return Ok(());
    }
    let Some(refresh_token) = record.idp_refresh_token.as_deref() else {
        sessions.unschedule_refresh(session_id).await?;
        return Ok(());
    };

    let Some(oidc) = state.oidc.for_stored_issuer(&record.idp_iss) else {
        metrics.record("transient");
        if let Some(failures) = sessions.bump_refresh_failures(session_id).await? {
            let retry_at = now + backoff_seconds(failures);
            sessions.reschedule_refresh(session_id, retry_at).await?;
            tracing::warn!(
                session_id,
                issuer = %record.idp_iss,
                failures,
                retry_at,
                "idp refresh: session issuer not in the configured map; backing off (never revoking)"
            );
        }
        return Ok(());
    };

    match oidc.refresh_grant(refresh_token).await {
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
            let mut stored = false;
            for attempt in 0..STORE_RETRY_ATTEMPTS {
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
                        sessions.unschedule_refresh(session_id).await.ok();
                        stored = true;
                        break;
                    }
                    Err(e) if attempt == STORE_RETRY_ATTEMPTS - 1 => {
                        tracing::error!(error = %e, session_id, "idp refresh: store failed after retries — the rotated token is lost, session will be logged out on the next attempt");
                    }
                    Err(e) => {
                        let backoff_ms = 200u64 << attempt;
                        tracing::warn!(error = %e, session_id, attempt, backoff_ms, "idp refresh store failed, retrying");
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    }
                }
            }
            if stored {
                tracing::debug!(session_id, next_due, "idp refresh ok");
            }
        }
        RefreshOutcome::InvalidGrant(detail) => {
            metrics.record("invalid_grant");
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

fn next_due_at(now: u64, expires_in: Option<u64>, margin: u64, jitter_window: u64) -> u64 {
    let base = match expires_in {
        Some(ttl) => now + ttl.saturating_sub(margin),
        None => now + margin.max(60),
    };
    let floor = now + (margin / 2).max(5);
    base.max(floor).saturating_add_signed(jitter(jitter_window))
}

fn backoff_seconds(failures: u64) -> u64 {
    #[allow(clippy::cast_possible_truncation)]
    let shift = failures.min(8) as u32;
    let base = (BACKOFF_BASE_SECONDS << shift).min(BACKOFF_MAX_SECONDS);
    base.saturating_add_signed(jitter(base / 4))
}

fn jitter(window: u64) -> i64 {
    if window == 0 {
        return 0;
    }
    let w = i64::try_from(window).unwrap_or(0);
    rand::RngExt::random_range(&mut rand::rng(), -w..=w)
}

fn now_secs() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_due_lands_margin_before_expiry_with_jitter() {
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
        for _ in 0..50 {
            let b0 = backoff_seconds(0);
            assert!((11..=19).contains(&b0), "{b0}");
            let b9 = backoff_seconds(9);
            assert!((225..=375).contains(&b9), "{b9}");
        }
    }
}
