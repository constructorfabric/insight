use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock, Semaphore, watch};

use super::disk::{Budget, Candidate, Reclaim, dir_size, needs_consolidation};
use super::key::CacheKey;
use super::meta::{RepoMeta, now_epoch_s};
use super::metrics::{self, DiskGauges, EvictionTier, FetchResult};
use super::runner::{GitCredentials, GitError, GitRunner};

const INLINE_WAIT: Duration = Duration::from_secs(15);
const COLD_RETRY_AFTER: Duration = Duration::from_secs(30);
const REPROOF_ATTEMPTS: usize = 2;
const BARE_REFSPEC: &str = "+refs/heads/*:refs/heads/*";
/// How often one entry's on-disk size is re-measured after being served.
const DRIFT_CHECK_INTERVAL: Duration = Duration::from_mins(1);
/// Consecutive drift checks that may find the entry busy and over threshold
/// before the repack stops being opportunistic and takes a real write lock.
const PURGE_ESCALATION_AFTER: u32 = 3;

/// Why a refresh failed, in a form that survives being broadcast to every
/// waiter (`GitError` is not `Clone`).
#[derive(Debug, Clone)]
pub enum RefreshFailure {
    Auth,
    NotFound,
    OriginUnavailable,
    PromisorRefused,
    AdmissionRejected,
    Throttled,
    Timeout,
    TooLarge { cap_bytes: u64 },
    Other(String),
}

impl From<&GitError> for RefreshFailure {
    fn from(error: &GitError) -> Self {
        match error {
            GitError::AuthRejected => Self::Auth,
            GitError::NotFound => Self::NotFound,
            GitError::OriginUnavailable => Self::OriginUnavailable,
            GitError::PromisorRefused => Self::PromisorRefused,
            GitError::AdmissionRejected | GitError::TransientlyOverCap => Self::AdmissionRejected,
            GitError::Throttled => Self::Throttled,
            GitError::TimedOut(_) => Self::Timeout,
            GitError::Failed(message) => Self::Other(message.clone()),
            GitError::Io(e) => Self::Other(e.to_string()),
            GitError::TooLarge { cap_bytes } => Self::TooLarge {
                cap_bytes: *cap_bytes,
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("origin rejected the supplied git credentials")]
    AuthRejected,
    #[error("repository not found at origin")]
    NotFound,
    #[error("origin declines to serve the repository")]
    OriginUnavailable,
    #[error("origin refuses to serve explicitly requested objects")]
    PromisorRefused,
    #[error("repository is being prepared; retry in {}s", retry_after.as_secs())]
    Busy { retry_after: Duration },
    #[error("origin is throttling this client")]
    Throttled,
    #[error("repository snapshot changed (current generation {current})")]
    SnapshotChanged { current: u64 },
    #[error("git failed: {0}")]
    Git(String),
    #[error("cache I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("repository exceeds the per-repository size cap of {cap_bytes} bytes")]
    TooLarge { cap_bytes: u64 },
}

impl From<RefreshFailure> for StoreError {
    fn from(failure: RefreshFailure) -> Self {
        match failure {
            RefreshFailure::Auth => Self::AuthRejected,
            RefreshFailure::NotFound => Self::NotFound,
            RefreshFailure::OriginUnavailable => Self::OriginUnavailable,
            RefreshFailure::PromisorRefused => Self::PromisorRefused,
            // §3.6: nothing could be freed, so the caller is asked to come
            // back rather than being served a half-prepared cache.
            RefreshFailure::AdmissionRejected => Self::Busy {
                retry_after: COLD_RETRY_AFTER,
            },
            RefreshFailure::Throttled => Self::Throttled,
            RefreshFailure::Timeout => Self::Git("git timed out".to_owned()),
            RefreshFailure::TooLarge { cap_bytes } => Self::TooLarge { cap_bytes },
            RefreshFailure::Other(message) => Self::Git(message),
        }
    }
}

impl From<GitError> for StoreError {
    fn from(error: GitError) -> Self {
        RefreshFailure::from(&error).into()
    }
}

/// What a reclaim-path blob purge did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlobPurge {
    Purged,
    /// Nothing to purge, or the entry has readers right now.
    Skipped,
    /// The heavy permit is busy with a clone or fetch.
    PermitBusy,
}

/// How a caller wants the snapshot resolved.
///
/// INVARIANT: `Pinned` never contacts origin — a paginating caller stays on
/// the snapshot its first page observed, so pages cannot straddle a fetch.
#[derive(Debug, Clone)]
pub enum Freshness {
    Refresh {
        max_staleness: Duration,
    },
    Pinned {
        generation: u64,
        incarnation: String,
    },
}

/// Read access to one cached repository. Holding the guard pins the entry:
/// fetch/repack/eviction take the write side and wait for readers to drain.
pub struct RepoGuard {
    git_dir: PathBuf,
    incarnation: String,
    generation: u64,
    _read: OwnedRwLockReadGuard<()>,
}

impl RepoGuard {
    #[must_use]
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Which clone of the entry this guard is reading. A cursor minted here
    /// carries it, so a continuation cannot land on a re-cloned entry.
    #[must_use]
    pub fn incarnation(&self) -> &str {
        &self.incarnation
    }
}

type RefreshResult = Result<u64, RefreshFailure>;

/// Headroom promised to one in-flight heavy operation, released on drop.
///
/// Admission without it only ever asks whether the cache is full RIGHT NOW.
/// Several cold clones can each be told yes before any of them has written a
/// byte, and then collectively overrun the budget — the caller sees a git or
/// I/O failure where it should have seen a `429`.
#[derive(Debug)]
pub struct Reservation<'a> {
    reserved: &'a AtomicU64,
    bytes: u64,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        // `fetch_sub` cannot underflow here: every reservation subtracts
        // exactly what it added, once.
        self.reserved.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// One entry's drift bookkeeping: the re-measurement throttle and how many
/// repacks in a row lost the non-blocking lock probe to readers.
#[derive(Debug)]
struct DriftState {
    checked: Instant,
    losses: u32,
}

/// What a background flight is supposed to do to the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RefreshKind {
    /// Clone when absent, fetch when stale or unproven.
    Sync,
    /// Rebuild the entry as a full clone (§ origin refuses promisor wants).
    Promote,
}

/// Identity of one in-flight flight: the entry, the credentials driving it,
/// and the work it performs. Promotion must not adopt a plain sync's result.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FlightKey {
    dir_name: String,
    cred_fingerprint: String,
    kind: RefreshKind,
}

pub struct RepoStore {
    data_dir: PathBuf,
    runner: GitRunner,
    budget: Budget,
    max_repo_bytes: u64,
    heavy: Semaphore,
    entries: Mutex<HashMap<String, Arc<RwLock<()>>>>,
    inflight: Mutex<HashMap<FlightKey, watch::Receiver<Option<RefreshResult>>>>,
    tmp_counter: AtomicU64,
    gauges: Arc<DiskGauges>,
    /// Per entry: when its on-disk size was last re-measured (without the
    /// throttle a 200-page walk pays a full `dir_size` per page), and how many
    /// checks in a row wanted a repack but lost the lock probe to readers.
    drift: Mutex<HashMap<String, DriftState>>,
    /// Bytes promised to heavy operations that have been admitted but have not
    /// finished writing them.
    reserved_bytes: AtomicU64,
    /// Serialises decide-and-reserve. Two callers that each read usage before
    /// either reserved would both be admitted against the same headroom.
    admission: Mutex<()>,
}

impl RepoStore {
    /// # Errors
    ///
    /// I/O failure creating the cache directories under `data_dir`.
    pub fn new(data_dir: &Path, heavy_ops_concurrency: usize) -> Result<Self, StoreError> {
        Self::open_cache(
            data_dir,
            heavy_ops_concurrency,
            None,
            Budget {
                total_bytes: u64::MAX,
            },
            u64::MAX,
        )
    }

