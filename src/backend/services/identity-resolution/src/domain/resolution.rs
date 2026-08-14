//! Operator identity corrections: the pure core.
//!
//! A correction is an appended binding observation in `persons` — never an
//! update. This module decides *whether* a correction has anything to append
//! and *what* the appended rows look like; the repository performs the write.

use sea_orm::prelude::DateTime;
use uuid::Uuid;

use super::observation_slot::{self, SlotAllocator};
use super::seed::{KnownBinding, SourceAccountKey};

/// The reserved person meaning "not a human". Bots, CI and service accounts
/// bind here; every consumer treats it as no person (NULL in analytics, not
/// served by the read API, hidden from the review queue). Unmintable: UUIDv7
/// never produces an all-ones value.
pub const EXCLUDED_PERSON: Uuid = Uuid::from_u128(u128::MAX);

/// Which verb produced a correction — stamped into `persons.reason` so the
/// journal explains itself without joining the operations log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Bind,
    Merge,
    Detach,
    Exclude,
}

impl Verb {
    #[must_use]
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::Bind => "operator-bind",
            Self::Merge => "operator-merge",
            Self::Detach => "operator-detach",
            Self::Exclude => "operator-exclude",
        }
    }
}

/// What a correction does to one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The identical operator decision is already recorded — nothing to append.
    AlreadyDecided,
    /// Append a binding observation for this account.
    Append,
}

/// Decide whether a correction has anything to append.
///
/// Idempotency is **decision-aware**: repeating an operator's own decision is a
/// no-op, but re-asserting a binding that automation made is the confirm act
/// and must be recorded — that is what takes the account out of the review
/// queue and makes the binding authoritative.
#[must_use]
pub fn decide(current: Option<KnownBinding>, target_person_id: Uuid) -> Outcome {
    match current {
        Some(binding)
            if binding.person_id == target_person_id && binding.is_operator_authored() =>
        {
            Outcome::AlreadyDecided
        }
        _ => Outcome::Append,
    }
}

/// A binding observation to append for one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRow {
    pub account: SourceAccountKey,
    pub person_id: Uuid,
    pub author_person_id: Uuid,
    pub reason: String,
    pub created_at: DateTime,
}

/// One account and the person the operator wants it bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub account: SourceAccountKey,
    pub person_id: Uuid,
}

/// Build the rows one correction appends, skipping targets whose decision is
/// already recorded. `at` is the operation's instant, truncated to whole
/// microseconds (`DATETIME(6)` stores nothing finer, and the write path
/// compares its rows against the stored ones); rows that would collide inside
/// the natural key are nudged forward (see [`SlotAllocator`]).
///
/// The whole operation shares one allocator — a bulk call may name several
/// persons, and two of its rows can still collide on the same key.
#[must_use]
pub fn build_rows<'a>(
    targets: impl IntoIterator<Item = (&'a Target, Option<KnownBinding>)>,
    operator_person_id: Uuid,
    verb: Verb,
    at: DateTime,
) -> Vec<BindingRow> {
    let at = observation_slot::truncate_to_micros(at);
    let mut slots = SlotAllocator::new();
    let mut rows = Vec::new();

    for (target, current) in targets {
        if decide(current, target.person_id) == Outcome::AlreadyDecided {
            continue;
        }

        let created_at = slots.claim(
            target.person_id,
            &target.account.source_type,
            target.account.source_id,
            BINDING_VALUE_TYPE,
            at,
        );

        rows.push(BindingRow {
            account: target.account.clone(),
            person_id: target.person_id,
            author_person_id: operator_person_id,
            reason: verb.reason_code().to_owned(),
            created_at,
        });
    }

    rows
}

/// Re-stamp rows the database refused so a retry cannot collide again: every
/// row moves past the last instant the operation used. The natural key has no
/// account discriminator, so two operations racing on the same microsecond can
/// have one of their rows dropped by the insert — this is how the caller
/// recovers the dropped row instead of losing a binding.
#[must_use]
pub fn restamp(rows: &[BindingRow], after: DateTime) -> Vec<BindingRow> {
    let after = observation_slot::truncate_to_micros(after);
    let mut slots = SlotAllocator::new();

    rows.iter()
        .map(|row| {
            let created_at = slots.claim(
                row.person_id,
                &row.account.source_type,
                row.account.source_id,
                BINDING_VALUE_TYPE,
                after,
            );
            BindingRow {
                created_at,
                ..row.clone()
            }
        })
        .collect()
}

