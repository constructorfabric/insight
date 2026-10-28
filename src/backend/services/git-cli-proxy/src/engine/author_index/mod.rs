//! Every commit author the proxy has observed per tenant and source, and the
//! proxy time each one last changed — the source of the workspace-wide author
//! change feed.
//!
//! Unlike a mirror, a book outlives eviction: a re-cloned entry is folded
//! against what the book already holds and reports nothing new. A lost book
//! is not a gap — every author re-surfaces as its mirrors are fetched again.

mod book;
mod cursor;
mod file;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OnceCell};

use crate::engine::metrics::{self, AuthorIndexFailure};
use crate::engine::read::authors::AuthorRow;

pub use book::AuthorChange;
pub use cursor::{ChangesCursor, MalformedCursor, format_stamp, ordinal_of_us, parse_instant_us};

use book::{Book, Observation, Window};
use file::Shelf;

/// What one feed request asks for; the cursor overrides the bounds on every
/// page after the first.
#[derive(Debug, Clone, Copy, Default)]
pub struct FeedQuery<'a> {
    /// Exclusive lower bound on `changed_at`.
    pub since_us: Option<i64>,
    /// Authors whose newest readable commit predates this are omitted.
    pub committed_since_us: Option<i64>,
    pub cursor: Option<&'a ChangesCursor>,
    pub limit: usize,
}

/// One page of the change feed.
#[derive(Debug)]
pub struct ChangesPage {
    pub items: Vec<AuthorChange>,
    pub next: Option<ChangesCursor>,
}

/// The refresh of one mirror, as the index learns of it.
#[derive(Debug)]
pub struct Refresh {
    pub tenant_id: String,
    pub source_id: String,
    pub dir_name: String,
    pub clone_url: String,
    pub incarnation: String,
    pub generation: u64,
    pub fingerprints: Vec<String>,
}

/// A book and where it lives on disk, behind one lock.
#[derive(Debug)]
struct Loaded {
    shelf: Shelf,
    book: Book,
}

type Slot = Arc<OnceCell<Arc<Mutex<Loaded>>>>;

pub struct AuthorIndex {
    dir: PathBuf,
    /// INVARIANT: held only to find or insert a slot, never across a load —
    /// one source's cold load must not stall every other source.
    books: Mutex<HashMap<String, Slot>>,
}