    /// # Errors
    ///
    /// I/O failure creating the cache directories under `data_dir`.
    pub fn open_cache(
        data_dir: &Path,
        heavy_ops_concurrency: usize,
        ca_cert_path: Option<String>,
        budget: Budget,
        max_repo_bytes: u64,
    ) -> Result<Self, StoreError> {
        std::fs::create_dir_all(data_dir.join("repos"))?;
        // A clone stages under tmp/ and is renamed into place on success. A
        // crash mid-clone strands it, and nothing else ever collects it: the
        // reclaim planner only walks repos/, so the loss is invisible to the
        // budget and permanent. Startup is the one moment no clone is running.
        let tmp = data_dir.join("tmp");
        if tmp.is_dir()
            && let Err(e) = std::fs::remove_dir_all(&tmp)
        {
            tracing::warn!(error = %e, "could not clear staging leftovers; stranded clones stay invisible to the budget");
        }
        std::fs::create_dir_all(&tmp)?;

        // An entry whose metadata is missing or unreadable — a crash between
        // moving refs and publishing, or a failed publish — can never be
        // served, never matches a cursor, and is invisible to the reclaim
        // planner: without this sweep its bytes are leaked until someone
        // requests the same repository again.
        if let Ok(entries) = std::fs::read_dir(data_dir.join("repos")) {
            for entry in entries.filter_map(Result::ok) {
                let dir = entry.path();
                if dir.is_dir() && RepoMeta::load(&dir).is_none() {
                    match std::fs::remove_dir_all(&dir) {
                        Ok(()) => {
                            tracing::info!(dir = %dir.display(), "removed an entry without readable metadata");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, dir = %dir.display(), "could not remove a metadata-less entry");
                        }
                    }
                }
            }
        }
        Ok(Self {
            data_dir: data_dir.to_owned(),
            runner: GitRunner::new().with_ca_cert(ca_cert_path),
            budget,
            max_repo_bytes,
            heavy: Semaphore::new(heavy_ops_concurrency),
            entries: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            tmp_counter: AtomicU64::new(0),
            gauges: Arc::new(DiskGauges::default()),
            drift: Mutex::new(HashMap::new()),
            reserved_bytes: AtomicU64::new(0),
            admission: Mutex::new(()),
        })
    }

    #[must_use]
    pub fn runner(&self) -> &GitRunner {
        &self.runner
    }

    /// The disk figures the §4.3 gauges observe. Shared so the collector's
    /// callback reads a cached snapshot instead of hitting the filesystem.
    #[must_use]
    pub fn gauges(&self) -> &Arc<DiskGauges> {
        &self.gauges
    }

    /// A value unique to one clone of one entry.
    ///
    /// Uniqueness comes from the same three things that already make a staging
    /// directory unique — process, wall clock, and a per-store counter — so no
    /// new source of entropy is introduced for a value that is only ever
    /// compared for equality.
    fn mint_incarnation(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(std::process::id().to_le_bytes());
        hasher.update(now_epoch_s().to_le_bytes());
        hasher.update(
            self.tmp_counter
                .fetch_add(1, Ordering::Relaxed)
                .to_le_bytes(),
        );
        hex::encode(hasher.finalize())[..16].to_owned()
    }

    fn entry_dir(&self, key: &CacheKey) -> PathBuf {
        self.data_dir.join("repos").join(key.dir_name())
    }

    async fn entry_lock(&self, key: &CacheKey) -> Arc<RwLock<()>> {
        let mut entries = self.entries.lock().await;
        entries.entry(key.dir_name()).or_default().clone()
    }

    /// One slot of the global heavy-ops cap.
    ///
    /// INVARIANT: acquired AFTER the entry's write lock, never before —
    /// fetch/clone/promote and the escalated purge all order write-lock →
    /// permit, and a permit-first path would deadlock against them (permit
    /// holder waits for the write lock a lock holder needs a permit to
    /// release). Never acquired while already holding a permit.
    async fn heavy_permit(&self) -> tokio::sync::SemaphorePermit<'_> {
        // The semaphore is never closed, so acquire cannot fail.
        match self.heavy.acquire().await {
            Ok(permit) => permit,
            Err(_) => unreachable!("the heavy semaphore is never closed"),
        }
    }

    /// Resolve `key` into a readable snapshot.
    ///
    /// `Refresh` clones when absent and fetches when stale or when the caller
    /// presents credentials that have not proved origin access for this entry;
    /// slow preparation returns [`StoreError::Busy`] while the work continues
    /// in the background. `Pinned` serves the requested generation or fails
    /// with [`StoreError::SnapshotChanged`].
    ///
    /// # Errors
    ///
    /// [`StoreError`] — typed origin failures, `Busy` while preparing, or
    /// `SnapshotChanged` when a pinned snapshot is gone.
    pub async fn open(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
        freshness: Freshness,
    ) -> Result<RepoGuard, StoreError> {
        let lock = self.entry_lock(key).await;
        let entry_dir = self.entry_dir(key);
        let git_dir = entry_dir.join("repo.git");
        let fingerprint = creds.fingerprint();

        match freshness {
            Freshness::Pinned {
                generation,
                incarnation,
            } => {
                let read = read_within(&lock, INLINE_WAIT).await?;
                let meta = usable_meta(&entry_dir, &fingerprint)
                    .ok_or(StoreError::SnapshotChanged { current: 0 })?;
                // INVARIANT: both must match. An entry evicted and re-cloned
                // between two pages is back at generation 1 with a different
                // history, and the generation alone cannot see that.
                if meta.generation != generation || meta.incarnation != incarnation {
                    return Err(StoreError::SnapshotChanged {
                        current: meta.generation,
                    });
                }
                let incarnation = meta.incarnation.clone();
                touch_access(&entry_dir, meta);
                Ok(RepoGuard {
                    git_dir,
                    incarnation,
                    generation,
                    _read: read,
                })
            }
            Freshness::Refresh { max_staleness } => {
                {
                    let read = read_within(&lock, INLINE_WAIT).await?;
                    if let Some(meta) = fresh_meta(&entry_dir, &fingerprint, max_staleness) {
                        let generation = meta.generation;
                        let incarnation = meta.incarnation.clone();
                        touch_access(&entry_dir, meta);
                        return Ok(RepoGuard {
                            git_dir,
                            incarnation,
                            generation,
                            _read: read,
                        });
                    }
                }

                // INVARIANT: a refresh joined from the in-flight map may have
                // been driven by a DIFFERENT caller's credentials, so the
                // generation it publishes is not by itself proof that THIS
                // caller may read the entry. Re-check the fingerprint under
                // the read guard and refresh again on a mismatch.
                for _ in 0..REPROOF_ATTEMPTS {
                    self.await_refresh(key, creds, max_staleness, RefreshKind::Sync)
                        .await?;

                    let read = read_within(&lock, INLINE_WAIT).await?;
                    if let Some(meta) = usable_meta(&entry_dir, &fingerprint) {
                        return Ok(RepoGuard {
                            git_dir,
                            incarnation: meta.incarnation,
                            generation: meta.generation,
                            _read: read,
                        });
                    }
                }

                // Two callers with different valid credentials can keep
                // invalidating each other's proof. Bounded, and the loser is
                // told to retry rather than served someone else's snapshot.
                Err(StoreError::Busy {
                    retry_after: COLD_RETRY_AFTER,
                })
            }
        }
    }

    /// Join (or start) the background refresh for `key` and wait a bounded
    /// slice of time for it. A cold clone outlives the request: the caller
    /// gets `Busy` and the task keeps running, so no HTTP request ever hangs
    /// for the length of a clone and no clone is cancelled by a client giving
    /// up.
    async fn await_refresh(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
        max_staleness: Duration,
        kind: RefreshKind,
    ) -> Result<u64, StoreError> {
        let mut receiver = self.refresh_task(key, creds, max_staleness, kind).await;

        let waited = tokio::time::timeout(INLINE_WAIT, receiver.wait_for(Option::is_some)).await;
        match waited {
            Ok(Ok(seen)) => match seen.clone() {
                Some(Ok(generation)) => Ok(generation),
                Some(Err(failure)) => Err(failure.into()),
                None => Err(StoreError::Busy {
                    retry_after: COLD_RETRY_AFTER,
                }),
            },
            // Sender dropped without publishing, or the wait timed out: the
            // work is still owned by the background task either way.
            Ok(Err(_)) | Err(_) => Err(StoreError::Busy {
                retry_after: COLD_RETRY_AFTER,
            }),
        }
    }

    /// INVARIANT: flights are keyed per credential, not per entry. Joining a
    /// flight means adopting its result, and a flight started with someone
    /// else's credentials proves nothing about this caller's access. Callers
    /// presenting the same credentials — the ordinary case of one connector
    /// running several streams — still collapse onto one clone.
    async fn refresh_task(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
        max_staleness: Duration,
        kind: RefreshKind,
    ) -> watch::Receiver<Option<RefreshResult>> {
        let flight = FlightKey {
            dir_name: key.dir_name(),
            cred_fingerprint: creds.fingerprint(),
            kind,
        };

        let mut inflight = self.inflight.lock().await;
        if let Some(existing) = inflight.get(&flight) {
            return existing.clone();
        }

        let (sender, receiver) = watch::channel(None);
        inflight.insert(flight.clone(), receiver.clone());
        drop(inflight);

        let store = self.clone();
        let key = key.clone();
        let creds = creds.clone();
        tokio::spawn(async move {
            // The work runs in its own task so a panic inside it becomes a
            // JoinError here instead of skipping the cleanup below — a flight
            // that dies without removing itself leaves a dead receiver in the
            // map, and every later request for this entry answers Busy until
            // the process restarts.
            let work = tokio::spawn({
                let store = Arc::clone(&store);
                let key = key.clone();
                let creds = creds.clone();
                async move {
                    match kind {
                        RefreshKind::Sync => store.refresh(&key, &creds, max_staleness).await,
                        RefreshKind::Promote => store.promote(&key, &creds).await,
                    }
                }
            });
            let outcome = match work.await {
                Ok(result) => result,
                Err(join_error) => Err(GitError::Failed(format!(
                    "refresh task died before finishing: {join_error}"
                ))),
            };
            let published = match &outcome {
                Ok(generation) => Ok(*generation),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        dir = %key.dir_name(),
                        tenant_id = %key.tenant_id,
                        source_id = %key.source_id,
                        kind = ?kind,
                        "background refresh failed"
                    );
                    Err(RefreshFailure::from(error))
                }
            };
            store.inflight.lock().await.remove(&flight);
            let _ = sender.send(Some(published));
        });

        receiver
    }

    async fn refresh(
        &self,
        key: &CacheKey,
        creds: &GitCredentials,
        max_staleness: Duration,
    ) -> Result<u64, GitError> {
        let lock = self.entry_lock(key).await;
        let entry_dir = self.entry_dir(key);
        let git_dir = entry_dir.join("repo.git");
        let fingerprint = creds.fingerprint();

        // INVARIANT: clone/fetch hold the WRITE side — readers never observe a
        // half-updated ref set, so every command of one sync sees one snapshot.
        let _write = lock.write().await;

        if let Some(meta) = fresh_meta(&entry_dir, &fingerprint, max_staleness) {
            return Ok(meta.generation);
        }
        if git_dir.is_dir() {
            self.fetch(key, &entry_dir, &git_dir, creds).await
        } else {
            self.clone(key, &entry_dir, &git_dir, creds).await
        }
    }

    async fn clone(
        &self,
        key: &CacheKey,
        entry_dir: &Path,
        git_dir: &Path,
        creds: &GitCredentials,
    ) -> Result<u64, GitError> {
        // Reclaim BEFORE taking disk, not after: an admission check that runs
        // post-clone has already overshot the budget.
        // INVARIANT: the reservation lives as long as the operation does. Drop
        // it early and a concurrent caller is admitted against headroom this
        // one has not finished consuming.
        let Some(_reserved) = self.admit(entry_dir).await else {
            return Err(GitError::AdmissionRejected);
        };

        // INVARIANT: the permit spans the whole clone — the semaphore IS the
        // global heavy-ops cap.
        let _permit = self.heavy_permit().await;

        let tmp = self.data_dir.join("tmp").join(format!(
            "clone-{}-{}",
            std::process::id(),
            self.tmp_counter.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&tmp);

        let tmp_str = tmp.to_string_lossy().into_owned();
        metrics::record_cold_clone();
        let cloned = self
            .runner
            .run_capped(
                None,
                &clone_argv(key.clone_url.as_str(), &tmp_str),
                Some(creds),
                &tmp,
                self.max_repo_bytes,
            )
            .await;
        if let Err(e) = cloned {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }

        // Bare clones ship without a fetch refspec; pin the standard mirror
        // refspec once so every later `fetch --prune` updates + prunes heads.
        self.runner
            .run(
                Some(&tmp),
                &["config", "remote.origin.fetch", BARE_REFSPEC],
                None,
            )
            .await?;

        let cloned_bytes = dir_size_off_reactor(tmp.clone()).await;
        if cloned_bytes > self.max_repo_bytes {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(GitError::TooLarge {
                cap_bytes: self.max_repo_bytes,
            });
        }

        std::fs::create_dir_all(entry_dir).map_err(GitError::Io)?;
        std::fs::rename(&tmp, git_dir).map_err(GitError::Io)?;

        self.build_page_index(git_dir, 1, creds).await;
        // The sizes were measured before the index existed; fold in its own
        // length rather than paying a second whole-tree walk. The index is
        // part of the skeleton: a blob purge keeps it.
        let cloned_bytes = cloned_bytes + index_len(git_dir, 1);

        let now = now_epoch_s();
        let meta = RepoMeta {
            clone_url: key.clone_url.as_str().to_owned(),
            tenant_id: key.tenant_id.clone(),
            source_id: key.source_id.clone(),
            last_fetched_at_epoch_s: now,
            last_accessed_at_epoch_s: now,
            size_bytes: cloned_bytes,
            skeleton_bytes: cloned_bytes,
            generation: 1,
            incarnation: self.mint_incarnation(),
            cred_fingerprints: vec![creds.fingerprint()],
            full_clone: false,
        };
        meta.store(entry_dir).map_err(GitError::Io)?;
        // The one place the dir hash is tied to its tenant and source: every
        // later log line names only the hash, and this line is what lets an
        // operator resolve it without reading meta.json off the volume.
        tracing::info!(
            dir = %key.dir_name(),
            tenant_id = %key.tenant_id,
            source_id = %key.source_id,
            "entry created"
        );
        Ok(meta.generation)
    }

    async fn fetch(
        &self,
        key: &CacheKey,
        entry_dir: &Path,
        git_dir: &Path,
        creds: &GitCredentials,
    ) -> Result<u64, GitError> {
        // INVARIANT: the reservation lives as long as the operation does. Drop
        // it early and a concurrent caller is admitted against headroom this
        // one has not finished consuming.
        let Some(_reserved) = self.admit(entry_dir).await else {
            return Err(GitError::AdmissionRejected);
        };

        // INVARIANT: the permit spans the whole fetch — the semaphore IS the
        // global heavy-ops cap.
        let permit = self.heavy_permit().await;

        // Shed the last window's blobs FIRST, while this task holds the write
        // side. The cap judges what the entry persistently costs; transient
        // blob weight left behind by a purge that lost its `try_write` race
        // would otherwise be counted against it, and a healthy warm entry
        // would be deleted and answered `413` — which §4.4 tells the connector
        // never to retry. The mid-run watcher below would trip on the same
        // weight, permanently, on its very first poll.
        if let Err(e) = self.repack_if_drifted(entry_dir, &permit).await {
            tracing::warn!(error = %e, "pre-fetch purge failed; the cap check may see transient weight");
        }

        let before = self.ref_digest(git_dir).await;
        let previous = RepoMeta::load(entry_dir);

        // Park the metadata before the refs can move. A crash between the
        // `--atomic` fetch and the meta publish would otherwise leave the OLD
        // document — old generation, old fingerprint — describing the NEW
        // refs, and a pinned cursor would silently validate against a
        // snapshot it never saw. Parked, a crash leaves a meta-less entry:
        // treated as absent, re-cloned, and every old cursor answers 409.
        park_meta(entry_dir);

        let fetched = self
            .runner
            .run_capped(
                Some(git_dir),
                &["fetch", "--prune", "--atomic", "origin", BARE_REFSPEC],
                Some(creds),
                git_dir,
                self.max_repo_bytes,
            )
            .await;
        if let Err(e) = fetched {
            // `--atomic` means a failed fetch moved nothing: the parked
            // document is still true, and restoring it keeps a warm entry
            // serveable through a transient origin outage.
            unpark_meta(entry_dir);
            metrics::record_fetch(FetchResult::Error);
            return Err(e);
        }

        self.track_origin_head(git_dir, creds).await;

        // INVARIANT: the generation identifies a REF SNAPSHOT, not a fetch
        // attempt. Bumping it when nothing moved would 409 every page token
        // already in flight — and a sync outliving the staleness window
        // refreshes routinely, so that is the common case, not a rare one.
        let after = self.ref_digest(git_dir).await;
        let unchanged = before.is_some() && before == after;

        let fetched_bytes = dir_size_off_reactor(git_dir.to_path_buf()).await;
        if fetched_bytes > self.max_repo_bytes {
            // The meta is parked, so even a failed removal leaves the entry
            // reading as absent — never as its pre-fetch snapshot.
            if let Err(e) = std::fs::remove_dir_all(entry_dir) {
                tracing::error!(error = %e, "could not remove an over-cap entry; it stays invalidated on disk");
            }
            self.drift.lock().await.remove(&key.dir_name());
            return Err(GitError::TooLarge {
                cap_bytes: self.max_repo_bytes,
            });
        }

        metrics::record_fetch(if unchanged {
            FetchResult::Noop
        } else {
            FetchResult::Updated
        });

        let now = now_epoch_s();
        let generation = match (&previous, unchanged) {
            (Some(meta), true) => meta.generation,
            (previous, _) => previous.as_ref().map_or(0, |m| m.generation) + 1,
        };

        // A no-op fetch keeps its generation and normally its index too; the
        // exception is an entry cloned before indexes existed, which upgrades
        // here instead of walking history on every page forever.
        if !unchanged || !super::index::index_path(git_dir, generation).is_file() {
            self.build_page_index(git_dir, generation, creds).await;
        }
        let fetched_bytes = fetched_bytes + index_len(git_dir, generation);

        let meta = RepoMeta {
            clone_url: key.clone_url.as_str().to_owned(),
            tenant_id: key.tenant_id.clone(),
            source_id: key.source_id.clone(),
            last_fetched_at_epoch_s: now,
            last_accessed_at_epoch_s: now,
            size_bytes: fetched_bytes,
            // A fetch adds objects but does not change the blobless baseline.
            skeleton_bytes: previous
                .as_ref()
                .map_or(fetched_bytes, |m| m.skeleton_bytes.min(fetched_bytes)),
            generation,
            // A fetch moves refs inside the SAME clone: the incarnation is a
            // property of the directory, not of the ref snapshot.
            incarnation: previous
                .as_ref()
                .map_or_else(|| self.mint_incarnation(), |m| m.incarnation.clone()),
            cred_fingerprints: RepoMeta::proofs_with(previous.as_ref(), creds.fingerprint()),
            // A plain fetch never changes the entry's clone shape.
            full_clone: previous.as_ref().is_some_and(|m| m.full_clone),
        };
        publish_meta(&meta, entry_dir)?;
        discard_parked_meta(entry_dir);
        Ok(generation)
    }

    /// Rebuild the entry as a full clone.
    ///
    /// Some origins serve a plain clone but refuse explicit promisor wants for
    /// individual objects (GitLab fork-network object pools do this), which
    /// makes a blobless clone permanently unreadable: every blob prefetch
    /// fails, on every retry. Heal it once by dropping the filter, refetching
    /// everything, and recording the entry as no longer partial.
    ///
    /// # Errors
    ///
    /// [`StoreError`] — typed origin failures, `Busy` while the rebuild runs,
    /// or `TooLarge` when the full clone exceeds the per-repository cap.
    pub async fn promote_to_full_clone(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
    ) -> Result<u64, StoreError> {
        self.await_refresh(key, creds, Duration::ZERO, RefreshKind::Promote)
            .await
    }

    async fn promote(&self, key: &CacheKey, creds: &GitCredentials) -> Result<u64, GitError> {
        let lock = self.entry_lock(key).await;
        let entry_dir = self.entry_dir(key);
        let git_dir = entry_dir.join("repo.git");

        let _write = lock.write().await;

        let previous = RepoMeta::load(&entry_dir);
        if let Some(meta) = &previous
            && meta.full_clone
        {
            return Ok(meta.generation);
        }
        if !git_dir.is_dir() {
            return Err(GitError::NotFound);
        }

        // A full clone is much larger than the skeleton it replaces.
        // INVARIANT: the reservation lives as long as the operation does. Drop
        // it early and a concurrent caller is admitted against headroom this
        // one has not finished consuming.
        let Some(_reserved) = self.admit(&entry_dir).await else {
            return Err(GitError::AdmissionRejected);
        };

        // INVARIANT: the permit spans the whole promotion — the semaphore IS
        // the global heavy-ops cap.
        let _permit = self.heavy_permit().await;

        // Park the metadata before the first mutation. Unlike fetch there is
        // no restore on failure: the refetch below is not `--atomic`, so a
        // failure can leave refs (and the promisor config stripped next)
        // half-moved — the parked state honestly reads as "this entry no
        // longer matches any published snapshot", and the recovery is a
        // re-clone.
        park_meta(&entry_dir);

        // Both must go BEFORE the refetch, or it re-applies the blob filter.
        for key_name in ["remote.origin.promisor", "remote.origin.partialclonefilter"] {
            let _ = self
                .runner
                .run(Some(&git_dir), &["config", "--unset-all", key_name], None)
                .await;
        }

        // The hungriest operation in the service: a refetch pulls every blob
        // in history, which is exactly what the cap exists for.
        self.runner
            .run_capped(
                Some(&git_dir),
                &["fetch", "--refetch", "--prune", "origin", BARE_REFSPEC],
                Some(creds),
                &git_dir,
                self.max_repo_bytes,
            )
            .await?;

        remove_promisor_markers(&git_dir);

        self.runner
            .run_heavy(
                Some(&git_dir),
                &["repack", "-a", "-d", "--no-write-bitmap-index"],
                Some(creds),
            )
            .await?;

        let promoted_bytes = dir_size_off_reactor(git_dir.clone()).await;
        if promoted_bytes > self.max_repo_bytes {
            // The meta is parked, so even a failed removal leaves the entry
            // reading as absent — never as its pre-promotion snapshot.
            if let Err(e) = std::fs::remove_dir_all(&entry_dir) {
                tracing::error!(error = %e, "could not remove an over-cap entry; it stays invalidated on disk");
            }
            self.drift.lock().await.remove(&key.dir_name());
            return Err(GitError::TooLarge {
                cap_bytes: self.max_repo_bytes,
            });
        }

        let now = now_epoch_s();
        let generation = previous.as_ref().map_or(0, |m| m.generation) + 1;
        self.build_page_index(&git_dir, generation, creds).await;
        let promoted_bytes = promoted_bytes + index_len(&git_dir, generation);
        let meta = RepoMeta {
            clone_url: key.clone_url.as_str().to_owned(),
            tenant_id: key.tenant_id.clone(),
            source_id: key.source_id.clone(),
            last_fetched_at_epoch_s: now,
            last_accessed_at_epoch_s: now,
            size_bytes: promoted_bytes,
            // The full clone IS the baseline now: a purge could free nothing
            // it could ever fetch back.
            skeleton_bytes: promoted_bytes,
            generation,
            // Rebuilt in place: same directory, so same incarnation. The
            // generation bump is what a pinned cursor trips over.
            incarnation: previous
                .as_ref()
                .map_or_else(|| self.mint_incarnation(), |m| m.incarnation.clone()),
            cred_fingerprints: RepoMeta::proofs_with(previous.as_ref(), creds.fingerprint()),
            full_clone: true,
        };
        publish_meta(&meta, &entry_dir)?;
        discard_parked_meta(&entry_dir);
        Ok(generation)
    }

    /// Refuse to start on a git that cannot perform the blob purge.
    ///
    /// The purge is the mechanism behind the whole blobless design (§3.3): an
    /// entry that cannot shed the blobs a window pulled grows until eviction
    /// throws the whole repository away. `repack --filter-to` is what makes it
    /// work, and a git without it does not fail loudly — it exits non-zero on
    /// one repack, inside a background task, and the symptom is disk pressure
    /// weeks later. Boot is the honest place to say so.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when git cannot be probed, or rejects the invocation.
    pub async fn require_purge_support(&self) -> Result<(), StoreError> {
        let probe = self.data_dir.join("tmp").join("purge-probe.git");
        let _ = std::fs::remove_dir_all(&probe);

        self.runner
            .run(
                None,
                &["init", "--bare", "--quiet", &probe.to_string_lossy()],
                None,
            )
            .await?;
        let evicted = probe.join("evicted");
        std::fs::create_dir_all(&evicted)?;

        let filter_to = format!("--filter-to={}", evicted.display());
        let probed = self
            .runner
            .run(
                Some(&probe),
                &[
                    "repack",
                    "-a",
                    "-d",
                    "--filter=blob:none",
                    &filter_to,
                    "--no-write-bitmap-index",
                ],
                None,
            )
            .await;
        let _ = std::fs::remove_dir_all(&probe);

        probed.map(|_| ()).map_err(StoreError::from)
    }

    /// Return an entry to its skeleton once a served window has left blobs
    /// behind, and in every case re-measure it.
    ///
    /// The re-measurement is not incidental. `blobs::prefetch` grows the entry
    /// and writes nothing back, so `size_bytes` stays at whatever the last
    /// clone or purge recorded — under which the reclaim planner believes
    /// every entry is skeleton-sized, never plans the cheap purge tier, and
    /// evicts whole warm repositories instead.
    ///
    /// Best-effort throughout: a reader holding the entry, unreadable metadata
    /// or a failed repack all leave the entry as it is. The reclaim path is
    /// the backstop.
    pub async fn purge_if_drifted(&self, key: &CacheKey) {
        if !self.drift_check_due(&key.dir_name()).await {
            return;
        }

        let entry_dir = self.entry_dir(key);
        let git_dir = entry_dir.join("repo.git");
        if !git_dir.is_dir() {
            return;
        }

        let lock = self.entry_lock(key).await;

        // The measurement runs under the READ side: readers coexist, so it
        // cannot re-starve the accounting the way the old write probe did —
        // and it excludes the write-side publishers. Lock-free load-measure-
        // store here clobbered a concurrent fetch's meta: the walk takes
        // seconds, and storing the pre-walk document rolled generation and
        // credentials back, answering 409 to every cursor the fetch had just
        // validated. A concurrent `touch_access` can still lose one LRU bump
        // to this write; that is the documented best-effort trade.
        let (measured, meta) = {
            let Ok(_read) = read_within(&lock, INLINE_WAIT).await else {
                // A writer holds the entry; it publishes fresh sizes itself.
                return;
            };
            let Some(mut meta) = RepoMeta::load(&entry_dir) else {
                return;
            };
            let measured = dir_size_off_reactor(git_dir.clone()).await;
            if meta.size_bytes != measured {
                meta.size_bytes = measured;
                if let Err(e) = meta.store(&entry_dir) {
                    tracing::warn!(error = %e, dir = %key.dir_name(), "could not record the measured size; the planner will understate this entry");
                }
            }
            (measured, meta)
        };

        // A promoted entry holds the only copy of its blobs: origin refuses to
        // serve them again, so purging would strand it. Pack count is its own
        // trigger: every page's prefetch adds packs, and object lookup slows
        // with each one, so a byte threshold alone lets a backfill of small
        // blobs degrade every git invocation without ever tripping a purge.
        let packs = pack_count(&git_dir);
        if meta.full_clone || !needs_consolidation(measured, meta.skeleton_bytes, packs) {
            self.settle_purge_debt(&key.dir_name()).await;
            return;
        }

        // INVARIANT: repack DELETES packs, so it takes the write side. It is
        // opportunistic first — a reader that still holds the entry gets
        // served, not repacked under — but under continuous paging the probe
        // NEVER wins, so after enough losses it queues for the lock like a
        // fetch would. Readers wait out one repack; the alternative is an
        // entry that grows for as long as anyone keeps reading it.
        let _write = if let Ok(guard) = lock.try_write() {
            guard
        } else {
            if !self.purge_debt_due(&key.dir_name()).await {
                return;
            }
            metrics::record_purge_escalation();
            tracing::info!(dir = %key.dir_name(), "purge starved by readers; queueing for the entry lock");
            lock.write().await
        };
        self.settle_purge_debt(&key.dir_name()).await;

        let permit = self.heavy_permit().await;
        match self.repack_blobless(&entry_dir, &permit).await {
            Ok(freed) => {
                metrics::record_eviction(EvictionTier::Blob);
                tracing::info!(dir = %key.dir_name(), freed_bytes = freed, "purged a served window");
            }
            Err(e) => tracing::warn!(error = %e, dir = %key.dir_name(), "post-serve purge failed"),
        }
    }

    /// Whether this entry has lost the non-blocking probe often enough that
    /// the repack should stop yielding to readers.
    async fn purge_debt_due(&self, dir_name: &str) -> bool {
        let mut drift = self.drift.lock().await;
        let Some(state) = drift.get_mut(dir_name) else {
            return false;
        };
        state.losses += 1;
        state.losses >= PURGE_ESCALATION_AFTER
    }

    async fn settle_purge_debt(&self, dir_name: &str) {
        if let Some(state) = self.drift.lock().await.get_mut(dir_name) {
            state.losses = 0;
        }
    }

    /// Pull the blobs a window touches, bounded by the per-repository cap.
    ///
    /// INVARIANT: this takes NO heavy permit and never triggers reclaim. The
    /// caller holds this entry's READ guard, and both of those wait on the
    /// heavy semaphore — whose permits are held by fetches waiting for write
    /// guards, one of which is the guard the caller is holding. The headroom
    /// check below is therefore the non-reclaiming kind: it refuses when the
    /// cache is already over its watermark and leaves reclaiming to the
    /// operations that can safely do it.
    ///
    /// # Errors
    ///
    /// [`GitError`] when the cache has no headroom, or the prefetch fails.
    /// Space refusals are [`GitError::TransientlyOverCap`] — retryable, and a
    /// pressure purge has already been scheduled — never the permanent `413`:
    /// on this path the measurement includes blob weight a purge reclaims, so
    /// "too large" would abort a sync over a condition that clears itself.
    pub async fn prefetch_window(
        self: &Arc<Self>,
        key: &CacheKey,
        git_dir: &Path,
        shas: &[String],
        creds: &GitCredentials,
    ) -> Result<usize, GitError> {
        if !self.has_headroom().await {
            metrics::record_rejection(metrics::RejectReason::PrefetchHeadroom);
            self.purge_under_pressure(key);
            return Err(GitError::TransientlyOverCap);
        }
        // Leftover weight from earlier pages can already sit at the cap; the
        // mid-run watcher would refuse the fetch anyway, so refuse before the
        // origin round trips — and a fetch quick enough to finish between two
        // watcher polls is caught here on the NEXT page instead of never.
        if dir_size_off_reactor(git_dir.to_path_buf()).await > self.max_repo_bytes {
            metrics::record_rejection(metrics::RejectReason::EntryOverCap);
            self.purge_under_pressure(key);
            return Err(GitError::TransientlyOverCap);
        }
        let fetched =
            super::read::blobs::prefetch(&self.runner, git_dir, shas, creds, self.max_repo_bytes)
                .await;
        if matches!(fetched, Err(GitError::TooLarge { .. })) {
            metrics::record_rejection(metrics::RejectReason::EntryOverCap);
            self.purge_under_pressure(key);
            return Err(GitError::TransientlyOverCap);
        }
        fetched
    }

    /// Schedule a purge that skips the drift throttle and escalates on its
    /// first lost probe.
    ///
    /// Under space pressure the throttle is the enemy: it is the only thing
    /// standing between a rejected request and the headroom its retry needs,
    /// and honoring it turns every retry within the interval into a
    /// guaranteed second rejection.
    fn purge_under_pressure(self: &Arc<Self>, key: &CacheKey) -> tokio::task::JoinHandle<()> {
        let store = Arc::clone(self);
        let key = key.clone();
        tokio::spawn(async move {
            {
                let mut drift = store.drift.lock().await;
                let state = drift.entry(key.dir_name()).or_insert_with(|| DriftState {
                    checked: Instant::now(),
                    losses: 0,
                });
                if let Some(due) = Instant::now().checked_sub(DRIFT_CHECK_INTERVAL) {
                    state.checked = due;
                }
                state.losses = state.losses.max(PURGE_ESCALATION_AFTER - 1);
            }
            store.purge_if_drifted(&key).await;
        })
    }

    /// Whether the cache is under its high watermark right now, counting
    /// reservations. Read-only: it reclaims nothing and takes no permit.
    async fn has_headroom(&self) -> bool {
        if !self.budget.is_bounded() {
            return true;
        }
        let candidates = self.candidates().await;
        let accounted: u64 = candidates.iter().map(|c| c.size_bytes).sum();
        let used = self.effective_used(accounted) + self.reserved_bytes.load(Ordering::Relaxed);
        // A pure paging workload admits nothing, so without this the gauges
        // freeze at the last clone-time figure while prefetched blobs grow.
        self.gauges
            .set(used, self.budget.total_bytes, candidates.len() as u64);
        !self.budget.over_high_watermark(used)
    }

    /// Repack the entry to its skeleton if it has drifted above it.
    ///
    /// The caller must hold the entry's write guard and pass its own heavy
    /// permit; unlike [`Self::purge_if_drifted`] this neither probes the lock
    /// nor throttles, because the caller is already committed to exclusive
    /// work.
    async fn repack_if_drifted(
        &self,
        entry_dir: &Path,
        permit: &tokio::sync::SemaphorePermit<'_>,
    ) -> Result<(), StoreError> {
        let git_dir = entry_dir.join("repo.git");
        if !git_dir.is_dir() {
            return Ok(());
        }
        let Some(meta) = RepoMeta::load(entry_dir) else {
            return Ok(());
        };
        let measured = dir_size_off_reactor(git_dir.clone()).await;
        if meta.full_clone
            || !needs_consolidation(measured, meta.skeleton_bytes, pack_count(&git_dir))
        {
            return Ok(());
        }
        self.repack_blobless(entry_dir, permit).await.map(|_| ())
    }

    /// Whether this entry is due a size re-measurement, marking it checked.
    async fn drift_check_due(&self, dir_name: &str) -> bool {
        let now = Instant::now();
        let mut drift = self.drift.lock().await;
        match drift.get_mut(dir_name) {
            Some(state) if now.duration_since(state.checked) < DRIFT_CHECK_INTERVAL => false,
            Some(state) => {
                state.checked = now;
                true
            }
            None => {
                drift.insert(
                    dir_name.to_owned(),
                    DriftState {
                        checked: now,
                        losses: 0,
                    },
                );
                true
            }
        }
    }

    /// Repack to the blobless skeleton, rewrite the entry's accounting, and
    /// report the bytes reclaimed.
    ///
    /// Two git behaviours make the obvious invocation free nothing at all:
    ///
    /// - `repack` repacks promisor packs SEPARATELY and never applies
    ///   `--filter` to them. In a blobless clone every pack is a promisor pack
    ///   — the clone's own and one per lazy fetch — so the filter has nothing
    ///   to act on. The markers come off first, and go back on after, because
    ///   the objects did come from origin and git must keep tolerating the
    ///   ones this purge is about to drop.
    /// - `--filter` alone writes the filtered-out objects to a second pack
    ///   beside the first. `--filter-to` is what puts them somewhere the
    ///   purge can delete.
    ///
    /// `--no-write-bitmap-index` is required, not cosmetic: the filter splits
    /// objects across packs, and bitmap writing assumes a single pack — with
    /// bitmaps enabled the repack fails and the blobs stay on disk.
    ///
    /// The caller must hold the entry's write guard AND a heavy permit —
    /// acquiring one here deadlocked when the caller (a fetch's pre-purge)
    /// already held its own: N such fetches exhaust the semaphore and each
    /// waits forever for a permit none will release.
    async fn repack_blobless(
        &self,
        entry_dir: &Path,
        _permit: &tokio::sync::SemaphorePermit<'_>,
    ) -> Result<u64, StoreError> {
        let git_dir = entry_dir.join("repo.git");
        let before = dir_size_off_reactor(git_dir.clone()).await;

        // Under the store's own tmp/, which is wiped at startup: a crash
        // mid-repack must not strand the evicted pack somewhere permanent.
        let evicted = self.data_dir.join("tmp").join(format!(
            "evicted-{}",
            self.tmp_counter.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&evicted)?;
        let filter_to = format!("--filter-to={}", evicted.display());

        remove_promisor_markers(&git_dir);
        let repacked = self
            .runner
            .run_heavy(
                Some(&git_dir),
                &[
                    "repack",
                    "-a",
                    "-d",
                    "--filter=blob:none",
                    &filter_to,
                    "--no-write-bitmap-index",
                ],
                None,
            )
            .await;
        mark_promisor_packs(&git_dir);
        let _ = std::fs::remove_dir_all(&evicted);
        repacked?;

        let purged = dir_size_off_reactor(git_dir.clone()).await;
        if let Some(mut meta) = RepoMeta::load(entry_dir) {
            meta.size_bytes = purged;
            meta.skeleton_bytes = purged;
            if let Err(e) = meta.store(entry_dir) {
                tracing::warn!(error = %e, "could not record the purged size; the planner will overstate this entry");
            }
        }
        Ok(before.saturating_sub(purged))
    }
}

impl RepoStore {
    /// Bring usage back under the low watermark when it has crossed the high
    /// one. Best-effort by design: a cache that cannot reclaim still serves
    /// warm repositories, and the per-repo cap is what refuses oversized work.
    /// Reclaim to the low watermark if the cache is over the high one, then
    /// report whether there is room to take more disk.
    ///
    /// Two views are consulted, and the stricter wins. The per-entry sum knows
    /// what the cache published; `statvfs` knows what the VOLUME holds —
    /// including a clone still staging under `tmp/` and anything else sharing
    /// the mount. Neither alone is sufficient.
    async fn admit(&self, entry_dir: &Path) -> Option<Reservation<'_>> {
        // INVARIANT: deciding and reserving must be one step. Two callers that
        // both read usage before either reserved would both be admitted
        // against the same headroom.
        let _decision = self.admission.lock().await;
        let want = self.headroom_for(entry_dir).await;

        let candidates = self.candidates().await;
        let accounted: u64 = candidates.iter().map(|c| c.size_bytes).sum();
        let used = self.effective_used(accounted) + self.reserved_bytes.load(Ordering::Relaxed);
        self.gauges
            .set(used, self.budget.total_bytes, candidates.len() as u64);
        if !self.budget.over_high_watermark(used + want) {
            return Some(self.reserve(want));
        }

        let target = self.budget.excess_over_low(used);
        let plan = super::disk::plan_reclaim(&candidates, target);
        tracing::info!(
            used_bytes = used,
            target_bytes = target,
            steps = plan.len(),
            "cache over the high watermark, reclaiming"
        );

        for step in plan {
            match step {
                // A purge that cannot run or cannot finish must not leave the
                // space unreclaimed: eviction frees it with no git involved.
                Reclaim::PurgeBlobs { dir_name, frees } => {
                    match self.purge_blobs_by_dir(&dir_name).await {
                        Ok(BlobPurge::Purged) => {
                            metrics::record_eviction(EvictionTier::Blob);
                            tracing::info!(dir = %dir_name, freed_bytes = frees, "purged blobs");
                        }
                        Ok(BlobPurge::Skipped) => {}
                        Ok(BlobPurge::PermitBusy) => {
                            tracing::info!(dir = %dir_name, "blob purge would wait for the heavy permit; evicting instead");
                            self.evict_dir(&dir_name, frees).await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, dir = %dir_name, "blob purge failed; evicting instead");
                            self.evict_dir(&dir_name, frees).await;
                        }
                    }
                }
                Reclaim::Evict { dir_name, frees } => {
                    self.evict_dir(&dir_name, frees).await;
                }
            }
        }

        // Re-read: the plan is what we intended, not what we achieved — a
        // step can fail, and an in-use entry is skipped entirely.
        let remaining = self.candidates().await;
        let accounted: u64 = remaining.iter().map(|c| c.size_bytes).sum();
        let after = self.effective_used(accounted) + self.reserved_bytes.load(Ordering::Relaxed);
        self.gauges
            .set(after, self.budget.total_bytes, remaining.len() as u64);
        if self.budget.over_high_watermark(after + want) {
            tracing::warn!(
                used_bytes = after,
                reserving_bytes = want,
                high_watermark = self.budget.high_watermark(),
                "nothing left to reclaim and still over the high watermark; refusing admission"
            );
            metrics::record_rejection(metrics::RejectReason::AdmissionExhausted);
            return None;
        }
        Some(self.reserve(want))
    }

    /// Evict one idle entry. A refused write lock means a reader or a writer
    /// holds it — the entry is skipped, never deleted under a reader.
    async fn evict_dir(&self, dir_name: &str, frees: u64) {
        let path = self.data_dir.join("repos").join(dir_name);
        let lock = self.lock_for_dir(dir_name).await;
        // INVARIANT: only a writer may delete an entry — a reader must never
        // observe a partially deleted repository.
        let Ok(_write) = lock.try_write() else {
            return;
        };
        match remove_tree_off_reactor(path).await {
            Ok(()) => {
                // A re-clone must not inherit the evicted entry's drift
                // throttle or escalation losses.
                self.drift.lock().await.remove(dir_name);
                metrics::record_eviction(EvictionTier::Full);
                tracing::info!(dir = %dir_name, freed_bytes = frees, "evicted repo");
            }
            Err(e) => tracing::warn!(error = %e, dir = %dir_name, "eviction failed"),
        }
    }

    /// The most this operation can still add to the entry: everything between
    /// its current size and the per-repository cap, since the cap is what the
    /// mid-run watcher enforces (§3.6).
    ///
    /// Zero when either figure is unbounded — the test constructor uses
    /// `u64::MAX` for both, and reserving against it would refuse everything.
    async fn headroom_for(&self, entry_dir: &Path) -> u64 {
        if !self.budget.is_bounded() || self.max_repo_bytes == u64::MAX {
            return 0;
        }
        let measured = dir_size_off_reactor(entry_dir.join("repo.git")).await;
        self.max_repo_bytes.saturating_sub(measured)
    }

    fn reserve(&self, bytes: u64) -> Reservation<'_> {
        self.reserved_bytes.fetch_add(bytes, Ordering::Relaxed);
        Reservation {
            reserved: &self.reserved_bytes,
            bytes,
        }
    }

    /// Build generation `generation`'s page index: both whole-history walks,
    /// run ONCE, so no page ever pays them again.
    ///
    /// Called with the entry's write lock held, after the refs are final and
    /// before the metadata that names the generation is published — a crash in
    /// between strands a file the next successful build deletes. Best-effort
    /// by design: a page finding no index falls back to the live walks, so a
    /// failed build costs the old performance, never correctness.
    async fn build_page_index(&self, git_dir: &Path, generation: u64, creds: &GitCredentials) {
        let built: Result<(), GitError> = async {
            let keys =
                crate::engine::read::commits::enumerate(&self.runner, git_dir, creds).await?;
            let in_default =
                crate::engine::read::commits::default_branch_commits(&self.runner, git_dir, creds)
                    .await?;

            let rows: Vec<super::index::IndexRow> = keys
                .into_iter()
                .map(|key| super::index::IndexRow {
                    in_default_branch: in_default.contains(&key.sha),
                    key,
                })
                .collect();

            let git_dir = git_dir.to_path_buf();
            tokio::task::spawn_blocking(move || super::index::write(&git_dir, generation, &rows))
                .await
                .map_err(|e| GitError::Io(std::io::Error::other(e)))?
                .map_err(GitError::Io)
        }
        .await;

        if let Err(e) = built {
            tracing::warn!(error = %e, generation, "page index build failed; pages fall back to the live walk");
        }
    }

    /// Point the mirror's `HEAD` at whatever origin now advertises.
    ///
    /// A fetch does not move `HEAD`, so a default-branch rename at origin
    /// would otherwise leave it on a branch `--prune` has just deleted, and
    /// every `/v1/commits` page would fail on `rev-list <gone>` until the
    /// entry was evicted.
    ///
    /// `git remote set-head --auto` cannot do this job here: it writes
    /// `refs/remotes/origin/HEAD`, and this mirror keeps branches under
    /// `refs/heads/*` with no remote-tracking namespace at all, so it fails
    /// with "Not a valid ref" and changes nothing.
    ///
    /// Best-effort: an origin that will not advertise a symref leaves the
    /// previous `HEAD` in place, which [`branches::default_branch`] already
    /// tolerates.
    async fn track_origin_head(&self, git_dir: &Path, creds: &GitCredentials) {
        let Ok(advertised) = self
            .runner
            .run(
                Some(git_dir),
                &["ls-remote", "--symref", "origin", "HEAD"],
                Some(creds),
            )
            .await
        else {
            return;
        };

        let listing = String::from_utf8_lossy(&advertised.stdout);
        let Some(reference) = parse_head_symref(&listing) else {
            return;
        };
        let _ = self
            .runner
            .run(Some(git_dir), &["symbolic-ref", "HEAD", &reference], None)
            .await;
    }

    /// Fingerprint of the entry's branch heads and default branch.
    ///
    /// `None` when it cannot be read, which is treated as "changed": failing
    /// open here would pin a stale generation, and a spurious bump is merely
    /// expensive where a missed one is wrong.
    async fn ref_digest(&self, git_dir: &Path) -> Option<String> {
        let refs = self
            .runner
            .run(
                Some(git_dir),
                &[
                    "for-each-ref",
                    "--format=%(refname) %(objectname)",
                    "refs/heads",
                ],
                None,
            )
            .await
            .ok()?;
        let head = self
            .runner
            .run(Some(git_dir), &["symbolic-ref", "--quiet", "HEAD"], None)
            .await
            .map(|out| out.stdout)
            .unwrap_or_default();

        let mut hasher = Sha256::new();
        hasher.update(&refs.stdout);
        hasher.update([0u8]);
        hasher.update(&head);
        Some(hex::encode(hasher.finalize()))
    }

    /// Usage as the budget sees it, folding in the volume's own view.
    fn effective_used(&self, accounted: u64) -> u64 {
        let available = super::disk::volume_available_bytes(&self.data_dir);
        self.budget.effective_used(accounted, available)
    }

    /// Every cache entry with the facts the reclaim planner needs. `in_use` is
    /// probed with a non-blocking write lock: a repo with readers, or one being
    /// cloned, refuses the lock and is skipped.
    async fn candidates(&self) -> Vec<Candidate> {
        // The directory scan and every `meta.json` read happen off the
        // reactor; only the lock probes, which never block, run on it.
        let repos = self.data_dir.join("repos");
        let scanned: Vec<(String, RepoMeta)> = tokio::task::spawn_blocking(move || {
            let Ok(entries) = std::fs::read_dir(&repos) else {
                return Vec::new();
            };
            entries
                .flatten()
                .filter_map(|entry| {
                    let dir_name = entry.file_name().to_str().map(ToOwned::to_owned)?;
                    Some((dir_name, RepoMeta::load(&entry.path())?))
                })
                .collect()
        })
        .await
        .unwrap_or_default();

        let mut candidates = Vec::new();
        for (dir_name, meta) in scanned {
            let in_use = self.lock_for_dir(&dir_name).await.try_write().is_err();
            candidates.push(Candidate {
                dir_name,
                size_bytes: meta.size_bytes,
                skeleton_bytes: meta.skeleton_bytes,
                last_accessed_at_epoch_s: meta.last_accessed_at_epoch_s,
                in_use,
                full_clone: meta.full_clone,
            });
        }
        candidates
    }

    async fn lock_for_dir(&self, dir_name: &str) -> Arc<RwLock<()>> {
        let mut entries = self.entries.lock().await;
        entries.entry(dir_name.to_owned()).or_default().clone()
    }

    pub(crate) async fn purge_blobs_by_dir(&self, dir_name: &str) -> Result<BlobPurge, StoreError> {
        let entry_dir = self.data_dir.join("repos").join(dir_name);
        if !entry_dir.join("repo.git").is_dir() {
            return Ok(BlobPurge::Skipped);
        }

        // A promoted entry has no promisor remote behind it: re-marking its
        // packs would make git tolerate blobs nothing can serve again.
        if RepoMeta::load(&entry_dir).is_none_or(|meta| meta.full_clone) {
            return Ok(BlobPurge::Skipped);
        }

        let lock = self.lock_for_dir(dir_name).await;
        // INVARIANT: repack DELETES packs — it must run with zero readers.
        let Ok(_write) = lock.try_write() else {
            return Ok(BlobPurge::Skipped);
        };

        // Never wait: the only caller holds the admission lock, and a permit
        // held by a clone would stall every admission behind that clone.
        let Ok(permit) = self.heavy.try_acquire() else {
            return Ok(BlobPurge::PermitBusy);
        };
        self.repack_blobless(&entry_dir, &permit)
            .await
            .map(|_| BlobPurge::Purged)
    }

    /// Current cache usage, as accounted per entry.
    pub async fn used_bytes(&self) -> u64 {
        self.candidates().await.iter().map(|c| c.size_bytes).sum()
    }
}

