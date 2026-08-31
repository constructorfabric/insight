//! Whether a read may answer from a measure's materialized rows. One store read
//! carries policy, coverage and the versions the definitions stand at; the
//! decision itself is per measure and per window, and anything degraded reads
//! the dataset instead.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement, Value};

use crate::domain::compiler::cache_build::CacheRowKind;

/// Why one measure is read from its dataset rather than from the cache. Logged
/// at debug; the answer states only that it was computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveReason {
    /// The read side is switched off for this deployment.
    ReadDisabled,
    /// The store carries no enabled definition of the measure.
    Undefined,
    /// No policy row, or one an operator disabled.
    PolicyOff,
    /// The store cannot be read, so nothing about the cache is known.
    StoreUnreadable,
    /// Nothing has been built for the measure at any version.
    NoCoverage,
    /// What was built stands at an older definition than the store holds.
    VersionStale,
    /// The window reaches outside the days the cache covers.
    RangeUncovered,
    /// What was built is a different row shape than the one this release folds
    /// the measure into, so re-folding it would answer a different question.
    KindChanged,
}

/// What one measure's read was decided to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheDecision {
    Cached { definition_version: u32 },
    Live { reason: LiveReason },
}

/// One measure's policy and coverage as the store holds them, before a window
/// decides anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageRow {
    pub measure_key: String,
    pub current_version: u32,
    pub enabled: bool,
    /// A version something was built at, absent when nothing was.
    pub cached_version: Option<u32>,
    /// The row shape that build wrote; absent for a spelling this release does
    /// not know, which is a shape no read may guess a fold for.
    pub cached_kind: Option<CacheRowKind>,
    pub covered_from: Option<NaiveDate>,
    pub covered_to: Option<NaiveDate>,
}

/// The store's answer for the measures one request names, read once and asked
/// per window afterwards.
#[derive(Debug)]
pub struct ReadGate {
    rows: Vec<CoverageRow>,
    /// What every measure decides to when the read side is off or unreadable.
    refused: Option<LiveReason>,
}

const COVERAGE_SQL: &str = "SELECT \
        m.measure_key AS measure_key, \
        m.definition_version AS current_version, \
        CAST(COALESCE(p.enabled, FALSE) AS SIGNED) AS enabled, \
        c.definition_version AS cached_version, \
        c.row_kind AS cached_kind, \
        c.covered_from AS covered_from, \
        c.covered_to AS covered_to \
     FROM semantic_measures m \
     LEFT JOIN semantic_cache_policies p ON p.measure_key = m.measure_key \
     LEFT JOIN semantic_cache_coverage c ON c.measure_key = m.measure_key \
     WHERE m.tenant_id IS NULL AND m.is_enabled = TRUE AND m.measure_key IN (";

impl ReadGate {
    /// INVARIANT: a store that cannot be read degrades to the dataset rather
    /// than refusing the request — slower is never wrong.
    pub async fn read(
        db: &DatabaseConnection,
        read_enabled: bool,
        measure_keys: &BTreeSet<String>,
    ) -> Self {
        if !read_enabled {
            return Self::refusing(LiveReason::ReadDisabled);
        }
        if measure_keys.is_empty() {
            return Self {
                rows: Vec::new(),
                refused: None,
            };
        }

        match coverage(db, measure_keys).await {
            Ok(rows) => Self {
                rows,
                refused: None,
            },
            Err(error) => {
                tracing::warn!(%error, "the measure cache policy store is unreadable; every read compiles over its dataset");
                Self::refusing(LiveReason::StoreUnreadable)
            }
        }
    }

    /// Every measure decided live, for a caller that pins the dataset read.
    #[cfg(test)]
    #[must_use]
    pub fn all_live() -> Self {
        Self::refusing(LiveReason::ReadDisabled)
    }

    /// The gate over rows a caller already holds, rather than over a store.
    #[cfg(test)]
    #[must_use]
    pub fn over(rows: Vec<CoverageRow>) -> Self {
        Self {
            rows,
            refused: None,
        }
    }

    fn refusing(reason: LiveReason) -> Self {
        Self {
            rows: Vec::new(),
            refused: Some(reason),
        }
    }