impl std::fmt::Debug for AuthorIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorIndex")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl AuthorIndex {
    /// # Errors
    ///
    /// I/O failure creating the index directory under `data_dir`.
    pub fn open(data_dir: &Path) -> std::io::Result<Self> {
        let dir = data_dir.join("authors");
        std::fs::create_dir_all(&dir)?;
        // Startup is the one moment no writer is running.
        file::sweep_temporaries(&dir);
        Ok(Self {
            dir,
            books: Mutex::new(HashMap::new()),
        })
    }

    /// Forget a mirror whose repository origin reports gone to a caller
    /// holding `fingerprint` — only if the entry recorded that credential.
    ///
    /// # Errors
    ///
    /// I/O failure removing its record; the in-memory book has forgotten it
    /// already, and a restart reloads it until origin is asked again.
    pub async fn forget(
        &self,
        tenant_id: &str,
        source_id: &str,
        dir_name: &str,
        fingerprint: &str,
    ) -> std::io::Result<()> {
        let loaded = self.loaded(tenant_id, source_id).await;
        let mut loaded = loaded.lock().await;
        if loaded.book.forget(dir_name, fingerprint) {
            loaded.shelf.remove_entry(dir_name)?;
        }
        Ok(())
    }

    /// Whether `refresh` describes a snapshot the index has not folded.
    pub async fn needs_fold(&self, refresh: &Refresh) -> bool {
        let loaded = self.loaded(&refresh.tenant_id, &refresh.source_id).await;
        let loaded = loaded.lock().await;
        loaded
            .book
            .needs_fold(&refresh.dir_name, &refresh.incarnation, refresh.generation)
    }

    /// Record one refresh; `authors` is `None` when the snapshot was already
    /// folded and only its credentials may have changed.
    ///
    /// # Errors
    ///
    /// I/O failure persisting what changed. The in-memory book keeps the
    /// update; the entry's on-disk record stays stale, so after a restart the
    /// entry is folded again and its changes are stamped again.
    pub async fn record(
        &self,
        refresh: Refresh,
        authors: Option<Vec<AuthorRow>>,
    ) -> std::io::Result<()> {
        let loaded = self.loaded(&refresh.tenant_id, &refresh.source_id).await;
        let mut loaded = loaded.lock_owned().await;

        // INVARIANT: the book lock spans the write, so two refreshes never
        // persist out of the order they were stamped in.
        tokio::task::spawn_blocking(move || {
            let observation = Observation {
                dir_name: refresh.dir_name,
                clone_url: refresh.clone_url,
                incarnation: refresh.incarnation,
                generation: refresh.generation,
                fingerprints: refresh.fingerprints,
                authors,
            };
            let Loaded { shelf, book } = &mut *loaded;
            let applied = book.apply(observation, cursor::now_us());
            shelf
                .persist(&applied.stamped, applied.entry.as_ref(), book)
                .inspect_err(|_| metrics::record_author_index_failure(AuthorIndexFailure::Persist))
        })
        .await
        .map_err(std::io::Error::other)?
    }

    /// One page of authors changed in `tenant_id`/`source_id`, visible to a
    /// caller holding `fingerprint`.
    ///
    pub async fn changes(
        &self,
        tenant_id: &str,
        source_id: &str,
        fingerprint: &str,
        query: FeedQuery<'_>,
    ) -> ChangesPage {
        let loaded = self.loaded(tenant_id, source_id).await;
        let loaded = loaded.lock().await;

        let cursor = query.cursor;
        let high_water_us = cursor.map_or(loaded.book.last_stamp_us(), |c| c.high_water_us);
        let committed_since_us = cursor.map_or(query.committed_since_us, |c| c.committed_since_us);
        let committed_since = committed_since_us.and_then(ordinal_of_us);
        let window = Window {
            since_us: cursor.map_or(query.since_us, |c| c.since_us),
            after: cursor.map(|c| (c.after_us, c.after_email.as_str())),
            high_water_us,
            committed_since: committed_since.as_deref(),
            limit: query.limit,
        };
        let (items, more) = loaded.book.changes(fingerprint, window);

        let next = more
            .then(|| items.last())
            .flatten()
            .map(|last| ChangesCursor {
                high_water_us,
                since_us: window.since_us,
                after_us: last.changed_us,
                after_email: last.email.clone(),
                committed_since_us,
            });
        ChangesPage { items, next }
    }

    async fn loaded(&self, tenant_id: &str, source_id: &str) -> Arc<Mutex<Loaded>> {
        let name = book_name(tenant_id, source_id);
        let slot: Slot = self
            .books
            .lock()
            .await
            .entry(name.clone())
            .or_default()
            .clone();

        slot.get_or_init(|| async {
            let dir = self.dir.join(name);
            let fallback = dir.clone();
            let (on_disk, book, damaged) = tokio::task::spawn_blocking(move || Shelf::load(dir))
                .await
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "author book load panicked; it starts empty");
                    (Shelf::empty(fallback), Book::default(), true)
                });
            if damaged {
                metrics::record_author_index_failure(AuthorIndexFailure::Load);
            }
            Arc::new(Mutex::new(Loaded {
                shelf: on_disk,
                book,
            }))
        })
        .await
        .clone()
    }
}