/// The on-disk length of generation `generation`'s page index, zero when the
/// build failed and left none.
fn index_len(git_dir: &Path, generation: u64) -> u64 {
    std::fs::metadata(super::index::index_path(git_dir, generation)).map_or(0, |m| m.len())
}

/// Walk a tree's size off the reactor.
///
/// `dir_size` recurses over every pack and loose object in a repository, and
/// it runs on the request path — inside admission, the drift check and the cap
/// verdicts. Left inline it blocks a worker thread, and enough concurrent
/// large repositories stall unrelated requests and the health probe with them.
async fn dir_size_off_reactor(path: PathBuf) -> u64 {
    tokio::task::spawn_blocking(move || dir_size(&path))
        .await
        .unwrap_or(0)
}

/// Delete a tree off the reactor. An eviction unlinks a whole repository.
async fn remove_tree_off_reactor(path: PathBuf) -> std::io::Result<()> {
    match tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&path)).await {
        Ok(result) => result,
        Err(joined) => Err(std::io::Error::other(joined)),
    }
}

/// Take the entry's read side, or give up and ask the caller back.
///
/// A clone or fetch holds the WRITE side for its whole heavy budget — up to
/// half an hour. Waiting on that unbounded turns the documented `429` +
/// `Retry-After` into an HTTP request that hangs until the connector's own
/// socket timeout fires, which is the failure mode the bounded wait exists to
/// avoid (§3.2). The first caller already gets its `429` from `INLINE_WAIT`;
/// this is what gives every later one the same answer.
async fn read_within(
    lock: &Arc<RwLock<()>>,
    budget: Duration,
) -> Result<OwnedRwLockReadGuard<()>, StoreError> {
    tokio::time::timeout(budget, lock.clone().read_owned())
        .await
        .map_err(|_| StoreError::Busy {
            retry_after: COLD_RETRY_AFTER,
        })
}

