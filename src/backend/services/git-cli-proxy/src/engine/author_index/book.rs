use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;

use serde::{Deserialize, Serialize};

use crate::engine::read::authors::AuthorRow;
use crate::engine::read::commits::ordinal_of;

/// Every author one tenant and source has been seen with, across the mirrors
/// the proxy fetched for them, indexed for the change feed.
///
/// INVARIANT: `last_stamp_us` only grows, also across restarts — it is the
/// change cursor a connector resumes from, so a stamp issued after a reader's
/// high-water mark must sort after it.
#[derive(Debug, Default)]
pub(crate) struct Book {
    last_stamp_us: i64,
    stamps: HashMap<String, AuthorStamps>,
    /// `(changed_us, email)` — the feed's order, so a page reads a range
    /// instead of sorting the whole book.
    order: BTreeSet<(i64, String)>,
    /// Keyed by the mirror's cache directory name.
    entries: HashMap<String, EntryRecord>,
    /// Which entries hold each e-mail, so a page resolves a sample from the
    /// author's own entries instead of scanning every entry.
    holders: HashMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AuthorStamps {
    pub(crate) first_seen_us: i64,
    pub(crate) changed_us: i64,
}

/// What one mirror held the last time its authors were folded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct EntryRecord {
    pub(crate) clone_url: String,
    /// The snapshot the authors were folded from; another one means the
    /// record is stale and the next refresh folds again.
    pub(crate) incarnation: String,
    pub(crate) generation: u64,
    pub(crate) fingerprints: Vec<String>,
    pub(crate) authors: BTreeMap<String, EntryAuthor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct EntryAuthor {
    pub(crate) name: String,
    pub(crate) sha: String,
    pub(crate) committed: String,
    /// `committed` as a fixed-width UTC instant, so picking the newest sample
    /// compares strings instead of parsing dates on every page.
    pub(crate) ordinal: String,
    pub(crate) count: u64,
}

/// One refresh of one mirror, as the index learns of it.
#[derive(Debug)]
pub(crate) struct Observation {
    pub(crate) dir_name: String,
    pub(crate) clone_url: String,
    pub(crate) incarnation: String,
    pub(crate) generation: u64,
    pub(crate) fingerprints: Vec<String>,
    /// `None` when the snapshot was already folded and only the credentials
    /// that proved access may have changed.
    pub(crate) authors: Option<Vec<AuthorRow>>,
}

/// What an [`Observation`] changed, in the order it must be made durable:
/// the stamps first, so a crash before the entry record lands re-folds the
/// entry and stamps again rather than recording a fold nobody was told of.
#[derive(Debug, Default)]
pub(crate) struct Applied {
    pub(crate) stamped: Vec<(String, AuthorStamps)>,
    pub(crate) entry: Option<(String, EntryRecord)>,
}

/// An author as the change feed reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorChange {
    pub email: String,
    pub name: String,
    pub changed_us: i64,
    pub first_seen_us: i64,
    pub last_committed: String,
    pub sample_url: String,
    pub sample_sha: String,
}

/// Which slice of the feed one page reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Window<'a> {
    /// Exclusive lower bound on `changed_us`.
    pub(crate) since_us: Option<i64>,
    /// Exclusive keyset position of the previous page's last item.
    pub(crate) after: Option<(i64, &'a str)>,
    /// Inclusive upper bound fixed by the walk's first page.
    pub(crate) high_water_us: i64,
    pub(crate) limit: usize,
}

impl Book {
    /// Rebuild a book from what was persisted.
    pub(crate) fn restore(
        stamps: HashMap<String, AuthorStamps>,
        entries: HashMap<String, EntryRecord>,
    ) -> Self {
        let mut book = Self::default();
        for (email, author_stamps) in stamps {
            book.set_stamps(email, author_stamps);
        }
        for (dir_name, record) in entries {
            book.put_entry(dir_name, record);
        }
        book
    }

    #[must_use]
    pub(crate) fn last_stamp_us(&self) -> i64 {
        self.last_stamp_us
    }

    #[must_use]
    pub(crate) fn stamps(&self) -> &HashMap<String, AuthorStamps> {
        &self.stamps
    }

    /// Whether a refresh must fold the mirror's authors: the index has no
    /// record of this entry, or the record describes another snapshot.
    #[must_use]
    pub(crate) fn needs_fold(&self, dir_name: &str, incarnation: &str, generation: u64) -> bool {
        self.entries.get(dir_name).is_none_or(|record| {
            record.incarnation != incarnation || record.generation != generation
        })
    }

