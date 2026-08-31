//! Domain instruments for the persons-seed pipeline. The `opentelemetry` major
//! here must match the toolkit's (the workspace pin), or these instruments
//! record into a no-op global. The seed/sync CLI installs the provider via
//! [`crate::infra::telemetry`].

use std::sync::OnceLock;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, Meter};

use crate::domain::seed_service::SeedSummary;

const METER_NAME: &str = "identity-resolution";

#[derive(Debug, Clone, Copy)]
pub(crate) enum ResolutionOutcome {
    Resolved,
    Ambiguous,
    Unmatched,
}

impl ResolutionOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::Ambiguous => "ambiguous",
            Self::Unmatched => "unmatched",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RunOutcome {
    Success,
    Error,
}

impl RunOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
        }
    }
}

// INVARIANT: one variant per SeedStore method — a closed set, so the `query`
// label on `identity_resolution.db.query.duration` stays bounded.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DbQuery {
    KnownAccountBindings,
    LatestEmailToPerson,
    Apply,
}

impl DbQuery {
    const fn as_str(self) -> &'static str {
        match self {
            Self::KnownAccountBindings => "known_account_bindings",
            Self::LatestEmailToPerson => "latest_email_to_person",
            Self::Apply => "apply",
        }
    }
}

struct Instruments {
    accounts: Counter<u64>,
    seed_duration: Histogram<f64>,
    db_query_duration: Histogram<f64>,
}

fn instruments() -> &'static Instruments {
    static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();
    INSTRUMENTS.get_or_init(|| {
        let meter: Meter = opentelemetry::global::meter(METER_NAME);
        Instruments {
            accounts: meter
                .u64_counter("identity_resolution.accounts")
                .with_description(
                    "Accounts a persons-seed run classified, by outcome. A sustained rise \
                     in ambiguous or unmatched relative to resolved means connectors are \
                     emitting accounts identity cannot bind — inspect the review queue.",
                )
                .build(),
            seed_duration: meter
                .f64_histogram("identity_resolution.seed.duration")
                .with_unit("s")
                .with_description("Wall time of one persons-seed run, by terminal outcome.")
                .build(),
            db_query_duration: meter
                .f64_histogram("identity_resolution.db.query.duration")
                .with_unit("s")
                .with_description("Wall time per seed-pipeline database operation, by query.")
                .build(),
        }
    })
}

// Partition the per-account dispositions into the three outcome labels:
// resolved bound a person; ambiguous hit a contested address; unmatched was
// left unbound for a structural reason.
pub(crate) fn record_seed_outcomes(summary: &SeedSummary) {
    let resolved = summary.reused_known
        + summary.linked_by_email
        + summary.minted
        + summary.minted_from_roster;
    let ambiguous = summary.skipped_contested_email;
    let unmatched = summary.skipped_closed
        + summary.skipped_no_email
        + summary.skipped_no_source_id
        + summary.skipped_excluded;

    add_outcome(ResolutionOutcome::Resolved, resolved);
    add_outcome(ResolutionOutcome::Ambiguous, ambiguous);
    add_outcome(ResolutionOutcome::Unmatched, unmatched);
}

fn add_outcome(outcome: ResolutionOutcome, count: usize) {
    instruments().accounts.add(
        u64::try_from(count).unwrap_or(u64::MAX),
        &[KeyValue::new("outcome", outcome.as_str())],
    );
}

pub(crate) fn record_seed_run(outcome: RunOutcome, elapsed: Duration) {
    instruments().seed_duration.record(
        elapsed.as_secs_f64(),
        &[KeyValue::new("outcome", outcome.as_str())],
    );
}

pub(crate) fn record_db_query(query: DbQuery, elapsed: Duration) {
    instruments().db_query_duration.record(
        elapsed.as_secs_f64(),
        &[KeyValue::new("query", query.as_str())],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_fixed_vocabularies() {
        assert_eq!(ResolutionOutcome::Resolved.as_str(), "resolved");
        assert_eq!(ResolutionOutcome::Ambiguous.as_str(), "ambiguous");
        assert_eq!(ResolutionOutcome::Unmatched.as_str(), "unmatched");
        assert_eq!(RunOutcome::Success.as_str(), "success");
        assert_eq!(RunOutcome::Error.as_str(), "error");
        assert_eq!(
            DbQuery::KnownAccountBindings.as_str(),
            "known_account_bindings"
        );
        assert_eq!(
            DbQuery::LatestEmailToPerson.as_str(),
            "latest_email_to_person"
        );
        assert_eq!(DbQuery::Apply.as_str(), "apply");
    }

    #[test]
    fn recording_without_a_provider_is_a_no_op_not_a_panic() {
        record_seed_outcomes(&SeedSummary::default());
        record_seed_run(RunOutcome::Success, Duration::from_millis(5));
        record_db_query(DbQuery::Apply, Duration::from_millis(5));
    }
}