/// The branch `ls-remote --symref origin HEAD` reports, if any.
///
/// The line is `ref: <refname>\tHEAD`; the rest of the listing is the ordinary
/// `<sha>\tHEAD` pair.
fn parse_head_symref(listing: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let rest = line.strip_prefix("ref: ")?;
        let (reference, name) = rest.split_once('\t')?;
        (name.trim() == "HEAD" && reference.starts_with("refs/heads/"))
            .then(|| reference.to_owned())
    })
}

/// Persist metadata that describes refs already on disk.
///
/// A failure here is not cosmetic. The refs moved, so the metadata still in
/// place describes a snapshot that no longer exists: a continuation pinned to
/// the old generation would be served the NEW refs, and a caller whose
/// fingerprint matches the old metadata would be served objects fetched with
/// someone else's credentials. Removing it makes the entry unreadable and the
/// next request re-clones — the cache is rebuildable by design.
const PARKED_META: &str = "meta.json.parked";

/// Take the published metadata out of circulation before mutating the refs
/// it describes. While parked the entry reads as absent; the caller either
/// republishes a fresh document or restores this one.
fn park_meta(entry_dir: &Path) {
    let _ = std::fs::rename(entry_dir.join("meta.json"), entry_dir.join(PARKED_META));
}

/// Put the parked document back — only valid when the refs demonstrably did
/// not move (an `--atomic` fetch that failed).
fn unpark_meta(entry_dir: &Path) {
    let _ = std::fs::rename(entry_dir.join(PARKED_META), entry_dir.join("meta.json"));
}