    /// Fold one refresh into the book and stamp every author it changed.
    pub(crate) fn apply(&mut self, observation: Observation, now_us: i64) -> Applied {
        let previous = self.entries.get(&observation.dir_name);

        // A credential this entry never recorded may be one no caller has read
        // its authors through yet; its cursor may already sit past them.
        let new_proof = observation.fingerprints.iter().any(|fingerprint| {
            previous.is_none_or(|record| !record.fingerprints.contains(fingerprint))
        });

        let (authors, incarnation, generation) = match observation.authors {
            Some(rows) => (
                fold_rows(rows),
                observation.incarnation,
                observation.generation,
            ),
            None => match previous {
                Some(record) => (
                    record.authors.clone(),
                    record.incarnation.clone(),
                    record.generation,
                ),
                // Nothing folded and nothing to fold against: no record yet.
                None => return Applied::default(),
            },
        };

        let changed: Vec<String> = authors
            .iter()
            .filter(|(email, author)| {
                new_proof
                    || previous
                        .and_then(|record| record.authors.get(*email))
                        // The count, not the date: a commit pushed with an old
                        // committer date grows the count without moving the
                        // newest one.
                        .is_none_or(|before| author.count > before.count)
            })
            .map(|(email, _)| email.clone())
            .collect();

        let record = EntryRecord {
            clone_url: observation.clone_url,
            incarnation,
            generation,
            fingerprints: observation.fingerprints,
            authors,
        };
        let entry =
            (previous != Some(&record)).then(|| (observation.dir_name.clone(), record.clone()));
        self.put_entry(observation.dir_name, record);

        let mut applied = Applied {
            stamped: Vec::with_capacity(changed.len()),
            entry,
        };
        if changed.is_empty() {
            return applied;
        }
        let stamp = self.tick(now_us);
        for email in changed {
            let author_stamps = AuthorStamps {
                first_seen_us: self.stamps.get(&email).map_or(stamp, |s| s.first_seen_us),
                changed_us: stamp,
            };
            self.set_stamps(email.clone(), author_stamps);
            applied.stamped.push((email, author_stamps));
        }
        applied
    }

    /// One page of the feed for a caller holding `fingerprint`, ascending by
    /// `(changed_us, email)`, and whether more pages follow.
    ///
    /// An author is reported only through entries that recorded the caller's
    /// fingerprint; an author with no such entry is not reported at all.
    #[must_use]
    pub(crate) fn changes(
        &self,
        fingerprint: &str,
        window: Window<'_>,
    ) -> (Vec<AuthorChange>, bool) {
        let lower = match (window.after, window.since_us) {
            (Some((at, email)), _) => Bound::Excluded((at, email.to_owned())),
            (None, Some(since)) => Bound::Included((since.saturating_add(1), String::new())),
            (None, None) => Bound::Unbounded,
        };
        let upper = Bound::Excluded((window.high_water_us.saturating_add(1), String::new()));

        let mut page = Vec::new();
        for (_, email) in self.order.range((lower, upper)) {
            let Some(change) = self.visible_change(email, fingerprint) else {
                continue;
            };
            if page.len() == window.limit {
                return (page, true);
            }
            page.push(change);
        }
        (page, false)
    }

    /// The author as `fingerprint` may see them: the newest commit among the
    /// entries that recorded that credential, or nothing when none did.
    fn visible_change(&self, email: &str, fingerprint: &str) -> Option<AuthorChange> {
        let stamps = self.stamps.get(email)?;
        let (record, author) = self
            .holders
            .get(email)?
            .iter()
            .filter_map(|dir_name| self.entries.get(dir_name))
            .filter(|record| record.fingerprints.iter().any(|proof| proof == fingerprint))
            .filter_map(|record| record.authors.get(email).map(|author| (record, author)))
            .max_by(|(_, a), (_, b)| a.ordinal.cmp(&b.ordinal))?;
        Some(AuthorChange {
            email: email.to_owned(),
            name: author.name.clone(),
            changed_us: stamps.changed_us,
            first_seen_us: stamps.first_seen_us,
            last_committed: author.committed.clone(),
            sample_url: record.clone_url.clone(),
            sample_sha: author.sha.clone(),
        })
    }