    /// The decision for each named measure over one window. `expected` states
    /// the row shape this release folds each measure into, which the coverage
    /// must attest before its rows can answer.
    #[must_use]
    pub fn decide(
        &self,
        expected: &BTreeMap<String, CacheRowKind>,
        from: NaiveDate,
        to: NaiveDate,
    ) -> BTreeMap<String, CacheDecision> {
        expected
            .iter()
            .map(|(key, kind)| {
                let decision = match self.refused {
                    Some(reason) => CacheDecision::Live { reason },
                    None => decide_one(&self.rows, key, *kind, from, to),
                };
                if let CacheDecision::Live { reason } = decision {
                    tracing::debug!(measure = %key, ?reason, "a measure is read from its dataset");
                }
                (key.clone(), decision)
            })
            .collect()
    }
}

/// INVARIANT: the coverage rows of one measure are one per version, so the
/// current version's row is the only one that can answer.
fn decide_one(
    rows: &[CoverageRow],
    measure_key: &str,
    expected: CacheRowKind,
    from: NaiveDate,
    to: NaiveDate,
) -> CacheDecision {
    let mut named = rows.iter().filter(|row| row.measure_key == measure_key);
    let Some(first) = named.next() else {
        return CacheDecision::Live {
            reason: LiveReason::Undefined,
        };
    };
    if !first.enabled {
        return CacheDecision::Live {
            reason: LiveReason::PolicyOff,
        };
    }

    let version = first.current_version;
    let mut any_coverage = false;
    for row in std::iter::once(first).chain(named) {
        let (Some(cached), Some(covered_from), Some(covered_to)) =
            (row.cached_version, row.covered_from, row.covered_to)
        else {
            continue;
        };
        any_coverage = true;
        if cached != version {
            continue;
        }
        if row.cached_kind != Some(expected) {
            return CacheDecision::Live {
                reason: LiveReason::KindChanged,
            };
        }
        if covered_from > from || covered_to < to {
            return CacheDecision::Live {
                reason: LiveReason::RangeUncovered,
            };
        }
        return CacheDecision::Cached {
            definition_version: version,
        };
    }

    CacheDecision::Live {
        reason: if any_coverage {
            LiveReason::VersionStale
        } else {
            LiveReason::NoCoverage
        },
    }
}

