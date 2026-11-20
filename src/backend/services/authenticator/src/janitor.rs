use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::api::AppState;

const LEADER_KEY: &str = "{asm}:leader:janitor";
const ORPHAN_GRACE_SECONDS: u64 = 600;

pub fn spawn(state: Arc<AppState>, cancel: CancellationToken) {
    tokio::spawn(run(state, cancel));
}

async fn run(state: Arc<AppState>, cancel: CancellationToken) {
    let tick = Duration::from_secs(state.cfg.janitor_interval_seconds.max(1));
    let holder = uuid::Uuid::now_v7().to_string();

    let meter = opentelemetry::global::meter("authenticator.janitor");
    let removed_total = meter
        .u64_counter("auth_janitor_removed_total")
        .with_description("Expired index members / schedule orphans trimmed")
        .build();
    let backlog_state = Arc::new(AtomicU64::new(0));
    let gauge_state = backlog_state.clone();
    meter
        .u64_observable_gauge("auth_janitor_backlog_size")
        .with_description("Expired-but-untrimmed index members seen by the last pass")
        .with_callback(move |observer| {
            observer.observe(gauge_state.load(Ordering::Relaxed), &[]);
        })
        .build();

    tracing::info!(
        interval_seconds = tick.as_secs(),
        "janitor started (leader-elected)"
    );

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                tracing::info!("janitor stopping");
                return;
            }
            () = tokio::time::sleep(tick) => {}
        }

        let lease_ms = u64::try_from(tick.as_millis()).unwrap_or(30_000) * 3;
        match state.sessions.try_lead(LEADER_KEY, &holder, lease_ms).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                tracing::warn!(error = %e, "janitor: leader election failed (skipping pass)");
                continue;
            }
        }

        let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
        match state.sessions.janitor_pass(now, ORPHAN_GRACE_SECONDS).await {
            Ok((removed, backlog)) => {
                removed_total.add(removed, &[]);
                backlog_state.store(backlog, Ordering::Relaxed);
                if removed > 0 {
                    tracing::debug!(removed, backlog, "janitor pass trimmed expired members");
                }
            }
            Err(e) => tracing::warn!(error = %e, "janitor pass failed"),
        }
    }
}