    /// The next change stamp: the wall clock, or one microsecond past the
    /// last stamp when the clock has not moved past it.
    fn tick(&mut self, now_us: i64) -> i64 {
        self.last_stamp_us = now_us.max(self.last_stamp_us.saturating_add(1));
        self.last_stamp_us
    }

    fn set_stamps(&mut self, email: String, stamps: AuthorStamps) {
        if let Some(old) = self.stamps.get(&email) {
            self.order.remove(&(old.changed_us, email.clone()));
        }
        self.last_stamp_us = self.last_stamp_us.max(stamps.changed_us);
        self.order.insert((stamps.changed_us, email.clone()));
        self.stamps.insert(email, stamps);
    }

    /// Drop a mirror whose repository origin no longer has, so none of its
    /// commits is offered as a sample the vendor would answer 404 for.
    /// Returns whether the book dropped it.
    ///
    /// SAFETY: only on the word of a credential the entry recorded. Origin
    /// answers 404 to a credential that cannot see a private repository, and
    /// such a caller must not erase what others proved access to.
    pub(crate) fn forget(&mut self, dir_name: &str, fingerprint: &str) -> bool {
        let proven = self
            .entries
            .get(dir_name)
            .is_some_and(|record| record.fingerprints.iter().any(|proof| proof == fingerprint));
        if !proven {
            return false;
        }
        let Some(record) = self.entries.remove(dir_name) else {
            return false;
        };
        for email in record.authors.keys() {
            if let Some(holders) = self.holders.get_mut(email) {
                holders.remove(dir_name);
            }
        }
        true
    }

    fn put_entry(&mut self, dir_name: String, record: EntryRecord) {
        if let Some(old) = self.entries.get(&dir_name) {
            for email in old.authors.keys() {
                if let Some(holders) = self.holders.get_mut(email) {
                    holders.remove(&dir_name);
                }
            }
        }
        for email in record.authors.keys() {
            self.holders
                .entry(email.clone())
                .or_default()
                .insert(dir_name.clone());
        }
        self.entries.insert(dir_name, record);
    }
}