fn discard_parked_meta(entry_dir: &Path) {
    let _ = std::fs::remove_file(entry_dir.join(PARKED_META));
}

/// How many packs the entry's object store currently holds. Cheap: one
/// directory listing, no tree walk.
fn pack_count(git_dir: &Path) -> usize {
    std::fs::read_dir(git_dir.join("objects").join("pack")).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "pack"))
            .count()
    })
}

fn publish_meta(meta: &RepoMeta, entry_dir: &Path) -> Result<(), GitError> {
    meta.store(entry_dir).map_err(|e| {
        // Whatever occupies the path goes: leaving anything behind risks a
        // later read finding the superseded document.
        let stale = entry_dir.join("meta.json");
        if std::fs::remove_file(&stale).is_err() {
            let _ = std::fs::remove_dir_all(&stale);
        }
        tracing::error!(
            error = %e,
            "could not publish repository metadata after moving refs; entry invalidated"
        );
        GitError::Io(e)
    })
}

/// Re-assert that every pack came from the promisor remote.
///
/// A blobless purge strips the markers so `repack --filter` will touch the
/// packs at all; without restoring them git stops tolerating the very objects
/// the purge just dropped.
fn mark_promisor_packs(git_dir: &Path) {
    let pack_dir = git_dir.join("objects").join("pack");
    let Ok(entries) = std::fs::read_dir(&pack_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "pack") {
            let _ = std::fs::File::create(path.with_extension("promisor"));
        }
    }
}

