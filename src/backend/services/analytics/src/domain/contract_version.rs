use std::time::Duration;

use clickhouse::Row;
use serde::Deserialize;

const PINNED_CONTRACT_VERSION: u32 = 1;

const SWEEP_INTERVAL: Duration = Duration::from_mins(5);
const STAMP_SQL: &str = "SELECT version FROM silver.contract_version LIMIT 1";

#[derive(Debug, Row, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
struct StampRow {
    version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StampState {
    Match,
    Mismatch(u32),
    Unreadable,
}

pub(crate) async fn run(ch: &insight_clickhouse::Client) {
    let mut ticks = tokio::time::interval(SWEEP_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut last: Option<StampState> = None;
    loop {
        ticks.tick().await;

        let read = ch.query(STAMP_SQL).fetch_one::<StampRow>().await;
        last = Some(observe(last, &read));
    }
}

fn observe(
    last: Option<StampState>,
    read: &Result<StampRow, clickhouse::error::Error>,
) -> StampState {
    let state = match read {
        Ok(row) if row.version == PINNED_CONTRACT_VERSION => StampState::Match,
        Ok(row) => StampState::Mismatch(row.version),
        Err(_) => StampState::Unreadable,
    };

    if last != Some(state) {
        report(state, read.as_ref().err());
    }
    state
}

fn report(state: StampState, error: Option<&clickhouse::error::Error>) {
    match state {
        StampState::Match => tracing::info!(
            version = PINNED_CONTRACT_VERSION,
            "contract version stamp matches the pinned contract surface"
        ),
        StampState::Mismatch(stamped) => tracing::error!(
            pinned = PINNED_CONTRACT_VERSION,
            stamped,
            "contract version stamp differs from the pinned contract surface"
        ),
        StampState::Unreadable => tracing::warn!(
            error = error.map(ToString::to_string),
            pinned = PINNED_CONTRACT_VERSION,
            "contract version stamp unreadable (silver.contract_version); \
             cannot confirm the contract surface this build was pinned to"
        ),
    }
}

#[cfg(test)]
mod tests {
    use clickhouse::test::{Mock, handlers, status};

    use super::*;

    fn row(version: u32) -> StampRow {
        StampRow { version }
    }

    fn unreadable() -> Result<StampRow, clickhouse::error::Error> {
        Err(clickhouse::error::Error::Custom("boom".to_owned()))
    }

    #[test]
    fn pinned_version_matches() {
        assert_eq!(
            observe(None, &Ok(row(PINNED_CONTRACT_VERSION))),
            StampState::Match
        );
    }

    #[test]
    fn drifted_stamp_is_a_mismatch_carrying_the_stamped_version() {
        assert_eq!(observe(None, &Ok(row(7))), StampState::Mismatch(7));
    }

    #[test]
    fn read_failure_is_unreadable() {
        assert_eq!(observe(None, &unreadable()), StampState::Unreadable);
    }

    #[test]
    fn unchanged_state_is_not_re_reported_but_still_tracked() {
        let first = observe(None, &Ok(row(7)));
        assert_eq!(observe(Some(first), &Ok(row(7))), StampState::Mismatch(7));
    }

    #[test]
    fn recovery_after_an_outage_is_a_state_change() {
        let outage = observe(None, &unreadable());
        assert_eq!(
            observe(Some(outage), &Ok(row(PINNED_CONTRACT_VERSION))),
            StampState::Match
        );
    }

    // One real sweep through the loop: the first interval tick fires
    // immediately and consumes the single mock handler; the second tick is
    // SWEEP_INTERVAL away, so the timeout cancels the loop before it can
    // issue a request the mock has no handler for.
    #[tokio::test]
    async fn run_reads_the_stamp_from_clickhouse() {
        let mock = Mock::new();
        mock.add(handlers::provide(vec![StampRow {
            version: PINNED_CONTRACT_VERSION,
        }]));

        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "silver"));

        let sweep = tokio::time::timeout(Duration::from_secs(2), run(&client)).await;
        assert!(sweep.is_err(), "run() must keep sweeping, not return");
    }

    #[tokio::test]
    async fn run_survives_a_failing_stamp_read() {
        let mock = Mock::new();
        mock.add(handlers::failure(status::INTERNAL_SERVER_ERROR));

        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "silver"));

        let sweep = tokio::time::timeout(Duration::from_secs(2), run(&client)).await;
        assert!(sweep.is_err(), "run() must keep sweeping, not return");
    }
}