fn fold_rows(rows: Vec<AuthorRow>) -> BTreeMap<String, EntryAuthor> {
    rows.into_iter()
        .map(|row| {
            let ordinal = ordinal_of(&row.last_committed_date);
            (
                row.author_email,
                EntryAuthor {
                    name: row.author_name,
                    sha: row.sample_sha,
                    committed: row.last_committed_date,
                    ordinal,
                    count: row.commit_count,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "alice@example.com";
    const BOB: &str = "bob@example.com";
    const FP: &str = "fp-reader";
    const OTHER_FP: &str = "fp-other";

    fn row(email: &str, sha: char, committed: &str, count: u64) -> AuthorRow {
        AuthorRow {
            author_email: email.to_owned(),
            author_name: email.split('@').next().unwrap_or_default().to_owned(),
            sample_sha: sha.to_string().repeat(40),
            last_committed_date: committed.to_owned(),
            commit_count: count,
        }
    }

    fn folded(
        dir: &str,
        generation: u64,
        fingerprints: &[&str],
        rows: Vec<AuthorRow>,
    ) -> Observation {
        Observation {
            dir_name: dir.to_owned(),
            clone_url: format!("https://git.example.com/acme/{dir}.git"),
            incarnation: "inc".to_owned(),
            generation,
            fingerprints: fingerprints.iter().map(|fp| (*fp).to_owned()).collect(),
            authors: Some(rows),
        }
    }

    fn window(
        since_us: Option<i64>,
        after: Option<(i64, &str)>,
        high_water_us: i64,
        limit: usize,
    ) -> Window<'_> {
        Window {
            since_us,
            after,
            high_water_us,
            limit,
        }
    }

    fn everything(book: &Book) -> Vec<AuthorChange> {
        book.changes(FP, window(None, None, book.last_stamp_us, usize::MAX))
            .0
    }

    fn emails_changed_after(book: &Book, since_us: i64) -> Vec<String> {
        book.changes(
            FP,
            window(Some(since_us), None, book.last_stamp_us, usize::MAX),
        )
        .0
        .into_iter()
        .map(|change| change.email)
        .collect()
    }

    #[test]
    fn every_rule_that_makes_an_author_change() {
        // (case, second observation of entry `app`, authors expected to change)
        let cases: Vec<(&str, Observation, Vec<&str>)> = vec![
            (
                "a new e-mail",
                folded(
                    "app",
                    2,
                    &[FP],
                    vec![
                        row(ALICE, 'a', "2026-01-01T00:00:00Z", 1),
                        row(BOB, 'b', "2026-01-02T00:00:00Z", 1),
                    ],
                ),
                vec![BOB],
            ),
            (
                "a back-dated commit grows the count, not the date",
                folded(
                    "app",
                    2,
                    &[FP],
                    vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 2)],
                ),
                vec![ALICE],
            ),
            (
                "a credential the entry never recorded",
                folded(
                    "app",
                    1,
                    &[OTHER_FP, FP],
                    vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
                ),
                vec![ALICE],
            ),
            (
                "a rewritten history that lost commits",
                folded(
                    "app",
                    2,
                    &[FP],
                    vec![row(ALICE, 'c', "2025-12-01T00:00:00Z", 1)],
                ),
                vec![],
            ),
            (
                "the same snapshot again",
                folded(
                    "app",
                    1,
                    &[FP],
                    vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
                ),
                vec![],
            ),
        ];

        for (case, second, expected) in cases {
            let mut book = Book::default();
            book.apply(
                folded(
                    "app",
                    1,
                    &[FP],
                    vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
                ),
                1_000,
            );
            let mark = book.last_stamp_us;

            book.apply(second, 2_000);

            assert_eq!(emails_changed_after(&book, mark), expected, "case: {case}");
        }
    }

    #[test]
    fn an_author_known_from_one_repository_is_new_in_another() {
        let mut book = Book::default();
        book.apply(
            folded(
                "app",
                1,
                &[FP],
                vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 3)],
            ),
            1_000,
        );
        let mark = book.last_stamp_us;

        book.apply(
            folded(
                "lib",
                1,
                &[FP],
                vec![row(ALICE, 'b', "2026-01-05T00:00:00Z", 1)],
            ),
            2_000,
        );

        assert_eq!(emails_changed_after(&book, mark), vec![ALICE]);
        let alice = &everything(&book)[0];
        assert_eq!(alice.first_seen_us, 1_000, "first sight is kept");
        assert_eq!(
            alice.sample_sha,
            "b".repeat(40),
            "the newest commit is the sample"
        );
        assert!(alice.sample_url.ends_with("/lib.git"));
    }

    #[test]
    fn the_sample_comes_only_from_an_entry_the_caller_may_read() {
        let mut book = Book::default();
        book.apply(
            folded(
                "app",
                1,
                &[FP],
                vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
            ),
            1_000,
        );
        book.apply(
            folded(
                "secret",
                1,
                &[OTHER_FP],
                vec![row(ALICE, 's', "2026-02-01T00:00:00Z", 1)],
            ),
            2_000,
        );

        let alice = &everything(&book)[0];
        assert_eq!(
            alice.sample_sha,
            "a".repeat(40),
            "the newer commit sits in an unreadable entry"
        );
        assert!(alice.sample_url.ends_with("/app.git"));
    }

    #[test]
    fn a_fingerprint_only_refresh_resurfaces_the_entrys_authors_for_the_new_credential() {
        let mut book = Book::default();
        book.apply(
            folded(
                "app",
                1,
                &[OTHER_FP],
                vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
            ),
            1_000,
        );
        assert!(
            everything(&book).is_empty(),
            "the reader holds no proof yet"
        );

        let applied = book.apply(
            Observation {
                authors: None,
                ..folded("app", 1, &[OTHER_FP, FP], Vec::new())
            },
            2_000,
        );

        assert_eq!(applied.stamped.len(), 1);
        assert!(applied.entry.is_some(), "the new proof must be persisted");
        assert_eq!(emails_changed_after(&book, 1_000), vec![ALICE]);
    }

    #[test]
    fn stamps_keep_increasing_when_the_clock_does_not() {
        let mut book = Book::default();
        book.apply(
            folded(
                "app",
                1,
                &[FP],
                vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
            ),
            5_000,
        );
        book.apply(
            folded(
                "lib",
                1,
                &[FP],
                vec![row(BOB, 'b', "2026-01-01T00:00:00Z", 1)],
            ),
            4_000,
        );

        let stamps: Vec<i64> = everything(&book).iter().map(|c| c.changed_us).collect();
        assert_eq!(
            stamps,
            vec![5_000, 5_001],
            "a clock stepping back must not reuse a stamp"
        );
    }

    #[test]
    fn a_change_recorded_mid_walk_waits_for_the_next_walk() {
        let mut book = Book::default();
        book.apply(
            folded(
                "app",
                1,
                &[FP],
                vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
            ),
            1_000,
        );
        let high_water = book.last_stamp_us;
        book.apply(
            folded(
                "lib",
                1,
                &[FP],
                vec![row(BOB, 'b', "2026-01-01T00:00:00Z", 1)],
            ),
            2_000,
        );

        let (page, more) = book.changes(FP, window(None, None, high_water, 10));

        assert_eq!(page.len(), 1);
        assert_eq!(page[0].email, ALICE);
        assert!(!more, "BOB sits above the walk's high-water mark");
    }

    #[test]
    fn pages_resume_after_the_last_item_and_report_whether_more_follow() {
        let mut book = Book::default();
        book.apply(
            folded(
                "app",
                1,
                &[FP],
                vec![
                    row(ALICE, 'a', "2026-01-01T00:00:00Z", 1),
                    row(BOB, 'b', "2026-01-01T00:00:00Z", 1),
                ],
            ),
            1_000,
        );

        let (first, more) = book.changes(FP, window(None, None, 1_000, 1));
        assert_eq!(first[0].email, ALICE);
        assert!(more);

        let (second, more) = book.changes(FP, window(None, Some((1_000, ALICE)), 1_000, 1));
        assert_eq!(second[0].email, BOB, "same stamp, ordered by e-mail");
        assert!(!more);
    }

    #[test]
    fn an_invisible_author_neither_fills_a_page_nor_signals_more() {
        let mut book = Book::default();
        book.apply(
            folded(
                "app",
                1,
                &[FP],
                vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
            ),
            1_000,
        );
        book.apply(
            folded(
                "secret",
                1,
                &[OTHER_FP],
                vec![row(BOB, 'b', "2026-01-01T00:00:00Z", 1)],
            ),
            2_000,
        );

        let (page, more) = book.changes(FP, window(None, None, 2_000, 1));

        assert_eq!(page.len(), 1);
        assert!(
            !more,
            "BOB is not the caller's to read, so no further page exists"
        );
    }

    #[test]
    fn a_re_cloned_entry_with_the_same_history_changes_nobody() {
        let mut book = Book::default();
        let rows = || vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 4)];
        book.apply(folded("app", 7, &[FP], rows()), 1_000);
        assert!(book.needs_fold("app", "fresh-incarnation", 1));

        let applied = book.apply(
            Observation {
                incarnation: "fresh-incarnation".to_owned(),
                ..folded("app", 1, &[FP], rows())
            },
            2_000,
        );

        assert!(applied.stamped.is_empty());
        assert!(emails_changed_after(&book, 1_000).is_empty());
        assert!(!book.needs_fold("app", "fresh-incarnation", 1));
    }

    #[test]
    fn a_forgotten_repository_no_longer_supplies_a_sample() {
        let mut book = Book::default();
        book.apply(
            folded(
                "app",
                1,
                &[FP],
                vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
            ),
            1_000,
        );
        book.apply(
            folded(
                "gone",
                1,
                &[FP],
                vec![row(ALICE, 'g', "2026-02-01T00:00:00Z", 1)],
            ),
            2_000,
        );

        assert!(
            !book.forget("gone", OTHER_FP),
            "a credential the entry never recorded cannot erase it"
        );
        assert!(book.forget("gone", FP));

        let alice = &everything(&book)[0];
        assert_eq!(
            alice.sample_sha,
            "a".repeat(40),
            "the surviving repository supplies it"
        );
        assert!(!book.forget("gone", FP), "forgetting twice is a no-op");
    }

    #[test]
    fn a_restored_book_answers_exactly_like_the_one_it_was_saved_from() {
        let mut book = Book::default();
        book.apply(
            folded(
                "app",
                1,
                &[FP],
                vec![row(ALICE, 'a', "2026-01-01T00:00:00Z", 1)],
            ),
            1_000,
        );
        book.apply(
            folded(
                "lib",
                1,
                &[FP],
                vec![row(BOB, 'b', "2026-01-02T00:00:00Z", 1)],
            ),
            2_000,
        );

        let restored = Book::restore(book.stamps.clone(), book.entries.clone());

        assert_eq!(everything(&restored), everything(&book));
        assert_eq!(restored.last_stamp_us, book.last_stamp_us);
    }
}
