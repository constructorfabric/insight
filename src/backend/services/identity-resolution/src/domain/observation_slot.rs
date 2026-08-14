//! Distinct `created_at` per observation within one write.
//!
//! `uq_person_observation` ends in `created_at` and carries no account
//! discriminator, so two accounts of one source landing on one person at the
//! same instant collide and `INSERT IGNORE` drops one binding. Both writers —
//! the seed (timestamps from `_synced_at`) and the operator corrections
//! (timestamps from the operation's clock) — claim slots through here.

use std::collections::HashSet;

use chrono::{TimeDelta, Timelike};
use sea_orm::prelude::DateTime;
use uuid::Uuid;

/// Truncate an instant to whole microseconds — the finest step `DATETIME(6)`
/// stores. The write path compares its in-memory rows against what the
/// database returns to tell landed rows from refused ones; an instant carrying
/// sub-microsecond nanoseconds (any OS clock read) would never compare equal
/// to its own stored row, and the recovery would re-insert rows that landed.
#[must_use]
pub fn truncate_to_micros(at: DateTime) -> DateTime {
    at - TimeDelta::nanoseconds(i64::from(at.nanosecond() % 1_000))
}

/// The natural-key columns a writer controls; the tenant is bound once per
/// operation by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ObservationKey {
    person_id: Uuid,
    source_type: String,
    source_id: Uuid,
    value_type: String,
    created_at: DateTime,
}

/// Hands out an unused `created_at` per natural key, nudging forward by whole
/// microseconds — the smallest step `DATETIME(6)` stores, so chronology is
/// preserved. One allocator per write operation.
#[derive(Debug, Default)]
pub struct SlotAllocator {
    taken: HashSet<ObservationKey>,
}

impl SlotAllocator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim `preferred`, or the next free microsecond for this key.
    pub fn claim(
        &mut self,
        person_id: Uuid,
        source_type: &str,
        source_id: Uuid,
        value_type: &str,
        preferred: DateTime,
    ) -> DateTime {
        let mut key = ObservationKey {
            person_id,
            source_type: source_type.to_owned(),
            source_id,
            value_type: value_type.to_owned(),
            created_at: preferred,
        };
        while !self.taken.insert(key.clone()) {
            key.created_at += TimeDelta::microseconds(1);
        }
        key.created_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> DateTime {
        chrono::DateTime::UNIX_EPOCH.naive_utc() + TimeDelta::days(20_000)
    }

    #[test]
    fn sub_microsecond_instants_truncate_to_what_the_database_stores() {
        let clean = ts() + TimeDelta::microseconds(3);

        assert_eq!(
            truncate_to_micros(clean + TimeDelta::nanoseconds(999)),
            clean,
            "sub-microsecond nanoseconds are dropped"
        );
        assert_eq!(truncate_to_micros(clean), clean, "a clean instant is kept");
    }

    #[test]
    fn colliding_keys_advance_by_one_microsecond_each() {
        let mut slots = SlotAllocator::new();
        let person = Uuid::from_u128(1);
        let source = Uuid::from_u128(2);

        let first = slots.claim(person, "bamboohr", source, "id", ts());
        let second = slots.claim(person, "bamboohr", source, "id", ts());
        let third = slots.claim(person, "bamboohr", source, "id", ts());

        assert_eq!(first, ts(), "the first claim keeps its own instant");
        assert_eq!(second, ts() + TimeDelta::microseconds(1));
        assert_eq!(third, ts() + TimeDelta::microseconds(2));
    }

    #[test]
    fn distinct_keys_keep_the_same_instant() {
        let mut slots = SlotAllocator::new();
        let person = Uuid::from_u128(1);
        let source = Uuid::from_u128(2);
        let other_person = Uuid::from_u128(9);

        let id_row = slots.claim(person, "bamboohr", source, "id", ts());
        let email_row = slots.claim(person, "bamboohr", source, "email", ts());
        let other_source = slots.claim(person, "slack", source, "id", ts());
        let other_person_row = slots.claim(other_person, "bamboohr", source, "id", ts());

        for (label, claimed) in [
            ("value_type differs", email_row),
            ("source_type differs", other_source),
            ("person differs", other_person_row),
        ] {
            assert_eq!(claimed, ts(), "no collision when {label}");
        }
        assert_eq!(id_row, ts());
    }
}