async fn coverage(
    db: &DatabaseConnection,
    measure_keys: &BTreeSet<String>,
) -> Result<Vec<CoverageRow>, DbErr> {
    let placeholders = vec!["?"; measure_keys.len()].join(", ");
    let sql = format!("{COVERAGE_SQL}{placeholders})");
    let values: Vec<Value> = measure_keys.iter().map(Value::from).collect();

    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            sql,
            values,
        ))
        .await?;

    rows.into_iter()
        .map(|row| {
            Ok(CoverageRow {
                measure_key: row.try_get("", "measure_key")?,
                current_version: row.try_get::<i32>("", "current_version")?.unsigned_abs(),
                enabled: row.try_get::<i64>("", "enabled")? != 0,
                cached_version: row
                    .try_get::<Option<i32>>("", "cached_version")?
                    .map(i32::unsigned_abs),
                cached_kind: row
                    .try_get::<Option<String>>("", "cached_kind")?
                    .as_deref()
                    .and_then(CacheRowKind::from_db),
                covered_from: row.try_get("", "covered_from")?,
                covered_to: row.try_get("", "covered_to")?,
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    fn covered(cached_version: u32, from: (i32, u32, u32), to: (i32, u32, u32)) -> CoverageRow {
        CoverageRow {
            measure_key: "commits".to_owned(),
            current_version: 4,
            enabled: true,
            cached_version: Some(cached_version),
            cached_kind: Some(CacheRowKind::Aggregate),
            covered_from: Some(date(from.0, from.1, from.2)),
            covered_to: Some(date(to.0, to.1, to.2)),
        }
    }

    /// The shape this release folds `commits` into, which its coverage must
    /// attest.
    fn expected() -> BTreeMap<String, CacheRowKind> {
        BTreeMap::from([("commits".to_owned(), CacheRowKind::Aggregate)])
    }

    fn keys() -> BTreeSet<String> {
        BTreeSet::from(["commits".to_owned()])
    }

    fn decide(rows: Vec<CoverageRow>) -> CacheDecision {
        let gate = ReadGate::over(rows);
        gate.decide(&expected(), date(2026, 3, 1), date(2026, 3, 31))["commits"]
    }

    #[test]
    fn a_measure_covered_at_its_current_version_over_the_whole_window_reads_the_cache() {
        assert_eq!(
            decide(vec![covered(4, (2026, 1, 1), (2026, 4, 30))]),
            CacheDecision::Cached {
                definition_version: 4
            }
        );
    }

    #[test]
    fn a_window_matching_the_covered_span_exactly_still_reads_the_cache() {
        assert_eq!(
            decide(vec![covered(4, (2026, 3, 1), (2026, 3, 31))]),
            CacheDecision::Cached {
                definition_version: 4
            }
        );
    }

    #[test]
    fn every_degraded_state_reads_the_dataset_and_names_why() {
        let cases = [
            (Vec::new(), LiveReason::Undefined),
            (
                vec![CoverageRow {
                    enabled: false,
                    ..covered(4, (2026, 1, 1), (2026, 4, 30))
                }],
                LiveReason::PolicyOff,
            ),
            (
                vec![CoverageRow {
                    cached_version: None,
                    cached_kind: None,
                    covered_from: None,
                    covered_to: None,
                    ..covered(4, (2026, 1, 1), (2026, 4, 30))
                }],
                LiveReason::NoCoverage,
            ),
            (
                vec![CoverageRow {
                    cached_kind: Some(CacheRowKind::Event),
                    ..covered(4, (2026, 1, 1), (2026, 4, 30))
                }],
                LiveReason::KindChanged,
            ),
            (
                vec![CoverageRow {
                    cached_kind: None,
                    ..covered(4, (2026, 1, 1), (2026, 4, 30))
                }],
                LiveReason::KindChanged,
            ),
            (
                vec![covered(3, (2026, 1, 1), (2026, 4, 30))],
                LiveReason::VersionStale,
            ),
            (
                vec![covered(4, (2026, 3, 5), (2026, 4, 30))],
                LiveReason::RangeUncovered,
            ),
            (
                vec![covered(4, (2026, 1, 1), (2026, 3, 20))],
                LiveReason::RangeUncovered,
            ),
        ];

        for (rows, reason) in cases {
            assert_eq!(decide(rows), CacheDecision::Live { reason }, "{reason:?}");
        }
    }

    #[test]
    fn a_measure_covered_at_one_version_among_several_reads_the_one_the_store_stands_at() {
        assert_eq!(
            decide(vec![
                covered(3, (2026, 1, 1), (2026, 4, 30)),
                covered(4, (2026, 2, 1), (2026, 4, 30)),
            ]),
            CacheDecision::Cached {
                definition_version: 4
            }
        );
    }

    #[test]
    fn a_measure_the_read_did_not_ask_about_is_absent_from_the_decision() {
        let gate = ReadGate::over(vec![covered(4, (2026, 1, 1), (2026, 4, 30))]);

        let decided = gate.decide(
            &BTreeMap::from([("prs_merged".to_owned(), CacheRowKind::Aggregate)]),
            date(2026, 3, 1),
            date(2026, 3, 31),
        );

        assert_eq!(
            decided,
            BTreeMap::from([(
                "prs_merged".to_owned(),
                CacheDecision::Live {
                    reason: LiveReason::Undefined
                }
            )])
        );
    }

    #[tokio::test]
    async fn the_kill_switch_decides_every_measure_live_without_asking_the_store() {
        let gate = ReadGate::read(&DatabaseConnection::default(), false, &keys()).await;

        assert_eq!(
            gate.decide(&expected(), date(2026, 3, 1), date(2026, 3, 31))["commits"],
            CacheDecision::Live {
                reason: LiveReason::ReadDisabled
            }
        );
    }

    #[test]
    fn a_store_that_cannot_answer_leaves_every_measure_live() {
        let gate = ReadGate::refusing(LiveReason::StoreUnreadable);

        assert_eq!(
            gate.decide(&expected(), date(2026, 3, 1), date(2026, 3, 31))["commits"],
            CacheDecision::Live {
                reason: LiveReason::StoreUnreadable
            }
        );
    }

    #[tokio::test]
    async fn a_request_naming_no_measure_asks_the_store_nothing() {
        let gate = ReadGate::read(&DatabaseConnection::default(), true, &BTreeSet::new()).await;

        assert!(
            gate.decide(&BTreeMap::new(), date(2026, 3, 1), date(2026, 3, 31))
                .is_empty()
        );
    }
}