/// The rows the journal does not hold, in input order — what a short write has
/// to retry. `present` is the answer to "which of these exact observations
/// landed", one flag per row.
#[must_use]
pub fn missing(rows: &[BindingRow], present: &[bool]) -> Vec<BindingRow> {
    rows.iter()
        .zip(present)
        .filter(|(_, landed)| !**landed)
        .map(|(row, _)| row.clone())
        .collect()
}

/// Fold a retry's outcome back into the first attempt's: the n-th row that was
/// missing is answered by the n-th flag of `recovered`, and rows that already
/// landed are never revisited. A gap with no answer stays refused — a row the
/// database would not take twice is reported, not counted as applied.
pub fn apply_recovery(present: &mut [bool], recovered: &[bool]) {
    let mut answers = recovered.iter();
    for landed in present.iter_mut().filter(|landed| !**landed) {
        *landed = answers.next().copied().unwrap_or(false);
    }
}

/// Binding observations are `value_type='id'` rows whose value is the account id
/// (ADR-0002).
pub const BINDING_VALUE_TYPE: &str = "id";

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::restamp as resolution_restamp;
    use super::*;

    fn account(source_type: &str, account_id: &str) -> SourceAccountKey {
        SourceAccountKey {
            source_type: source_type.to_owned(),
            source_id: Uuid::from_u128(1),
            account_id: account_id.to_owned(),
        }
    }

    fn seed_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::nil(),
        }
    }

    fn operator_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::from_u128(0xAD_1119),
        }
    }

    fn ts() -> DateTime {
        chrono::DateTime::UNIX_EPOCH.naive_utc() + TimeDelta::days(20_000)
    }

    #[test]
    fn repeating_an_operator_decision_is_a_no_op() {
        let outcome = decide(Some(operator_bound(5)), Uuid::from_u128(5));
        assert_eq!(outcome, Outcome::AlreadyDecided);
    }

    #[test]
    fn confirming_an_automation_binding_appends() {
        // Same person, but the binding came from automation: the operator's
        // confirmation must be recorded — this is what clears the review item.
        let outcome = decide(Some(seed_bound(5)), Uuid::from_u128(5));
        assert_eq!(outcome, Outcome::Append);
    }

    #[test]
    fn rebinding_and_first_binding_append() {
        for (label, current) in [
            (
                "operator moves the account elsewhere",
                Some(operator_bound(5)),
            ),
            ("automation bound it elsewhere", Some(seed_bound(5))),
            ("never bound", None),
        ] {
            assert_eq!(
                decide(current, Uuid::from_u128(9)),
                Outcome::Append,
                "should append: {label}"
            );
        }
    }

    #[test]
    fn rows_skip_already_decided_accounts() {
        let settled = account("slack", "U1");
        let fresh = account("slack", "U2");
        let target = Uuid::from_u128(5);

        let settled_target = Target {
            account: settled.clone(),
            person_id: target,
        };
        let fresh_target = Target {
            account: fresh.clone(),
            person_id: target,
        };
        let rows = build_rows(
            [
                (&settled_target, Some(operator_bound(5))),
                (&fresh_target, Some(seed_bound(7))),
            ],
            Uuid::from_u128(42),
            Verb::Bind,
            ts(),
        );

        assert_eq!(rows.len(), 1, "the settled account contributes no row");
        assert_eq!(rows[0].account, fresh);
        assert_eq!(rows[0].person_id, target);
        assert_eq!(rows[0].author_person_id, Uuid::from_u128(42));
        assert_eq!(rows[0].reason, "operator-bind");
    }

    #[test]
    fn same_source_accounts_get_distinct_timestamps() {
        // Two accounts of one source rebound to one person in one operation:
        // their id rows share every other natural-key column.
        let a = account("bamboohr", "1");
        let b = account("bamboohr", "2");

        let a_target = Target {
            account: a,
            person_id: Uuid::from_u128(5),
        };
        let b_target = Target {
            account: b,
            person_id: Uuid::from_u128(5),
        };
        let rows = build_rows(
            [(&a_target, None), (&b_target, None)],
            Uuid::from_u128(42),
            Verb::Merge,
            ts(),
        );

        assert_eq!(rows[0].created_at, ts());
        assert_eq!(rows[1].created_at, ts() + TimeDelta::microseconds(1));
    }

    #[test]
    fn an_os_clock_instant_is_stamped_at_database_precision() {
        // `Utc::now()` carries nanoseconds; the journal stores DATETIME(6).
        // Rows must be built at microsecond precision or the write path could
        // never match them against what the database returns.
        let target = Target {
            account: account("slack", "U1"),
            person_id: Uuid::from_u128(5),
        };
        let os_clock = ts() + TimeDelta::nanoseconds(1_999);

        let rows = build_rows([(&target, None)], Uuid::from_u128(42), Verb::Bind, os_clock);
        let retried = resolution_restamp(&rows, os_clock + TimeDelta::nanoseconds(2_500));

        assert_eq!(rows[0].created_at, ts() + TimeDelta::microseconds(1));
        assert_eq!(retried[0].created_at, ts() + TimeDelta::microseconds(4));
    }

    #[test]
    fn restamped_rows_move_past_the_contended_instant() {
        // The recovery path: a row the insert refused is re-stamped after the
        // moment two operations fought over, so the retry cannot collide again.
        let account = account("slack", "U1");
        let refused = vec![BindingRow {
            account: account.clone(),
            person_id: Uuid::from_u128(5),
            author_person_id: Uuid::from_u128(42),
            reason: Verb::Bind.reason_code().to_owned(),
            created_at: ts(),
        }];

        let retry = resolution_restamp(&refused, ts() + TimeDelta::seconds(1));

        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].created_at, ts() + TimeDelta::seconds(1));
        assert_eq!(retry[0].account, account, "the row itself is unchanged");
        assert_eq!(retry[0].person_id, refused[0].person_id);
        assert_eq!(retry[0].author_person_id, refused[0].author_person_id);
    }

    #[test]
    fn restamping_keeps_colliding_rows_apart() {
        // Two refused rows of one source retried together must not collide
        // with each other on the way back in.
        let rows = vec![
            BindingRow {
                account: account("bamboohr", "1"),
                person_id: Uuid::from_u128(5),
                author_person_id: Uuid::from_u128(42),
                reason: Verb::Merge.reason_code().to_owned(),
                created_at: ts(),
            },
            BindingRow {
                account: account("bamboohr", "2"),
                person_id: Uuid::from_u128(5),
                author_person_id: Uuid::from_u128(42),
                reason: Verb::Merge.reason_code().to_owned(),
                created_at: ts(),
            },
        ];

        let retry = resolution_restamp(&rows, ts() + TimeDelta::seconds(1));

        assert_ne!(retry[0].created_at, retry[1].created_at);
    }

    fn row(account_id: &str) -> BindingRow {
        BindingRow {
            account: account("slack", account_id),
            person_id: Uuid::from_u128(5),
            author_person_id: Uuid::from_u128(42),
            reason: Verb::Bind.reason_code().to_owned(),
            created_at: ts(),
        }
    }

    #[test]
    fn only_the_rows_the_journal_lacks_are_retried() {
        // Re-sending a row that landed would duplicate history, so the retry
        // set is exactly the gaps — in input order, since the caller answers
        // its items by position.
        let rows = [row("U1"), row("U2"), row("U3")];

        let retry = missing(&rows, &[true, false, false]);

        let retried: Vec<&str> = retry
            .iter()
            .map(|r| r.account.account_id.as_str())
            .collect();
        assert_eq!(retried, vec!["U2", "U3"]);
    }

    #[test]
    fn recovery_answers_the_gaps_in_order_and_leaves_landed_rows_alone() {
        // The n-th retried row answers the n-th gap: an off-by-one here would
        // credit one account's write to another.
        let mut present = vec![true, false, true, false];

        apply_recovery(&mut present, &[false, true]);

        assert_eq!(present, vec![true, false, true, true]);
    }

    #[test]
    fn a_row_the_database_refuses_twice_stays_refused() {
        // Fewer answers than gaps: the unanswered row is reported as refused
        // rather than optimistically counted as applied.
        let mut present = vec![false, false];

        apply_recovery(&mut present, &[true]);

        assert_eq!(present, vec![true, false]);
    }

    #[test]
    fn a_write_with_nothing_missing_is_left_untouched() {
        let mut present = vec![true, true];

        apply_recovery(&mut present, &[]);

        assert_eq!(present, vec![true, true]);
        assert!(missing(&[row("U1"), row("U2")], &present).is_empty());
    }

    #[test]
    fn verbs_carry_distinct_reason_codes() {
        let codes: Vec<&str> = [Verb::Bind, Verb::Merge, Verb::Detach, Verb::Exclude]
            .into_iter()
            .map(Verb::reason_code)
            .collect();
        assert_eq!(
            codes,
            vec![
                "operator-bind",
                "operator-merge",
                "operator-detach",
                "operator-exclude"
            ]
        );
    }

    #[test]
    fn excluded_sentinel_is_not_a_mintable_uuid() {
        // UUIDv7 sets version/variant bits, so an all-ones value can never be
        // minted for a real person.
        assert_ne!(EXCLUDED_PERSON.get_version_num(), 7);
        assert_eq!(EXCLUDED_PERSON, Uuid::from_u128(u128::MAX));
    }
}