/// Drop the `.promisor` markers left by a partial clone. Without this git
/// still treats the packs as promisor packs and keeps deferring to the origin
/// for objects it now has locally.
fn remove_promisor_markers(git_dir: &Path) {
    let pack_dir = git_dir.join("objects").join("pack");
    let Ok(entries) = std::fs::read_dir(&pack_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .path()
            .extension()
            .is_some_and(|ext| ext == "promisor")
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The clone invocation, isolated so the option/operand boundary is testable.
/// `--` is what keeps a hostile URL from being read as a git option; the URL is
/// already an [`crate::engine::url::CloneUrl`], so this is the second of two
/// independent guards.
fn clone_argv<'a>(url: &'a str, target: &'a str) -> [&'a str; 7] {
    // `--no-tags`: the mirror refspec only ever prunes refs/heads, so a tag
    // taken at clone time is kept forever and keeps its commits reachable
    // long after their branch is gone (§4.2).
    [
        "clone",
        "--bare",
        "--filter=blob:none",
        "--no-tags",
        "--",
        url,
        target,
    ]
}

/// The entry's meta when the repo exists and the caller's credentials match
/// the ones that proved origin access.
fn usable_meta(entry_dir: &Path, fingerprint: &str) -> Option<RepoMeta> {
    if !entry_dir.join("repo.git").is_dir() {
        return None;
    }
    let meta = RepoMeta::load(entry_dir)?;
    // Both sides are locally derived sha256 digests, never a presented secret,
    // so a plain compare leaks nothing an attacker can use; the one bearer
    // comparison in the service (`api::auth`) is constant-time.
    meta.proven(fingerprint).then_some(meta)
}

/// The entry's meta when it is usable AND was fetched within the window.
fn fresh_meta(entry_dir: &Path, fingerprint: &str, max_staleness: Duration) -> Option<RepoMeta> {
    let meta = usable_meta(entry_dir, fingerprint)?;
    let age = now_epoch_s().saturating_sub(meta.last_fetched_at_epoch_s);
    // Strict: max_staleness == 0 always fetches (an `age <= window` check
    // would call a same-second clone "fresh" and never contact origin).
    (age < max_staleness.as_secs()).then_some(meta)
}

/// Best-effort last-access bump for LRU; a lost update only skews eviction
/// order, never correctness.
fn touch_access(entry_dir: &Path, mut meta: RepoMeta) {
    meta.last_accessed_at_epoch_s = now_epoch_s();
    let _ = meta.store(entry_dir);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Fixture {
        pub(crate) root: PathBuf,
        pub(crate) origin_url: String,
        pub(crate) store: Arc<RepoStore>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    pub(crate) fn sh(dir: &Path, script: &str) {
        let output = std::process::Command::new("sh")
            .arg("-ec")
            .arg(script)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output();
        let output = match output {
            Ok(o) => o,
            Err(e) => panic!("spawn sh: {e}"),
        };
        assert!(
            output.status.success(),
            "script failed: {script}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A cache with a tiny budget, so reclaim can be exercised deterministically.
    pub(crate) fn fixture_with_budget(tag: &str, budget_bytes: u64, cap_bytes: u64) -> Fixture {
        let mut f = fixture(tag);
        let store = match RepoStore::open_cache(
            &f.root.join("cache-bounded"),
            2,
            None,
            Budget {
                total_bytes: budget_bytes,
            },
            cap_bytes,
        ) {
            Ok(s) => s,
            Err(e) => panic!("bounded store init: {e}"),
        };
        f.store = Arc::new(store);
        f
    }

    pub(crate) fn fixture(tag: &str) -> Fixture {
        // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; names carry pid/thread/counter and hold no secrets
        let root = std::env::temp_dir().join(format!(
            "git-cli-proxy-store-{tag}-{}-{}",
            std::process::id(),
            now_epoch_s()
        ));
        let origin = root.join("origin");
        if let Err(e) = std::fs::create_dir_all(&origin) {
            panic!("create origin dir: {e}");
        }
        // A file:// origin with partial-clone support enabled mirrors what a
        // real server (GitHub/GitLab) offers. The commit date is explicit: the
        // walk order is (committed_date, sha), so a wall-clock date would make
        // tests that add later commits depend on the machine's clock.
        sh(
            &origin,
            "git init -q -b main . && \
             git config uploadpack.allowFilter true && \
             git config uploadpack.allowAnySHA1InWant true && \
             echo one > a.txt && git add a.txt && \
             GIT_AUTHOR_DATE='2026-08-01T10:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-01T10:00:00+0000' git commit -qm c1",
        );
        let store = match RepoStore::new(&root.join("cache"), 2) {
            Ok(s) => s,
            Err(e) => panic!("store init: {e}"),
        };
        Fixture {
            origin_url: format!("file://{}", origin.display()),
            root,
            store: Arc::new(store),
        }
    }

    /// Pin the snapshot the entry currently holds, the way a page token minted
    /// from a guard does.
    pub(crate) fn pinned(fixture: &Fixture, key: &CacheKey, generation: u64) -> Freshness {
        let incarnation = RepoMeta::load(&fixture.store.entry_dir(key))
            .map(|meta| meta.incarnation)
            .unwrap_or_default();
        Freshness::Pinned {
            generation,
            incarnation,
        }
    }

    pub(crate) fn key(fixture: &Fixture) -> CacheKey {
        CacheKey {
            tenant_id: "t".to_owned(),
            source_id: "s".to_owned(),
            clone_url: fixture_url(&fixture.origin_url),
        }
    }

    /// Fixtures clone from local repositories, which production refuses.
    pub(crate) fn fixture_url(raw: &str) -> crate::engine::url::CloneUrl {
        let Ok(url) = crate::engine::url::CloneUrl::parse(
            raw,
            crate::engine::url::CloneUrlPolicy::with_file_origins(),
        ) else {
            panic!("fixture url must parse: {raw}")
        };
        url
    }

    /// An origin that serves a clone but refuses explicit object requests —
    /// the shape of a GitLab fork-network pool. Reproduced faithfully: the
    /// skeleton's history still references a blob the origin has since
    /// orphaned and garbage-collected, so asking for it by OID is refused.
    pub(crate) fn fixture_refusing_promisor_wants(tag: &str) -> Fixture {
        let f = fixture(tag);
        sh(
            &f.root.join("origin"),
            "echo two > a.txt && git add a.txt && \
             GIT_AUTHOR_DATE='2026-08-02T10:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-02T10:00:00+0000' git commit -qm c2",
        );
        f
    }

    /// Orphan the newest commit at origin AFTER a clone, so the cached
    /// skeleton needs objects the origin will no longer hand out.
    pub(crate) fn orphan_newest_commit_at_origin(f: &Fixture) {
        sh(
            &f.root.join("origin"),
            "git reset -q --hard HEAD~1 && \
             git reflog expire --expire=now --all && \
             git gc -q --prune=now",
        );
    }

    /// The tip commit of the cached clone.
    pub(crate) async fn newest_sha(f: &Fixture, git_dir: &Path) -> String {
        let keys = match crate::engine::read::commits::enumerate(
            f.store.runner(),
            git_dir,
            &creds(),
        )
        .await
        {
            Ok(keys) => keys,
            Err(e) => panic!("enumerate: {e}"),
        };
        let Some(last) = keys.last() else {
            panic!("fixture must have commits")
        };
        last.sha.clone()
    }

    pub(crate) fn creds() -> GitCredentials {
        GitCredentials {
            username: "u".to_owned(),
            token: "unused-for-file-transport".to_owned(),
        }
    }

    pub(crate) fn refresh() -> Freshness {
        Freshness::Refresh {
            max_staleness: Duration::from_mins(5),
        }
    }

    pub(crate) fn always_fetch() -> Freshness {
        Freshness::Refresh {
            max_staleness: Duration::ZERO,
        }
    }

    /// Cold opens answer `Busy` while the clone runs in the background; retry
    /// until it lands (the real caller is the connector's 429 retry loop).
    pub(crate) async fn open_until_ready(
        fixture: &Fixture,
        key: &CacheKey,
        freshness: Freshness,
    ) -> RepoGuard {
        for _ in 0..100u32 {
            let freshness = freshness.clone();
            match fixture.store.open(key, &creds(), freshness).await {
                Ok(guard) => return guard,
                Err(StoreError::Busy { .. }) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => panic!("open failed: {e}"),
            }
        }
        panic!("repository never became ready")
    }

    fn head_of(git_dir: &Path) -> String {
        let output = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(git_dir)
            .args(["rev-parse", "refs/heads/main"])
            .output();
        let output = match output {
            Ok(o) => o,
            Err(e) => panic!("rev-parse spawn: {e}"),
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    #[tokio::test]
    async fn clones_blobless_bare_on_first_request() {
        let f = fixture("clone");
        let guard = open_until_ready(&f, &key(&f), refresh()).await;
        assert!(
            guard.git_dir().join("HEAD").is_file(),
            "bare repo must exist"
        );
        assert_eq!(guard.generation(), 1, "first snapshot is generation 1");

        let entry = f.store.entry_dir(&key(&f));
        let Some(meta) = RepoMeta::load(&entry) else {
            panic!("meta.json must be written")
        };
        assert_eq!(meta.clone_url, f.origin_url);
        assert!(meta.size_bytes > 0, "size accounting must run");
        assert!(
            meta.proven(&creds().fingerprint()),
            "the credentials that proved access are fingerprinted"
        );
    }

    /// Grow the origin by a blob big enough that pulling it dwarfs the
    /// blobless skeleton, so drift is unambiguous rather than noise.
    /// Incompressible on purpose: zeros pack down to nothing, and the entry
    /// would never look as though it had drifted.
    async fn entry_with_fetched_blobs(tag: &str) -> (Fixture, CacheKey, u64) {
        let f = fixture(tag);
        sh(
            &f.root.join("origin"),
            "dd if=/dev/urandom of=big.bin bs=1024 count=4096 status=none && \
             git add big.bin && \
             GIT_AUTHOR_DATE='2026-08-01T11:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-01T11:00:00+0000' git commit -qm big",
        );

        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let skeleton = match RepoMeta::load(&f.store.entry_dir(&k)) {
            Some(m) => m.skeleton_bytes,
            None => panic!("meta must exist after a clone"),
        };

        let head = head_of(guard.git_dir());
        if let Err(e) = crate::engine::read::blobs::prefetch(
            f.store.runner(),
            guard.git_dir(),
            &[head],
            &creds(),
            u64::MAX,
        )
        .await
        {
            panic!("prefetch: {e}");
        }
        drop(guard);

        (f, k, skeleton)
    }

    #[tokio::test]
    async fn a_cursor_does_not_survive_an_eviction_and_re_clone() {
        // Every clone starts at generation 1. An entry evicted between two
        // pages and cloned again is back at generation 1 over a repository
        // that may have moved on, so the generation alone cannot tell the
        // second page it is looking at a different walk.
        let f = fixture("incarnation");
        let k = key(&f);
        let first = open_until_ready(&f, &k, refresh()).await;
        let generation = first.generation();
        let incarnation = first.incarnation().to_owned();
        drop(first);

        let entry_dir = f.store.entry_dir(&k);
        if let Err(e) = std::fs::remove_dir_all(&entry_dir) {
            panic!("evict: {e}");
        }
        sh(
            &f.root.join("origin"),
            "echo two > b.txt && git add b.txt && git commit -qm c2",
        );
        let second = open_until_ready(&f, &k, refresh()).await;

        assert_eq!(
            second.generation(),
            generation,
            "the re-clone is back at the same generation, which is the trap"
        );
        assert_ne!(
            second.incarnation(),
            incarnation,
            "but it must be a different incarnation"
        );
        drop(second);

        match f
            .store
            .open(
                &k,
                &creds(),
                Freshness::Pinned {
                    generation,
                    incarnation,
                },
            )
            .await
        {
            Err(StoreError::SnapshotChanged { .. }) => {}
            Ok(_) => panic!("a cursor from the evicted clone must not be served"),
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn refs_that_moved_without_their_metadata_invalidate_the_entry() {
        // The refs are already published when the metadata write happens. If
        // the old metadata survived, a continuation pinned to the old
        // generation would be served the NEW refs.
        let f = fixture("meta-fail");
        let k = key(&f);
        open_until_ready(&f, &k, refresh()).await;
        let entry_dir = f.store.entry_dir(&k);

        let Some(meta) = RepoMeta::load(&entry_dir) else {
            panic!("meta must exist")
        };
        // A non-empty directory where `meta.json` belongs: the rename cannot
        // land on it, so publishing fails after the refs are already in place.
        let stale = entry_dir.join("meta.json");
        if let Err(e) = std::fs::remove_file(&stale)
            .and_then(|()| std::fs::create_dir(&stale))
            .and_then(|()| std::fs::write(stale.join("occupied"), b"x"))
        {
            panic!("stage the failure: {e}");
        }

        assert!(
            publish_meta(&meta, &entry_dir).is_err(),
            "the write must fail for this test to mean anything"
        );
        assert!(
            !stale.exists(),
            "the superseded metadata must be gone, not left for a later read"
        );
        assert!(
            RepoMeta::load(&entry_dir).is_none(),
            "an entry whose metadata could not be published must not be readable"
        );
    }

    #[tokio::test]
    async fn admission_reserves_headroom_for_work_it_has_already_allowed() {
        // High watermark is 850k. One reservation of 500k fits; two do not.
        // Without reservations both callers see an empty cache, are both
        // admitted, and together overrun the budget.
        let f = fixture_with_budget("reserve", 1_000_000, 500_000);
        let entry_dir = f.store.entry_dir(&key(&f));

        let Some(first) = f.store.admit(&entry_dir).await else {
            panic!("an empty cache must admit the first caller")
        };
        assert!(
            f.store.admit(&entry_dir).await.is_none(),
            "the second caller must be refused against the first's reservation"
        );

        drop(first);
        assert!(
            f.store.admit(&entry_dir).await.is_some(),
            "and admitted again once that reservation is released"
        );
    }

    #[tokio::test]
    async fn a_reader_behind_a_writer_is_asked_back_rather_than_left_hanging() {
        // A clone or fetch holds the write side for its whole heavy budget —
        // up to half an hour. Waiting on that unbounded turns the documented
        // 429 into a request that hangs until the connector's socket timeout
        // fires. Every read path in `open` goes through this one function.
        let lock: Arc<RwLock<()>> = Arc::new(RwLock::new(()));
        let writer = lock.clone().write_owned().await;

        match read_within(&lock, Duration::from_millis(50)).await {
            Err(StoreError::Busy { retry_after }) => {
                assert_eq!(retry_after, COLD_RETRY_AFTER, "the caller is told when");
            }
            Ok(_) => panic!("a reader must not be served while a writer holds the entry"),
            Err(e) => panic!("expected Busy, got {e}"),
        }

        drop(writer);
        assert!(
            read_within(&lock, Duration::from_millis(50)).await.is_ok(),
            "and is served as soon as the writer is gone"
        );
    }

    #[test]
    fn the_head_symref_is_read_out_of_an_ls_remote_listing() {
        let listing = "ref: refs/heads/trunk\tHEAD\n4f0a71e8\tHEAD\n";
        assert_eq!(
            parse_head_symref(listing).as_deref(),
            Some("refs/heads/trunk")
        );
        for absent in [
            "",
            "4f0a71e8\tHEAD\n",
            "ref: refs/tags/v1\tHEAD\n",
            "ref: refs/heads/x",
        ] {
            assert!(
                parse_head_symref(absent).is_none(),
                "must not invent a symref: {absent:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_default_branch_rename_at_origin_is_followed() {
        // `git remote set-head --auto` writes refs/remotes/origin/HEAD, which
        // this mirror does not have, so it fails and leaves HEAD on a branch
        // `--prune` just deleted. Every later page then died on `rev-list`.
        let f = fixture("rename");
        let k = key(&f);
        open_until_ready(&f, &k, refresh()).await;

        let origin = f.root.join("origin");
        sh(
            &origin,
            "git branch -m main trunk && git symbolic-ref HEAD refs/heads/trunk",
        );

        let guard = open_until_ready(&f, &k, always_fetch()).await;
        let head = match f
            .store
            .runner()
            .run(Some(guard.git_dir()), &["symbolic-ref", "HEAD"], None)
            .await
        {
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
            Err(e) => panic!("symbolic-ref: {e}"),
        };
        assert_eq!(head, "refs/heads/trunk", "HEAD must follow the rename");

        // And the membership walk must not fail on the way through.
        let tip = match f
            .store
            .runner()
            .run(Some(guard.git_dir()), &["rev-parse", "HEAD"], None)
            .await
        {
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
            Err(e) => panic!("rev-parse HEAD: {e}"),
        };
        let shas = vec![tip];
        match crate::engine::read::commits::default_branch_membership(
            f.store.runner(),
            guard.git_dir(),
            &shas,
            &creds(),
        )
        .await
        {
            Ok(in_default) => assert!(
                in_default.contains(&shas[0]),
                "the renamed branch is still the default branch"
            ),
            Err(e) => panic!("membership must not fail after a rename: {e}"),
        }
    }

    #[tokio::test]
    async fn a_head_pointing_at_a_deleted_branch_does_not_fail_the_page() {
        let f = fixture("dangling-head");
        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let shas = vec![head_of(guard.git_dir())];

        sh(
            guard.git_dir(),
            "git symbolic-ref HEAD refs/heads/never-existed",
        );

        match crate::engine::read::commits::default_branch_membership(
            f.store.runner(),
            guard.git_dir(),
            &shas,
            &creds(),
        )
        .await
        {
            Ok(in_default) => assert!(
                in_default.is_empty(),
                "no default branch is the honest answer, not a 500"
            ),
            Err(e) => panic!("a dangling HEAD must not fail the page: {e}"),
        }
    }

    #[tokio::test]
    async fn a_fetch_sheds_the_last_window_before_judging_the_cap() {
        // A purge that lost its `try_write` race leaves window blobs behind.
        // Judging the cap against that transient weight deletes a healthy warm
        // entry and answers 413 — which §4.4 tells the connector never to
        // retry — or kills the fetch on the watcher's first poll, forever.
        let (f, k, skeleton) = entry_with_fetched_blobs("fetch-purge").await;
        let entry_dir = f.store.entry_dir(&k);
        let inflated = dir_size(&entry_dir.join("repo.git"));
        assert!(inflated > skeleton * 2, "the entry must carry blob weight");

        // A cap that the skeleton fits under but the inflated entry does not.
        let bounded = RepoStore::open_cache(
            &f.root.join("cache"),
            2,
            None,
            Budget {
                total_bytes: u64::MAX,
            },
            (skeleton * 3).max(inflated / 2),
        );
        let store = match bounded {
            Ok(s) => Arc::new(s),
            Err(e) => panic!("bounded store: {e}"),
        };

        match store.open(&k, &creds(), always_fetch()).await {
            Ok(_) | Err(StoreError::Busy { .. }) => {}
            Err(StoreError::TooLarge { .. }) => {
                panic!("transient window blobs must not be charged against the cap")
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
        assert!(
            entry_dir.join("repo.git").is_dir(),
            "a healthy warm entry must survive its own leftover blobs"
        );
    }

    #[tokio::test]
    async fn a_noop_fetch_rebuilds_a_missing_index() {
        // The upgrade path for entries cloned before indexes existed, and the
        // self-heal for a build that failed: the next fetch notices the file
        // is gone even when the refs did not move.
        let f = fixture("index-upgrade");
        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let path = super::super::index::index_path(guard.git_dir(), guard.generation());
        assert!(path.is_file(), "the clone must have built an index");

        if let Err(e) = std::fs::remove_file(&path) {
            panic!("stage: {e}");
        }
        drop(guard);

        let guard = open_until_ready(&f, &k, always_fetch()).await;
        assert!(
            super::super::index::index_path(guard.git_dir(), guard.generation()).is_file(),
            "a fetch that moved nothing must still replace a missing index"
        );
    }

    #[tokio::test]
    async fn the_index_is_counted_by_the_entry_accounting() {
        let f = fixture("index-accounting");
        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let path = super::super::index::index_path(guard.git_dir(), guard.generation());
        let index_bytes = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) => panic!("index must exist: {e}"),
        };
        assert!(index_bytes > 0);

        let Some(meta) = RepoMeta::load(&f.store.entry_dir(&k)) else {
            panic!("meta must exist")
        };
        let measured = dir_size(guard.git_dir());
        assert!(meta.size_bytes >= measured.min(meta.size_bytes), "sanity");
        assert!(
            meta.size_bytes >= index_bytes,
            "the published size must include the index: {} vs index {index_bytes}",
            meta.size_bytes
        );
        assert!(
            meta.size_bytes + 4096 >= measured,
            "the published size must not understate the disk by more than noise: \
             published {} vs measured {measured}",
            meta.size_bytes
        );
    }

    #[tokio::test]
    async fn the_purge_probe_accepts_a_capable_git() {
        // Guards the boot check itself: a probe that cannot pass on a git that
        // demonstrably purges would refuse every deployment.
        let f = fixture("purge-probe");
        if let Err(e) = f.store.require_purge_support().await {
            panic!("this git runs the purge; the probe must accept it: {e}");
        }
    }

    #[tokio::test]
    async fn a_served_window_purges_the_blobs_it_pulled() {
        let (f, k, skeleton) = entry_with_fetched_blobs("drift-purge").await;
        let entry_dir = f.store.entry_dir(&k);
        assert!(
            dir_size(&entry_dir.join("repo.git")) > skeleton * 2,
            "the prefetch must have inflated the entry, or the test proves nothing"
        );

        f.store.purge_if_drifted(&k).await;

        let Some(after) = RepoMeta::load(&entry_dir) else {
            panic!("meta must survive a purge")
        };
        assert!(
            after.size_bytes < skeleton * 2,
            "a served window must not leave its blobs behind: {} vs skeleton {skeleton}",
            after.size_bytes
        );
        assert_eq!(
            after.size_bytes,
            dir_size(&entry_dir.join("repo.git")),
            "accounting must match the disk after a purge"
        );
    }

    #[tokio::test]
    async fn accounting_reflects_blobs_even_when_no_purge_is_warranted() {
        // A promoted entry holds the only copy of its blobs, so it is never
        // purged — but the planner still has to see its true size, or it
        // evicts warm entries believing everything is skeleton-sized.
        let (f, k, _) = entry_with_fetched_blobs("drift-accounting").await;
        let entry_dir = f.store.entry_dir(&k);
        let Some(mut meta) = RepoMeta::load(&entry_dir) else {
            panic!("meta must exist")
        };
        let understated = meta.size_bytes;
        meta.full_clone = true;
        if let Err(e) = meta.store(&entry_dir) {
            panic!("meta store: {e}");
        }

        f.store.purge_if_drifted(&k).await;

        let Some(after) = RepoMeta::load(&entry_dir) else {
            panic!("meta must exist")
        };
        assert!(
            after.size_bytes > understated,
            "the fetched blobs must be accounted: {} was already {understated}",
            after.size_bytes
        );
        assert_eq!(
            after.size_bytes,
            dir_size(&entry_dir.join("repo.git")),
            "accounting must match the disk"
        );
        assert!(after.full_clone, "a promoted entry must stay promoted");
    }

    #[tokio::test]
    async fn a_purge_never_runs_while_a_reader_holds_the_entry() {
        let (f, k, _) = entry_with_fetched_blobs("drift-reader").await;
        let entry_dir = f.store.entry_dir(&k);
        let inflated = dir_size(&entry_dir.join("repo.git"));

        let reader = match f.store.open(&k, &creds(), pinned(&f, &k, 1)).await {
            Ok(g) => g,
            Err(e) => panic!("pinned open: {e}"),
        };
        f.store.purge_if_drifted(&k).await;

        assert_eq!(
            dir_size(&entry_dir.join("repo.git")),
            inflated,
            "repack deletes packs; it must not run opportunistically under a reader"
        );

        // The measurement, though, needs no lock at all: gating it behind the
        // repack's probe starves the accounting under continuous paging, and
        // the reclaim planner then treats an inflated entry as skeleton-sized.
        let Some(meta) = RepoMeta::load(&entry_dir) else {
            panic!("meta must exist")
        };
        assert_eq!(
            meta.size_bytes, inflated,
            "accounting must be updated even while a reader holds the entry"
        );
        drop(reader);
    }

    #[tokio::test]
    async fn a_repack_starved_by_readers_eventually_queues_for_the_lock() {
        // When a reader is always in place before the purge task runs the
        // opportunistic probe never wins, and an entry served continuously
        // grows for as long as anyone keeps reading it. After enough losses
        // the repack must queue for the write side like a fetch would.
        //
        // Rewinding the throttle stands in for waiting out the interval;
        // losses must survive the rewind.
        async fn rewind_throttle(store: &RepoStore) {
            for state in store.drift.lock().await.values_mut() {
                if let Some(rewound) = Instant::now().checked_sub(DRIFT_CHECK_INTERVAL) {
                    state.checked = rewound;
                }
            }
        }

        let (f, k, skeleton) = entry_with_fetched_blobs("drift-escalate").await;
        let entry_dir = f.store.entry_dir(&k);
        let inflated = dir_size(&entry_dir.join("repo.git"));

        let reader = match f.store.open(&k, &creds(), pinned(&f, &k, 1)).await {
            Ok(g) => g,
            Err(e) => panic!("pinned open: {e}"),
        };

        // Drive the real entry point, not the counter: each round is one page
        // served under the held guard, with only the throttle stepped forward.
        for lost in 1..PURGE_ESCALATION_AFTER {
            rewind_throttle(&f.store).await;
            f.store.purge_if_drifted(&k).await;
            assert_eq!(
                dir_size(&entry_dir.join("repo.git")),
                inflated,
                "loss {lost} must stay opportunistic under a reader"
            );
        }

        rewind_throttle(&f.store).await;
        let escalated = tokio::spawn({
            let store = Arc::clone(&f.store);
            let k = k.clone();
            async move { store.purge_if_drifted(&k).await }
        });

        // The escalated repack must reach the lock while the reader still
        // holds it: releasing first would let the opportunistic probe win and
        // the test would pass with no escalation at all.
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(
            dir_size(&entry_dir.join("repo.git")),
            inflated,
            "the escalated repack must wait for the reader, not repack under it"
        );

        drop(reader);
        if let Err(e) = escalated.await {
            panic!("the escalated purge must not panic: {e}");
        }

        let Some(meta) = RepoMeta::load(&entry_dir) else {
            panic!("meta must exist")
        };
        assert!(
            meta.size_bytes < skeleton * 2,
            "the repack must have run: {} vs skeleton {skeleton}",
            meta.size_bytes
        );
        assert!(
            !f.store.purge_debt_due(&k.dir_name()).await,
            "a completed repack must reset the escalation counter"
        );
    }

    #[tokio::test]
    async fn a_fetch_of_a_drifted_entry_completes_with_one_heavy_permit() {
        // The pre-fetch purge repacks while the fetch already holds the only
        // permit; a repack that acquired its own would deadlock right here —
        // no git subprocess running, no timeout ever starting.
        let (f, k, _) = entry_with_fetched_blobs("drift-one-permit").await;
        let one_permit = match RepoStore::new(&f.root.join("cache"), 1) {
            Ok(s) => Arc::new(s),
            Err(e) => panic!("store init: {e}"),
        };

        let fetched = tokio::time::timeout(
            Duration::from_mins(2),
            one_permit.open(&k, &creds(), always_fetch()),
        )
        .await;
        let Ok(outcome) = fetched else {
            panic!("the fetch deadlocked against its own pre-purge")
        };
        if let Err(e) = outcome {
            panic!("the fetch must succeed: {e}");
        }
    }

    #[tokio::test]
    async fn a_failed_fetch_keeps_the_entry_serveable() {
        // A transient origin outage must cost one failed sync, not the warm
        // cache: the metadata parked before the --atomic fetch is restored,
        // because a failed atomic fetch moved nothing.
        let f = fixture("fetch-outage");
        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let generation = guard.generation();
        drop(guard);

        let origin = f.root.join("origin");
        let parked_origin = f.root.join("origin-gone");
        if let Err(e) = std::fs::rename(&origin, &parked_origin) {
            panic!("park origin: {e}");
        }
        let refused = f.store.open(&k, &creds(), always_fetch()).await;
        assert!(refused.is_err(), "a fetch without an origin must fail");

        if let Err(e) = std::fs::rename(&parked_origin, &origin) {
            panic!("restore origin: {e}");
        }
        let pinned_read = f.store.open(&k, &creds(), pinned(&f, &k, generation)).await;
        match pinned_read {
            Ok(g) => assert_eq!(g.generation(), generation, "the snapshot must survive"),
            Err(e) => panic!("the entry must stay serveable after a failed fetch: {e}"),
        }
    }

    #[tokio::test]
    async fn boot_removes_an_entry_without_metadata() {
        // A crash between moving refs and publishing metadata leaves a dir no
        // request can use and no reclaim plan can see: without the boot sweep
        // its bytes are leaked until the same repository is requested again.
        let f = fixture("boot-sweep");
        let orphan = f.root.join("cache").join("repos").join("a".repeat(64));
        if let Err(e) = std::fs::create_dir_all(orphan.join("repo.git")) {
            panic!("stage orphan: {e}");
        }

        let rebooted = match RepoStore::new(&f.root.join("cache"), 2) {
            Ok(s) => s,
            Err(e) => panic!("reboot: {e}"),
        };
        drop(rebooted);
        assert!(
            !orphan.exists(),
            "boot must remove an entry whose metadata cannot be read"
        );
    }

    #[tokio::test]
    async fn a_second_prefetch_of_the_same_window_fetches_nothing() {
        // Consecutive pages share blobs; re-requesting local ones pays origin
        // round trips and lands duplicate copies in new packs.
        let (f, k, _) = entry_with_fetched_blobs("prefetch-dedup").await;
        let git_dir = f.store.entry_dir(&k).join("repo.git");
        let head = head_of(&git_dir);

        let refetched = crate::engine::read::blobs::prefetch(
            f.store.runner(),
            &git_dir,
            &[head],
            &creds(),
            u64::MAX,
        )
        .await;
        match refetched {
            Ok(count) => assert_eq!(count, 0, "every blob of this window is already local"),
            Err(e) => panic!("presence filtering must not fail the prefetch: {e}"),
        }
    }

    #[tokio::test]
    async fn a_pressure_purge_needs_no_throttle_window_and_no_repeated_losses() {
        // The rejected request's retry needs headroom NOW: the pressure purge
        // must skip the per-minute throttle (a check just ran, from the page
        // that grew the entry) and must not spend three more losing probes
        // before it queues behind the reader.
        let (f, k, skeleton) = entry_with_fetched_blobs("pressure-purge").await;
        let entry_dir = f.store.entry_dir(&k);

        let reader = match f.store.open(&k, &creds(), pinned(&f, &k, 1)).await {
            Ok(g) => g,
            Err(e) => panic!("pinned open: {e}"),
        };
        f.store.purge_if_drifted(&k).await;
        f.store.purge_if_drifted(&k).await;
        assert!(
            dir_size(&entry_dir.join("repo.git")) > skeleton * 2,
            "under a reader and inside the throttle window the plain purge must not reclaim"
        );

        let purge = f.store.purge_under_pressure(&k);
        drop(reader);
        if let Err(e) = purge.await {
            panic!("the pressure purge must not panic: {e}");
        }

        let Some(after) = RepoMeta::load(&entry_dir) else {
            panic!("meta must survive")
        };
        assert!(
            after.size_bytes < skeleton * 2,
            "one pressure purge must reclaim despite throttle and losses: {} vs skeleton {skeleton}",
            after.size_bytes
        );
    }

    #[tokio::test]
    async fn a_prefetch_over_the_entry_cap_is_transient_not_permanent() {
        // The prefetch measurement includes blob weight a purge reclaims, so
        // "too large" here must be the retryable rejection, never the 413
        // that tells the connector to abandon the repository for good.
        let (f, k, skeleton) = entry_with_fetched_blobs("prefetch-cap").await;
        let git_dir = f.store.entry_dir(&k).join("repo.git");

        let capped = match RepoStore::open_cache(
            &f.root.join("cache"),
            2,
            None,
            Budget {
                total_bytes: u64::MAX,
            },
            skeleton + 1024,
        ) {
            Ok(s) => Arc::new(s),
            Err(e) => panic!("capped store: {e}"),
        };
        let head = head_of(&git_dir);
        let refused = capped
            .prefetch_window(&k, &git_dir, &[head], &creds())
            .await;
        assert!(
            matches!(refused, Err(GitError::TransientlyOverCap)),
            "expected the retryable rejection, got {refused:?}"
        );
    }

    #[tokio::test]
    async fn a_purged_entry_still_serves_and_refetches() {
        // The dangerous half of a purge: an entry that frees bytes but can no
        // longer produce the objects it dropped is worse than one that frees
        // nothing.
        let (f, k, _) = entry_with_fetched_blobs("drift-refetch").await;
        f.store.purge_if_drifted(&k).await;

        let guard = open_until_ready(&f, &k, refresh()).await;
        let head = head_of(guard.git_dir());
        match crate::engine::read::blobs::prefetch(
            f.store.runner(),
            guard.git_dir(),
            &[head],
            &creds(),
            u64::MAX,
        )
        .await
        {
            Ok(_) => {}
            Err(e) => panic!("a purged entry must re-fetch what it dropped: {e}"),
        }
    }

    #[tokio::test]
    async fn the_drift_check_is_throttled_per_entry() {
        let f = fixture("drift-throttle");
        assert!(
            f.store.drift_check_due("repo-a").await,
            "the first serve of an entry measures it"
        );
        assert!(
            !f.store.drift_check_due("repo-a").await,
            "a paginating caller must not pay a dir_size per page"
        );
        assert!(
            f.store.drift_check_due("repo-b").await,
            "the throttle is per entry, not global"
        );
    }

    #[tokio::test]
    async fn fresh_repo_skips_the_fetch() {
        let f = fixture("fresh");
        let k = key(&f);
        open_until_ready(&f, &k, refresh()).await;
        let entry_dir = f.store.entry_dir(&k);
        let before = match RepoMeta::load(&entry_dir) {
            Some(m) => m.generation,
            None => panic!("meta must exist"),
        };

        sh(
            &f.root.join("origin"),
            "echo two > b.txt && git add b.txt && git commit -qm c2",
        );
        let guard = open_until_ready(&f, &k, refresh()).await;

        assert_eq!(guard.generation(), before, "no fetch within the window");
        assert_ne!(
            head_of(guard.git_dir()),
            head_of(&f.root.join("origin").join(".git")),
            "cache serves the snapshot it already had"
        );
    }

    #[tokio::test]
    async fn stale_repo_fetches_new_commits() {
        let f = fixture("stale");
        let k = key(&f);
        open_until_ready(&f, &k, refresh()).await;

        let origin = f.root.join("origin");
        sh(
            &origin,
            "echo two > b.txt && git add b.txt && git commit -qm c2",
        );

        let guard = open_until_ready(&f, &k, always_fetch()).await;
        assert_eq!(
            head_of(guard.git_dir()),
            head_of(&origin.join(".git")),
            "fetch must advance to the origin head"
        );
        assert_eq!(guard.generation(), 2, "a fetch bumps the generation");
    }

    #[tokio::test]
    async fn pinned_generation_never_fetches_and_detects_a_new_snapshot() {
        let f = fixture("pinned");
        let k = key(&f);
        let first = open_until_ready(&f, &k, refresh()).await;
        let generation = first.generation();
        drop(first);

        sh(
            &f.root.join("origin"),
            "echo two > b.txt && git add b.txt && git commit -qm c2",
        );

        let continuation = match f.store.open(&k, &creds(), pinned(&f, &k, generation)).await {
            Ok(g) => g,
            Err(e) => panic!("pinned open: {e}"),
        };
        assert_ne!(
            head_of(continuation.git_dir()),
            head_of(&f.root.join("origin").join(".git")),
            "a continuation page must not contact origin"
        );
        drop(continuation);

        open_until_ready(&f, &k, always_fetch()).await;
        let stale_page = f.store.open(&k, &creds(), pinned(&f, &k, generation)).await;
        match stale_page {
            Err(StoreError::SnapshotChanged { current }) => {
                assert_eq!(current, generation + 1, "reports the live generation");
            }
            Ok(_) => panic!("a superseded snapshot must not be served"),
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn warm_cache_is_never_served_on_unproven_credentials() {
        let f = fixture("creds");
        let k = key(&f);
        let warm = open_until_ready(&f, &k, refresh()).await;
        let warm_generation = warm.generation();
        drop(warm);

        let intruder = GitCredentials {
            username: "u".to_owned(),
            token: "someone-elses-token".to_owned(),
        };
        // Unknown credentials must never short-circuit onto the warm entry:
        // the caller is forced to prove origin access first. This file://
        // origin accepts anyone, so the proof succeeds — against a real vendor
        // the same path is where the caller gets rejected.
        //
        // The observable is the fingerprint, NOT the generation. Only `clone`
        // and `fetch` write cred_fingerprint, and both only after git has
        // actually run against origin, so the intruder's fingerprint landing
        // on the entry IS the proof that origin was contacted. The generation
        // deliberately does not move here: origin had nothing new, and bumping
        // it would 409 every page token already in flight.
        let outcome = f.store.open(&k, &intruder, refresh()).await;
        match outcome {
            Err(StoreError::Busy { .. }) | Ok(_) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }

        let Some(meta) = RepoMeta::load(&f.store.entry_dir(&k)) else {
            panic!("meta must exist")
        };
        assert_eq!(
            meta.generation, warm_generation,
            "an unchanged origin must not invalidate live page tokens"
        );
        assert!(
            meta.proven(&intruder.fingerprint()),
            "whoever last proved access is recorded"
        );
    }

    #[tokio::test]
    async fn refresh_flights_are_keyed_per_credential() {
        let f = fixture("flightkey");
        let k = key(&f);
        let intruder = GitCredentials {
            username: "u".to_owned(),
            token: "someone-elses-token".to_owned(),
        };

        // Hold the entry's write lock so both refreshes stay in flight while
        // the map is inspected.
        let lock = f.store.entry_lock(&k).await;
        let held = lock.write_owned().await;

        let mine = f
            .store
            .refresh_task(&k, &creds(), Duration::ZERO, RefreshKind::Sync)
            .await;
        let mine_again = f
            .store
            .refresh_task(&k, &creds(), Duration::ZERO, RefreshKind::Sync)
            .await;
        assert_eq!(
            f.store.inflight.lock().await.len(),
            1,
            "the same credentials must collapse onto one flight"
        );

        let theirs = f
            .store
            .refresh_task(&k, &intruder, Duration::ZERO, RefreshKind::Sync)
            .await;
        assert_eq!(
            f.store.inflight.lock().await.len(),
            2,
            "different credentials must not adopt another caller's proof"
        );

        drop((mine, mine_again, theirs));
        drop(held);
    }

    #[tokio::test]
    async fn a_joined_refresh_never_serves_a_foreign_proof() {
        let f = fixture("reproof");
        let k = key(&f);
        let intruder = GitCredentials {
            username: "u".to_owned(),
            token: "someone-elses-token".to_owned(),
        };

        // Alternating callers force the entry's fingerprint to flip on every
        // open, which is exactly when a joined flight could hand back someone
        // else's proof. A guard must never outlive its own proof, so each is
        // dropped before the next caller runs.
        let mut served = 0;
        for who in [creds(), intruder.clone(), creds(), intruder] {
            match f.store.open(&k, &who, refresh()).await {
                Ok(guard) => {
                    served += 1;
                    let Some(meta) = RepoMeta::load(&f.store.entry_dir(&k)) else {
                        panic!("meta must exist alongside a served guard")
                    };
                    assert!(
                        meta.proven(&who.fingerprint()),
                        "served a snapshot never proved by these credentials"
                    );
                    assert_eq!(
                        guard.generation(),
                        meta.generation,
                        "the guard must carry the generation the reader will see"
                    );
                }
                Err(StoreError::Busy { .. }) => {}
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert_eq!(served, 4, "every sequential caller must be served");
    }

    #[tokio::test]
    async fn a_promisor_refusal_promotes_the_entry_once() {
        let f = fixture_refusing_promisor_wants("promote");
        let k = key(&f);

        let guard = open_until_ready(&f, &k, refresh()).await;
        let git_dir = guard.git_dir().to_path_buf();
        let before = guard.generation();
        drop(guard);

        orphan_newest_commit_at_origin(&f);

        // The blob the orphaned commit introduced is still referenced by the
        // cached history but is gone at origin: an explicit want is refused.
        let newest = newest_sha(&f, &git_dir).await;
        let refusal = crate::engine::read::blobs::prefetch(
            f.store.runner(),
            &git_dir,
            &[newest],
            &creds(),
            u64::MAX,
        )
        .await;
        assert!(
            matches!(refusal, Err(GitError::PromisorRefused)),
            "the fixture must reproduce a promisor refusal, got {refusal:?}"
        );

        let promoted = f.store.promote_to_full_clone(&k, &creds()).await;
        let generation = match promoted {
            Ok(generation) => generation,
            Err(e) => panic!("promotion failed: {e}"),
        };
        assert!(generation > before, "promotion must bump the generation");

        let Some(meta) = RepoMeta::load(&f.store.entry_dir(&k)) else {
            panic!("meta must exist")
        };
        assert!(meta.full_clone, "the entry must be recorded as promoted");
        assert_eq!(
            meta.skeleton_bytes, meta.size_bytes,
            "a promoted entry has no reclaimable blob weight"
        );

        let markers: Vec<_> = std::fs::read_dir(git_dir.join("objects").join("pack"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".promisor"))
            .collect();
        assert!(
            markers.is_empty(),
            "promisor markers left behind: {markers:?}"
        );
    }

    #[tokio::test]
    async fn an_already_promoted_entry_is_not_promoted_again() {
        let f = fixture_refusing_promisor_wants("promote-idempotent");
        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        drop(guard);

        let Ok(first) = f.store.promote_to_full_clone(&k, &creds()).await else {
            panic!("first promotion must succeed")
        };
        let Ok(second) = f.store.promote_to_full_clone(&k, &creds()).await else {
            panic!("second promotion must be a no-op, not a failure")
        };
        assert_eq!(first, second, "a promoted entry must not be rebuilt again");
    }

    #[tokio::test]
    async fn concurrent_requests_single_flight_the_clone() {
        let f = fixture("concurrent");
        let k = key(&f);

        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = f.store.clone();
            let k = k.clone();
            handles.push(tokio::spawn(async move {
                store
                    .open(&k, &creds(), refresh())
                    .await
                    .map(|g| (g.git_dir().to_path_buf(), g.generation()))
            }));
        }

        let mut ready = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(result)) => ready.push(result),
                // Losing the inline wait is a valid outcome, not a failure.
                Ok(Err(StoreError::Busy { .. })) => {}
                Ok(Err(e)) => panic!("open failed: {e}"),
                Err(e) => panic!("task panicked: {e}"),
            }
        }

        let guard = open_until_ready(&f, &k, refresh()).await;
        assert_eq!(guard.generation(), 1, "one clone, one generation");
        for (dir, generation) in ready {
            assert_eq!(dir, guard.git_dir(), "all callers see one repo");
            assert_eq!(generation, 1, "no caller observed a second clone");
        }
    }

    #[tokio::test]
    async fn unknown_origin_reports_a_typed_error() {
        let f = fixture("missing");
        let k = CacheKey {
            clone_url: fixture_url(&format!("file://{}", f.root.join("no-such-repo").display())),
            ..key(&f)
        };
        for _ in 0..100u32 {
            match f.store.open(&k, &creds(), refresh()).await {
                Err(StoreError::Busy { .. }) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(StoreError::NotFound | StoreError::Git(_)) => return,
                Err(e) => panic!("unexpected error kind: {e}"),
                Ok(_) => panic!("clone of a missing origin must fail"),
            }
        }
        panic!("clone failure never surfaced");
    }

    #[tokio::test]
    async fn a_repository_over_the_cap_is_refused_permanently() {
        // Cap of one byte: any real clone exceeds it.
        let f = fixture_with_budget("cap", 10_000_000, 1);
        let k = key(&f);

        for _ in 0..40u32 {
            match f
                .store
                .open(
                    &k,
                    &creds(),
                    Freshness::Refresh {
                        max_staleness: Duration::from_mins(5),
                    },
                )
                .await
            {
                Err(StoreError::Busy { .. }) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(StoreError::TooLarge { cap_bytes }) => {
                    assert_eq!(cap_bytes, 1, "the cap is reported to the caller");
                    let entry = f.store.entry_dir(&k);
                    assert!(
                        !entry.join("repo.git").is_dir(),
                        "an oversized clone must not be left on disk"
                    );
                    return;
                }
                Ok(_) => panic!("an oversized repository must not be served"),
                Err(e) => panic!("expected a size refusal, got {e}"),
            }
        }
        panic!("the cap was never enforced");
    }

    #[tokio::test]
    async fn crossing_the_high_watermark_reclaims_an_idle_repository() {
        // Budget of 1 byte: any entry puts the cache over the high watermark,
        // so the next admission must reclaim.
        let f = fixture_with_budget("reclaim", 1, u64::MAX);
        let k = key(&f);

        let guard = open_until_ready(
            &f,
            &k,
            Freshness::Refresh {
                max_staleness: Duration::from_mins(5),
            },
        )
        .await;
        assert!(f.store.used_bytes().await > 0, "the clone is accounted for");
        drop(guard);

        // A second repository forces admission to run with nothing pinned.
        let origin_two = f.root.join("origin2");
        if let Err(e) = std::fs::create_dir_all(&origin_two) {
            panic!("create second origin: {e}");
        }
        sh(
            &origin_two,
            "git init -q -b main . && git config uploadpack.allowFilter true && \
             echo x > x.txt && git add . && \
             GIT_AUTHOR_DATE='2026-08-01T10:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-01T10:00:00+0000' git commit -qm x",
        );
        let second = CacheKey {
            clone_url: fixture_url(&format!("file://{}", origin_two.display())),
            ..key(&f)
        };
        open_until_ready(
            &f,
            &second,
            Freshness::Refresh {
                max_staleness: Duration::from_mins(5),
            },
        )
        .await;

        let first_entry = f.store.entry_dir(&k);
        assert!(
            !first_entry.join("repo.git").is_dir(),
            "the idle repository must have been reclaimed to make room"
        );
    }

    #[tokio::test]
    async fn a_pinned_repository_survives_reclaim() {
        let f = fixture_with_budget("pinned-reclaim", 1, u64::MAX);
        let k = key(&f);

        // INVARIANT: holding the guard pins the entry; reclaim must skip it.
        let guard = open_until_ready(
            &f,
            &k,
            Freshness::Refresh {
                max_staleness: Duration::from_mins(5),
            },
        )
        .await;

        let _ = f.store.admit(&f.store.entry_dir(&key(&f))).await;
        assert!(
            guard.git_dir().is_dir(),
            "a repository with a live reader must never be deleted"
        );
    }

    #[tokio::test]
    async fn a_reclaim_purge_never_waits_for_the_heavy_permit() {
        let f = fixture("permit-busy-purge");
        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        drop(guard);

        // Clones or fetches elsewhere hold every heavy permit.
        let Ok(_held) = f.store.heavy.try_acquire_many(2) else {
            panic!("the fixture's heavy permits must be free")
        };

        match f.store.purge_blobs_by_dir(&k.dir_name()).await {
            Ok(BlobPurge::PermitBusy) => {}
            other => panic!("a busy permit must be reported, not waited on: {other:?}"),
        }
    }

    #[tokio::test]
    async fn purge_blobs_succeeds_with_bitmaps_configured() {
        let f = fixture("purge");
        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let git_dir = guard.git_dir().to_path_buf();
        drop(guard);

        // Bitmap writing on + `--filter` is exactly the combination that fails
        // without `--no-write-bitmap-index`.
        sh(&git_dir, "git config repack.writeBitmaps true");
        if let Err(e) = f.store.purge_blobs_by_dir(&k.dir_name()).await {
            panic!("purge must survive repack.writeBitmaps=true: {e}");
        }
    }
}
