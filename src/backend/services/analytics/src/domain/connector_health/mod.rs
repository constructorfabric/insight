//! What the operator's connector-health page reports.
//!
//! The recorded facts arrive in three shapes — per-connector run and sync
//! summaries, storage observations, and the configured set — and merge here into
//! one row per connector. Everything in this module is a function over values;
//! [`read`] holds the queries.
//!
//! INVARIANT: this module reports what the ledger recorded and nothing else. It
//! derives no freshness verdict, because the declared thresholds have no runtime
//! source, and it labels no state — the page composes chips from these facts by
//! its own documented precedence.

pub(crate) mod read;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

/// How a sync was started, as the writers recorded it. Never re-derived here:
/// the read path sees only the warehouse, so a corroboration finding that was
/// not stored is a finding that is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Claim {
    /// A recorded run carries this sync's job identity.
    Claimed,
    /// Corroborated by exact identity that no run started it.
    OutOfBand,
    /// No claim and nothing left to corroborate against. Unknown provenance —
    /// never presented as manual.
    Unclaimed,
}

impl Claim {
    pub(crate) fn parse(raw: &str) -> Self {
        match raw {
            "claimed" => Self::Claimed,
            "out_of_band" => Self::OutOfBand,
            _ => Self::Unclaimed,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::OutOfBand => "out_of_band",
            Self::Unclaimed => "unclaimed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunFacts {
    pub(crate) status: String,
    /// The step the run reached; empty when it did not fail.
    pub(crate) step: String,
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) duration_ms: u64,
    /// Outcome of the transform step, when the run got that far.
    pub(crate) transform_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncFacts {
    pub(crate) claim: Claim,
    pub(crate) status: String,
    pub(crate) started_at: DateTime<Utc>,
    /// Absent until the mover's history has been swept: only that history knows
    /// how long a sync took and how much it moved, so reporting the pipeline
    /// row's zeros would state a measurement nobody made.
    pub(crate) duration_ms: Option<u64>,
    pub(crate) records_moved: Option<u64>,
    /// Rows the pipeline measured as delivered by this sync. Absent on a swept
    /// row: the measurement window had passed, and it is not reconstructed.
    pub(crate) rows_landed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StorageFacts {
    pub(crate) observed_at: DateTime<Utc>,
    pub(crate) streams: u16,
    pub(crate) streams_with_data: u16,
    pub(crate) rows_total: u64,
    pub(crate) bytes_on_disk: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamFacts {
    pub(crate) stream: String,
    pub(crate) rows_total: u64,
    pub(crate) bytes_on_disk: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectorHealth {
    pub(crate) connector: String,
    /// Present in the newest sealed configured snapshot.
    pub(crate) configured: bool,
    pub(crate) last_run: Option<RunFacts>,
    pub(crate) last_sync: Option<SyncFacts>,
    pub(crate) storage: Option<StorageFacts>,
    pub(crate) streams: Vec<StreamFacts>,
}

/// Why a connector sorts where it does.
///
/// Deliberately not serialised: the surface reports facts, and a rank is a
/// judgement. It exists so the operator reads what needs attention first, not
/// so the product asserts a verdict (spec FR-9, FR-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Attention {
    /// The mover reported moving records and storage gained none.
    Mismatched,
    /// The last run failed.
    RunFailed,
    /// The mover's own sync failed.
    SyncFailed,
    /// The sync succeeded and the transform did not.
    TransformFailed,
    /// Delivering, or at least not visibly broken.
    Quiet,
    /// Configured, never ran.
    NeverRan,
    /// A bronze schema with no configuration and no runs.
    NotConfigured,
}

fn failed(status: &str) -> bool {
    status == "failed" || status == "cancelled"
}

fn attention(health: &ConnectorHealth) -> Attention {
    if !health.configured && health.last_run.is_none() && health.last_sync.is_none() {
        return Attention::NotConfigured;
    }

    // Both halves must be known for a mismatch: an unrecorded counter or an
    // unmeasured delivery is a gap, and a gap is not a finding.
    if let Some(sync) = &health.last_sync
        && sync.records_moved.is_some_and(|moved| moved > 0)
        && sync.rows_landed == Some(0)
    {
        return Attention::Mismatched;
    }

    if let Some(run) = &health.last_run
        && failed(&run.status)
    {
        return Attention::RunFailed;
    }
    if let Some(sync) = &health.last_sync
        && failed(&sync.status)
    {
        return Attention::SyncFailed;
    }
    if let Some(run) = &health.last_run
        && run.transform_status.as_deref().is_some_and(failed)
    {
        return Attention::TransformFailed;
    }

    if health.last_run.is_none() && health.last_sync.is_none() {
        return Attention::NeverRan;
    }

    Attention::Quiet
}

/// Newest activity first within an attention band, so a stale broken connector
/// does not outrank one that broke this morning.
fn last_activity(health: &ConnectorHealth) -> Option<DateTime<Utc>> {
    let run = health.last_run.as_ref().map(|r| r.started_at);
    let sync = health.last_sync.as_ref().map(|s| s.started_at);
    run.max(sync)
}

/// The row every shape contributes to: a connector exists on this page as soon
/// as anything knows about it.
fn entry_for<'a>(
    by_connector: &'a mut BTreeMap<String, ConnectorHealth>,
    connector: &str,
) -> &'a mut ConnectorHealth {
    by_connector
        .entry(connector.to_owned())
        .or_insert_with(|| ConnectorHealth {
            connector: connector.to_owned(),
            configured: false,
            last_run: None,
            last_sync: None,
            storage: None,
            streams: Vec::new(),
        })
}

/// Merge the recorded shapes into one row per connector, ordered by attention.
///
/// A connector appears when anything knows about it: a run, a sync, a storage
/// observation, or the configured snapshot. That is what lets the page separate
/// *never configured* from *configured and never ran* (spec FR-15).
pub(crate) fn summarize(
    runs: Vec<(String, RunFacts)>,
    syncs: Vec<(String, SyncFacts)>,
    storage: Vec<(String, StorageFacts)>,
    streams: Vec<(String, StreamFacts)>,
    configured: &[String],
) -> Vec<ConnectorHealth> {
    let mut by_connector: BTreeMap<String, ConnectorHealth> = BTreeMap::new();

    for connector in configured {
        entry_for(&mut by_connector, connector).configured = true;
    }
    for (connector, facts) in runs {
        entry_for(&mut by_connector, &connector).last_run = Some(facts);
    }
    for (connector, facts) in syncs {
        entry_for(&mut by_connector, &connector).last_sync = Some(facts);
    }
    for (connector, facts) in storage {
        entry_for(&mut by_connector, &connector).storage = Some(facts);
    }
    for (connector, facts) in streams {
        entry_for(&mut by_connector, &connector).streams.push(facts);
    }

    let mut summaries: Vec<ConnectorHealth> = by_connector.into_values().collect();
    for summary in &mut summaries {
        summary.streams.sort_by(|a, b| {
            b.rows_total
                .cmp(&a.rows_total)
                .then(a.stream.cmp(&b.stream))
        });
    }

    summaries.sort_by(|a, b| {
        attention(a)
            .cmp(&attention(b))
            .then(last_activity(b).cmp(&last_activity(a)))
            .then(a.connector.cmp(&b.connector))
    });
    summaries
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap()
    }

    fn run(status: &str, transform: Option<&str>) -> RunFacts {
        RunFacts {
            status: status.to_owned(),
            step: String::new(),
            started_at: at(1),
            duration_ms: 1000,
            transform_status: transform.map(str::to_owned),
        }
    }

    fn sync(records_moved: u64, rows_landed: Option<u64>) -> SyncFacts {
        SyncFacts {
            claim: Claim::Claimed,
            status: "ok".to_owned(),
            started_at: at(1),
            duration_ms: Some(1000),
            records_moved: Some(records_moved),
            rows_landed,
        }
    }

    fn storage(streams: u16, with_data: u16) -> StorageFacts {
        StorageFacts {
            observed_at: at(1),
            streams,
            streams_with_data: with_data,
            rows_total: 10,
            bytes_on_disk: 100,
        }
    }

    fn names(summaries: &[ConnectorHealth]) -> Vec<&str> {
        summaries.iter().map(|s| s.connector.as_str()).collect()
    }

    #[test]
    fn a_connector_known_only_to_the_configured_snapshot_still_appears() {
        let summaries = summarize(vec![], vec![], vec![], vec![], &["alpha".to_owned()]);

        assert_eq!(names(&summaries), vec!["alpha"]);
        assert!(summaries[0].configured);
        assert!(summaries[0].last_run.is_none());
    }

    #[test]
    fn a_bronze_schema_with_no_configuration_and_no_runs_is_not_configured() {
        let summaries = summarize(
            vec![],
            vec![],
            vec![("ghost".to_owned(), storage(4, 0))],
            vec![],
            &[],
        );

        assert!(!summaries[0].configured);
        assert_eq!(attention(&summaries[0]), Attention::NotConfigured);
    }

    #[test]
    fn configured_and_never_ran_outranks_a_schema_nobody_configured() {
        let summaries = summarize(
            vec![],
            vec![],
            vec![("ghost".to_owned(), storage(4, 0))],
            vec![],
            &["waiting".to_owned()],
        );

        assert_eq!(names(&summaries), vec!["waiting", "ghost"]);
    }

    #[test]
    fn a_sync_that_moved_records_while_storage_gained_none_sorts_first() {
        let summaries = summarize(
            vec![("broken".to_owned(), run("ok", Some("ok")))],
            vec![
                ("broken".to_owned(), sync(12_400, Some(0))),
                ("fine".to_owned(), sync(400, Some(400))),
            ],
            vec![],
            vec![],
            &["broken".to_owned(), "fine".to_owned()],
        );

        assert_eq!(names(&summaries), vec!["broken", "fine"]);
        assert_eq!(attention(&summaries[0]), Attention::Mismatched);
    }

    #[test]
    fn a_failed_sync_is_not_reported_as_delivering() {
        let broken = ConnectorHealth {
            connector: "broken".to_owned(),
            configured: true,
            last_run: None,
            last_sync: Some(SyncFacts {
                status: "failed".to_owned(),
                ..sync(0, None)
            }),
            storage: None,
            streams: vec![],
        };

        assert_eq!(attention(&broken), Attention::SyncFailed);
    }

    #[test]
    fn an_unrecorded_counter_is_not_read_as_a_mismatch() {
        // Until the sweep covers the job the counters are unknown, and unknown
        // beside a measured zero is a gap rather than a misdelivery.
        let pending = ConnectorHealth {
            connector: "pending".to_owned(),
            configured: true,
            last_run: None,
            last_sync: Some(SyncFacts {
                records_moved: None,
                ..sync(0, Some(0))
            }),
            storage: None,
            streams: vec![],
        };

        assert_eq!(attention(&pending), Attention::Quiet);
    }

    #[test]
    fn a_sync_with_no_measurement_is_not_read_as_a_mismatch() {
        let swept = ConnectorHealth {
            connector: "swept".to_owned(),
            configured: true,
            last_run: None,
            last_sync: Some(sync(12_400, None)),
            storage: None,
            streams: vec![],
        };

        assert_eq!(attention(&swept), Attention::Quiet, "absent is not zero");
    }

    #[test]
    fn a_failed_transform_after_a_successful_sync_is_its_own_state() {
        let stalled = ConnectorHealth {
            connector: "stalled".to_owned(),
            configured: true,
            last_run: Some(run("ok", Some("failed"))),
            last_sync: Some(sync(400, Some(400))),
            storage: None,
            streams: vec![],
        };

        assert_eq!(attention(&stalled), Attention::TransformFailed);
    }

    #[test]
    fn a_failed_run_outranks_a_stalled_transform() {
        let summaries = summarize(
            vec![
                ("failing".to_owned(), run("failed", None)),
                ("stalled".to_owned(), run("ok", Some("failed"))),
            ],
            vec![],
            vec![],
            vec![],
            &[],
        );

        assert_eq!(names(&summaries), vec!["failing", "stalled"]);
    }

    #[test]
    fn within_a_band_the_most_recent_activity_comes_first() {
        let older = RunFacts {
            started_at: at(1),
            ..run("failed", None)
        };
        let newer = RunFacts {
            started_at: at(9),
            ..run("failed", None)
        };

        let summaries = summarize(
            vec![("stale".to_owned(), older), ("fresh".to_owned(), newer)],
            vec![],
            vec![],
            vec![],
            &[],
        );

        assert_eq!(names(&summaries), vec!["fresh", "stale"]);
    }

    #[test]
    fn streams_are_ordered_by_volume_so_an_empty_one_reads_as_the_exception() {
        let streams = vec![
            (
                "alpha".to_owned(),
                StreamFacts {
                    stream: "empty".to_owned(),
                    rows_total: 0,
                    bytes_on_disk: 0,
                },
            ),
            (
                "alpha".to_owned(),
                StreamFacts {
                    stream: "big".to_owned(),
                    rows_total: 900,
                    bytes_on_disk: 9,
                },
            ),
        ];

        let summaries = summarize(vec![], vec![], vec![], streams, &[]);

        let ordered: Vec<&str> = summaries[0]
            .streams
            .iter()
            .map(|s| s.stream.as_str())
            .collect();
        assert_eq!(ordered, vec!["big", "empty"]);
    }

    #[test]
    fn an_unrecognised_claim_reads_as_unknown_provenance_not_as_manual() {
        assert_eq!(Claim::parse("claimed"), Claim::Claimed);
        assert_eq!(Claim::parse("out_of_band"), Claim::OutOfBand);
        assert_eq!(Claim::parse(""), Claim::Unclaimed);
        assert_eq!(Claim::parse("something-new"), Claim::Unclaimed);
    }
}