/// Directory name of a book: hex sha256 over the NUL-joined scope, so tenant
/// and source names never become paths.
fn book_name(tenant_id: &str, source_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(tenant_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(source_id.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; names carry pid/counter and hold no secrets
        let dir = std::env::temp_dir().join(format!(
            "git-cli-proxy-authors-{tag}-{}-{}",
            std::process::id(),
            crate::engine::meta::now_epoch_s()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn open(dir: &Path) -> AuthorIndex {
        match AuthorIndex::open(dir) {
            Ok(index) => index,
            Err(e) => panic!("open: {e}"),
        }
    }

    fn refresh(dir_name: &str, generation: u64, fingerprint: &str) -> Refresh {
        Refresh {
            tenant_id: "acme".to_owned(),
            source_id: "git-main".to_owned(),
            dir_name: dir_name.to_owned(),
            clone_url: format!("https://git.example.com/acme/{dir_name}.git"),
            incarnation: "inc".to_owned(),
            generation,
            fingerprints: vec![fingerprint.to_owned()],
        }
    }

    fn author(email: &str, count: u64) -> AuthorRow {
        AuthorRow {
            author_email: email.to_owned(),
            author_name: "Dev".to_owned(),
            sample_sha: "a".repeat(40),
            last_committed_date: "2026-01-01T00:00:00Z".to_owned(),
            commit_count: count,
        }
    }

    async fn record(index: &AuthorIndex, refresh: Refresh, authors: Vec<AuthorRow>) {
        if let Err(e) = index.record(refresh, Some(authors)).await {
            panic!("record: {e}");
        }
    }

    async fn feed(index: &AuthorIndex, fingerprint: &str) -> Vec<AuthorChange> {
        index
            .changes(
                "acme",
                "git-main",
                fingerprint,
                FeedQuery {
                    limit: 100,
                    ..FeedQuery::default()
                },
            )
            .await
            .items
    }

    #[tokio::test]
    async fn a_book_survives_a_restart_and_keeps_its_stamps() {
        let dir = temp_dir("restart");
        let index = open(&dir);
        record(
            &index,
            refresh("app", 1, "fp"),
            vec![author("dev@example.com", 1)],
        )
        .await;
        let before = feed(&index, "fp").await;

        let reopened = open(&dir);

        assert_eq!(
            before,
            feed(&reopened, "fp").await,
            "a restart must not re-stamp anybody"
        );
        assert!(!reopened.needs_fold(&refresh("app", 1, "fp")).await);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_stamp_issued_after_a_restart_sorts_after_every_earlier_one() {
        let dir = temp_dir("monotonic");
        let index = open(&dir);
        record(
            &index,
            refresh("app", 1, "fp"),
            vec![author("a@example.com", 1)],
        )
        .await;
        let first = feed(&index, "fp").await[0].changed_us;

        let reopened = open(&dir);
        record(
            &reopened,
            refresh("lib", 1, "fp"),
            vec![author("b@example.com", 1)],
        )
        .await;

        let stamps: Vec<i64> = feed(&reopened, "fp")
            .await
            .iter()
            .map(|c| c.changed_us)
            .collect();
        assert_eq!(stamps.len(), 2);
        assert!(
            stamps[1] > first,
            "stamps must keep increasing across a restart: {stamps:?}"
        );
    }

    #[tokio::test]
    async fn an_unreadable_entry_document_is_folded_again_instead_of_failing() {
        let dir = temp_dir("corrupt");
        let index = open(&dir);
        record(
            &index,
            refresh("app", 1, "fp"),
            vec![author("dev@example.com", 1)],
        )
        .await;
        let entry = dir
            .join("authors")
            .join(book_name("acme", "git-main"))
            .join("entries")
            .join("app.json");
        if let Err(e) = std::fs::write(&entry, b"{not json") {
            panic!("stage: {e}");
        }

        let reopened = open(&dir);

        assert!(reopened.needs_fold(&refresh("app", 1, "fp")).await);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn books_of_two_sources_never_mix() {
        let dir = temp_dir("scopes");
        let index = open(&dir);
        let other_source = Refresh {
            source_id: "git-other".to_owned(),
            ..refresh("app", 1, "fp")
        };
        record(&index, other_source, vec![author("dev@example.com", 1)]).await;

        assert!(
            feed(&index, "fp").await.is_empty(),
            "an author of another source must not leak into this one"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cursor_walks_every_page_of_one_snapshot() {
        let dir = temp_dir("paging");
        let index = open(&dir);
        let rows = (0..5)
            .map(|n| author(&format!("dev{n}@example.com"), 1))
            .collect();
        record(&index, refresh("app", 1, "fp"), rows).await;

        let mut seen = Vec::new();
        let mut cursor: Option<ChangesCursor> = None;
        loop {
            let page = index
                .changes(
                    "acme",
                    "git-main",
                    "fp",
                    FeedQuery {
                        cursor: cursor.as_ref(),
                        limit: 2,
                        ..FeedQuery::default()
                    },
                )
                .await;
            seen.extend(page.items.into_iter().map(|change| change.email));
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        assert_eq!(seen.len(), 5, "every author exactly once: {seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_cursor_carries_the_committed_since_bound_to_every_later_page() {
        let dir = temp_dir("bounded-paging");
        let index = open(&dir);
        let mut rows: Vec<AuthorRow> = (0..3)
            .map(|n| author(&format!("recent{n}@example.com"), 1))
            .collect();
        let mut old = author("old@example.com", 1);
        old.last_committed_date = "2023-06-01T00:00:00Z".to_owned();
        rows.insert(1, old);
        record(&index, refresh("app", 1, "fp"), rows).await;

        let Some(bound_us) = parse_instant_us("2024-01-01") else {
            panic!("the bound must parse");
        };
        let first = index
            .changes(
                "acme",
                "git-main",
                "fp",
                FeedQuery {
                    committed_since_us: Some(bound_us),
                    limit: 1,
                    ..FeedQuery::default()
                },
            )
            .await;
        let mut seen: Vec<String> = first.items.into_iter().map(|change| change.email).collect();

        let mut cursor = first.next;
        while let Some(next) = cursor {
            let page = index
                .changes(
                    "acme",
                    "git-main",
                    "fp",
                    FeedQuery {
                        cursor: Some(&next),
                        limit: 1,
                        ..FeedQuery::default()
                    },
                )
                .await;
            seen.extend(page.items.into_iter().map(|change| change.email));
            cursor = page.next;
        }

        assert_eq!(
            seen.len(),
            3,
            "later pages must keep the first page's bound: {seen:?}"
        );
        assert!(
            !seen.iter().any(|email| email == "old@example.com"),
            "an author older than the bound must stay out of every page: {seen:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
