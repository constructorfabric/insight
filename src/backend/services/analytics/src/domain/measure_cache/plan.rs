//! What one refresh tick covers and the statements it issues.
//!
//! Partitions are `(measure, definition version, month)`, so every statement
//! here names a partition by that triple and touches nothing else.

use chrono::{Datelike, NaiveDate, TimeDelta};

use crate::domain::compiler::cache_build::{CACHE_RELATION, STAGING_RELATION};

/// The span a refresh rebuilds. Days that fall out of it keep the rows an
/// earlier run left, which is what makes a settled period cheap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotWindow {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

pub fn hot_window(today: NaiveDate, hot_window_days: u32) -> HotWindow {
    let from = today
        .checked_sub_signed(TimeDelta::days(i64::from(hot_window_days)))
        .unwrap_or(NaiveDate::MIN);

    HotWindow { from, to: today }
}

/// The partition months the window spans, as `toYYYYMM` reads them.
pub fn months(window: HotWindow) -> Vec<u32> {
    let last = (window.to.year(), window.to.month());
    let (mut year, mut month) = (window.from.year(), window.from.month());

    let mut months = Vec::new();
    while (year, month) <= last {
        months.push(year_month(year, month));
        if month == 12 {
            year += 1;
            month = 1;
        } else {
            month += 1;
        }
    }

    months
}

fn year_month(year: i32, month: u32) -> u32 {
    year.unsigned_abs() * 100 + month
}

/// Clears the slot a build is about to fill, so a crashed run's leftovers can
/// never be swapped in as if they were this run's work.
pub fn clear_staging_partition_sql() -> String {
    format!("ALTER TABLE {STAGING_RELATION} DROP PARTITION (?, ?, ?)")
}

/// The swap: one month of one measure at one version, replaced whole. A reader
/// sees either the previous partition or the new one, never a half-built mix.
pub fn swap_partition_sql() -> String {
    format!("ALTER TABLE {CACHE_RELATION} REPLACE PARTITION (?, ?, ?) FROM {STAGING_RELATION}")
}

pub fn drop_cache_partition_sql() -> String {
    format!("ALTER TABLE {CACHE_RELATION} DROP PARTITION (?, ?, ?)")
}

/// The partitions a superseded definition still occupies. Read after the
/// current version has landed, so a failed build never drops what still serves.
pub fn superseded_partitions_sql() -> String {
    format!(
        "SELECT DISTINCT definition_version AS version, toYYYYMM(metric_date) AS month \
         FROM {CACHE_RELATION} WHERE measure_key = ? AND definition_version < ?"
    )
}

/// The partitions one version currently occupies, for a rebuild that cannot
/// leave a settled month holding rows of the shape it is replacing.
pub fn version_partitions_sql() -> String {
    format!(
        "SELECT DISTINCT definition_version AS version, toYYYYMM(metric_date) AS month \
         FROM {CACHE_RELATION} WHERE measure_key = ? AND definition_version = ?"
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    #[test]
    fn the_hot_window_ends_today_and_reaches_back_the_policy_days() {
        let window = hot_window(date(2026, 3, 10), 35);

        assert_eq!(window.to, date(2026, 3, 10));
        assert_eq!(window.from, date(2026, 2, 3));
    }

    #[test]
    fn a_window_covers_every_month_it_touches() {
        let cases = [
            (hot_window(date(2026, 3, 10), 35), vec![202_602, 202_603]),
            (hot_window(date(2026, 3, 10), 1), vec![202_603]),
            (
                hot_window(date(2026, 1, 15), 60),
                vec![202_511, 202_512, 202_601],
            ),
        ];

        for (window, expected) in cases {
            assert_eq!(months(window), expected, "window {window:?}");
        }
    }

    #[test]
    fn a_window_whose_end_precedes_its_start_covers_nothing() {
        let inverted = HotWindow {
            from: date(2026, 5, 1),
            to: date(2026, 4, 1),
        };

        assert!(months(inverted).is_empty());
    }

    #[test]
    fn every_statement_names_one_partition_by_measure_version_and_month() {
        for sql in [
            clear_staging_partition_sql(),
            swap_partition_sql(),
            drop_cache_partition_sql(),
        ] {
            assert!(sql.contains("PARTITION (?, ?, ?)"), "{sql}");
            assert_eq!(sql.matches('?').count(), 3, "{sql}");
        }
    }

    #[test]
    fn the_swap_moves_a_partition_from_staging_into_the_served_relation() {
        assert_eq!(
            swap_partition_sql(),
            "ALTER TABLE insight.semantic_measure_cache REPLACE PARTITION (?, ?, ?) \
             FROM insight.semantic_measure_cache_staging"
        );
    }

    #[test]
    fn the_superseded_read_asks_only_about_versions_below_the_current_one() {
        let sql = superseded_partitions_sql();

        assert!(sql.contains("definition_version < ?"), "{sql}");
        assert_eq!(sql.matches('?').count(), 2, "{sql}");
    }

    #[test]
    fn the_rebuild_read_asks_only_about_the_version_it_is_replacing() {
        let sql = version_partitions_sql();

        assert!(sql.contains("definition_version = ?"), "{sql}");
        assert_eq!(sql.matches('?').count(), 2, "{sql}");
    }
}
