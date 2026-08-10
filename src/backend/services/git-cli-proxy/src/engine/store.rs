use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock, Semaphore, watch};

use super::disk::{Budget, Candidate, Reclaim, dir_size, worth_purging};
use super::key::CacheKey;
use super::meta::{RepoMeta, now_epoch_s};
use super::metrics::{self, DiskGauges, EvictionTier, FetchResult};
use super::runner::{GitCredentials, GitError, GitRunner, Timeouts};

const INLINE_WAIT: Duration = Duration::from_secs(15);
const COLD_RETRY_AFTER: Duration = Duration::from_secs(30);
const REPROOF_ATTEMPTS: usize = 2;
const BARE_REFSPEC: &str = "+refs/heads/*:refs/heads/*";
/// How often one entry's on-disk size is re-measured after being served.
const DRIFT_CHECK_INTERVAL: Duration = Duration::from_mins(1);

/// Why a refresh failed, in a form that survives being broadcast to every
/// waiter (`GitError` is not `Clone`).
#[derive(Debug, Clone)]
pub enum RefreshFailure {
    Auth,
    NotFound,
    PromisorRefused,
    AdmissionRejected,
    Timeout,
    TooLarge { cap_bytes: u64 },
    Other(String),
}

impl From<&GitError> for RefreshFailure {
    fn from(error: &GitError) -> Self {
        match error {
            GitError::AuthRejected => Self::Auth,
            GitError::NotFound => Self::NotFound,
            GitError::PromisorRefused => Self::PromisorRefused,
            GitError::AdmissionRejected => Self::AdmissionRejected,
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
    #[error("origin refuses to serve explicitly requested objects")]
    PromisorRefused,
    #[error("repository is being prepared; retry in {}s", retry_after.as_secs())]
    Busy { retry_after: Duration },
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
            RefreshFailure::PromisorRefused => Self::PromisorRefused,
            // §3.6: nothing could be freed, so the caller is asked to come
            // back rather than being served a half-prepared cache.
            RefreshFailure::AdmissionRejected => Self::Busy {
                retry_after: COLD_RETRY_AFTER,
            },
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

/// How a caller wants the snapshot resolved.
///
/// INVARIANT: `Pinned` never contacts origin — a paginating caller stays on
/// the snapshot its first page observed, so pages cannot straddle a fetch.
#[derive(Debug, Clone, Copy)]
pub enum Freshness {
    Refresh { max_staleness: Duration },
    Pinned { generation: u64 },
}

/// Read access to one cached repository. Holding the guard pins the entry:
/// fetch/repack/eviction take the write side and wait for readers to drain.
pub struct RepoGuard {
    git_dir: PathBuf,
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
}

type RefreshResult = Result<u64, RefreshFailure>;

/// Whether the cache may take more disk right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    Ok,
    Rejected,
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
    /// When each entry last had its on-disk size re-measured. Without it a
    /// 200-page walk pays a full `dir_size` per page.
    drift_checks: Mutex<HashMap<String, Instant>>,
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
        if tmp.is_dir() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        std::fs::create_dir_all(&tmp)?;
        Ok(Self {
            data_dir: data_dir.to_owned(),
            runner: GitRunner::new(Timeouts::default()).with_ca_cert(ca_cert_path),
            budget,
            max_repo_bytes,
            heavy: Semaphore::new(heavy_ops_concurrency),
            entries: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            tmp_counter: AtomicU64::new(0),
            gauges: Arc::new(DiskGauges::default()),
            drift_checks: Mutex::new(HashMap::new()),
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

    fn entry_dir(&self, key: &CacheKey) -> PathBuf {
        self.data_dir.join("repos").join(key.dir_name())
    }

    async fn entry_lock(&self, key: &CacheKey) -> Arc<RwLock<()>> {
        let mut entries = self.entries.lock().await;
        entries.entry(key.dir_name()).or_default().clone()
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
            Freshness::Pinned { generation } => {
                let read = lock.read_owned().await;
                let meta = usable_meta(&entry_dir, &fingerprint)
                    .ok_or(StoreError::SnapshotChanged { current: 0 })?;
                if meta.generation != generation {
                    return Err(StoreError::SnapshotChanged {
                        current: meta.generation,
                    });
                }
                touch_access(&entry_dir, meta);
                Ok(RepoGuard {
                    git_dir,
                    generation,
                    _read: read,
                })
            }
            Freshness::Refresh { max_staleness } => {
                {
                    let read = lock.clone().read_owned().await;
                    if let Some(meta) = fresh_meta(&entry_dir, &fingerprint, max_staleness) {
                        let generation = meta.generation;
                        touch_access(&entry_dir, meta);
                        return Ok(RepoGuard {
                            git_dir,
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

                    let read = lock.clone().read_owned().await;
                    if let Some(meta) = usable_meta(&entry_dir, &fingerprint) {
                        return Ok(RepoGuard {
                            git_dir,
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
            let outcome = match kind {
                RefreshKind::Sync => store.refresh(&key, &creds, max_staleness).await,
                RefreshKind::Promote => store.promote(&key, &creds).await,
            };
            let published = match &outcome {
                Ok(generation) => Ok(*generation),
                Err(error) => Err(RefreshFailure::from(error)),
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
        if self.admit().await == Admission::Rejected {
            return Err(GitError::AdmissionRejected);
        }

        // INVARIANT: the permit spans the whole clone — the semaphore IS the
        // global heavy-ops cap.
        let _permit = self.heavy.acquire().await;

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

        let cloned_bytes = dir_size(&tmp);
        if cloned_bytes > self.max_repo_bytes {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(GitError::TooLarge {
                cap_bytes: self.max_repo_bytes,
            });
        }

        std::fs::create_dir_all(entry_dir).map_err(GitError::Io)?;
        std::fs::rename(&tmp, git_dir).map_err(GitError::Io)?;

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
            cred_fingerprint: creds.fingerprint(),
            full_clone: false,
        };
        meta.store(entry_dir).map_err(GitError::Io)?;
        Ok(meta.generation)
    }

    async fn fetch(
        &self,
        key: &CacheKey,
        entry_dir: &Path,
        git_dir: &Path,
        creds: &GitCredentials,
    ) -> Result<u64, GitError> {
        if self.admit().await == Admission::Rejected {
            return Err(GitError::AdmissionRejected);
        }

        // INVARIANT: the permit spans the whole fetch — the semaphore IS the
        // global heavy-ops cap.
        let _permit = self.heavy.acquire().await;

        let before = self.ref_digest(git_dir).await;

        self.runner
            .run_capped(
                Some(git_dir),
                &["fetch", "--prune", "--atomic", "origin", BARE_REFSPEC],
                Some(creds),
                git_dir,
                self.max_repo_bytes,
            )
            .await
            .inspect_err(|_| metrics::record_fetch(FetchResult::Error))?;

        // A fetch does not update the mirrored HEAD, so a default-branch
        // rename at origin would stay invisible until the entry is evicted —
        // and `is_in_default_branch` would then be wrong for every row.
        // Best-effort: a vendor that refuses it must not fail the sync.
        let _ = self
            .runner
            .run(
                Some(git_dir),
                &["remote", "set-head", "origin", "--auto"],
                Some(creds),
            )
            .await;

        // INVARIANT: the generation identifies a REF SNAPSHOT, not a fetch
        // attempt. Bumping it when nothing moved would 409 every page token
        // already in flight — and a sync outliving the staleness window
        // refreshes routinely, so that is the common case, not a rare one.
        let after = self.ref_digest(git_dir).await;
        let unchanged = before.is_some() && before == after;

        let fetched_bytes = dir_size(git_dir);
        if fetched_bytes > self.max_repo_bytes {
            let _ = std::fs::remove_dir_all(entry_dir);
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
        let previous = RepoMeta::load(entry_dir);
        let generation = match (&previous, unchanged) {
            (Some(meta), true) => meta.generation,
            (previous, _) => previous.as_ref().map_or(0, |m| m.generation) + 1,
        };
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
            cred_fingerprint: creds.fingerprint(),
            // A plain fetch never changes the entry's clone shape.
            full_clone: previous.as_ref().is_some_and(|m| m.full_clone),
        };
        meta.store(entry_dir).map_err(GitError::Io)?;
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

        if let Some(meta) = RepoMeta::load(&entry_dir)
            && meta.full_clone
        {
            return Ok(meta.generation);
        }
        if !git_dir.is_dir() {
            return Err(GitError::NotFound);
        }

        // A full clone is much larger than the skeleton it replaces.
        if self.admit().await == Admission::Rejected {
            return Err(GitError::AdmissionRejected);
        }

        // INVARIANT: the permit spans the whole promotion — the semaphore IS
        // the global heavy-ops cap.
        let _permit = self.heavy.acquire().await;

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

        let promoted_bytes = dir_size(&git_dir);
        if promoted_bytes > self.max_repo_bytes {
            let _ = std::fs::remove_dir_all(&entry_dir);
            return Err(GitError::TooLarge {
                cap_bytes: self.max_repo_bytes,
            });
        }

        let now = now_epoch_s();
        let generation = RepoMeta::load(&entry_dir).map_or(0, |m| m.generation) + 1;
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
            cred_fingerprint: creds.fingerprint(),
            full_clone: true,
        };
        meta.store(&entry_dir).map_err(GitError::Io)?;
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
        // INVARIANT: a repack may follow, and repack DELETES packs. Both the
        // measurement and the repack take the write side, and neither ever
        // waits for it: a reader that still holds the entry gets served, not
        // repacked under.
        let Ok(_write) = lock.try_write() else {
            return;
        };

        let Some(mut meta) = RepoMeta::load(&entry_dir) else {
            return;
        };
        let measured = dir_size(&git_dir);
        if meta.size_bytes != measured {
            meta.size_bytes = measured;
            let _ = meta.store(&entry_dir);
        }

        // A promoted entry holds the only copy of its blobs: origin refuses to
        // serve them again, so purging would strand it.
        if meta.full_clone || !worth_purging(measured, meta.skeleton_bytes) {
            return;
        }

        match self.repack_blobless(&entry_dir).await {
            Ok(freed) => {
                metrics::record_eviction(EvictionTier::Blob, freed);
                tracing::debug!(dir = %key.dir_name(), freed_bytes = freed, "purged a served window");
            }
            Err(e) => tracing::warn!(error = %e, dir = %key.dir_name(), "post-serve purge failed"),
        }
    }

    /// Whether this entry is due a size re-measurement, marking it checked.
    async fn drift_check_due(&self, dir_name: &str) -> bool {
        let now = Instant::now();
        let mut checks = self.drift_checks.lock().await;
        match checks.get(dir_name) {
            Some(last) if now.duration_since(*last) < DRIFT_CHECK_INTERVAL => false,
            _ => {
                checks.insert(dir_name.to_owned(), now);
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
    /// The caller must hold the entry's write guard.
    async fn repack_blobless(&self, entry_dir: &Path) -> Result<u64, StoreError> {
        let git_dir = entry_dir.join("repo.git");
        let before = dir_size(&git_dir);

        // Under the store's own tmp/, which is wiped at startup: a crash
        // mid-repack must not strand the evicted pack somewhere permanent.
        let evicted = self.data_dir.join("tmp").join(format!(
            "evicted-{}",
            self.tmp_counter.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&evicted)?;
        let filter_to = format!("--filter-to={}", evicted.display());

        let _permit = self.heavy.acquire().await;

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

        let purged = dir_size(&git_dir);
        if let Some(mut meta) = RepoMeta::load(entry_dir) {
            meta.size_bytes = purged;
            meta.skeleton_bytes = purged;
            let _ = meta.store(entry_dir);
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
    async fn admit(&self) -> Admission {
        let candidates = self.candidates().await;
        let accounted: u64 = candidates.iter().map(|c| c.size_bytes).sum();
        let used = self.effective_used(accounted);
        self.gauges
            .set(used, self.budget.total_bytes, candidates.len() as u64);
        if !self.budget.over_high_watermark(used) {
            return Admission::Ok;
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
                Reclaim::PurgeBlobs { dir_name, frees } => {
                    match self.purge_blobs_by_dir(&dir_name).await {
                        Ok(()) => {
                            metrics::record_eviction(EvictionTier::Blob, frees);
                            tracing::info!(dir = %dir_name, freed_bytes = frees, "purged blobs");
                        }
                        Err(e) => tracing::warn!(error = %e, dir = %dir_name, "blob purge failed"),
                    }
                }
                Reclaim::Evict { dir_name, frees } => {
                    let path = self.data_dir.join("repos").join(&dir_name);
                    let lock = self.lock_for_dir(&dir_name).await;
                    // INVARIANT: only a writer may delete an entry — a reader
                    // must never observe a partially deleted repository.
                    let Ok(_write) = lock.try_write() else {
                        continue;
                    };
                    match std::fs::remove_dir_all(&path) {
                        Ok(()) => {
                            metrics::record_eviction(EvictionTier::Full, frees);
                            tracing::info!(dir = %dir_name, freed_bytes = frees, "evicted repo");
                        }
                        Err(e) => tracing::warn!(error = %e, dir = %dir_name, "eviction failed"),
                    }
                }
            }
        }

        // Re-read: the plan is what we intended, not what we achieved — a
        // step can fail, and an in-use entry is skipped entirely.
        let remaining = self.candidates().await;
        let accounted: u64 = remaining.iter().map(|c| c.size_bytes).sum();
        let after = self.effective_used(accounted);
        self.gauges
            .set(after, self.budget.total_bytes, remaining.len() as u64);
        if self.budget.over_high_watermark(after) {
            tracing::warn!(
                used_bytes = after,
                high_watermark = self.budget.high_watermark(),
                "nothing left to reclaim and still over the high watermark; refusing admission"
            );
            metrics::record_admission_reject();
            return Admission::Rejected;
        }
        Admission::Ok
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
        let Ok(entries) = std::fs::read_dir(self.data_dir.join("repos")) else {
            return Vec::new();
        };

        let mut candidates = Vec::new();
        for entry in entries.flatten() {
            let Some(dir_name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            let Some(meta) = RepoMeta::load(&entry.path()) else {
                continue;
            };
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

    pub(crate) async fn purge_blobs_by_dir(&self, dir_name: &str) -> Result<(), StoreError> {
        let entry_dir = self.data_dir.join("repos").join(dir_name);
        if !entry_dir.join("repo.git").is_dir() {
            return Ok(());
        }

        // A promoted entry has no promisor remote behind it: re-marking its
        // packs would make git tolerate blobs nothing can serve again.
        if RepoMeta::load(&entry_dir).is_none_or(|meta| meta.full_clone) {
            return Ok(());
        }

        let lock = self.lock_for_dir(dir_name).await;
        // INVARIANT: repack DELETES packs — it must run with zero readers.
        let Ok(_write) = lock.try_write() else {
            return Ok(());
        };

        self.repack_blobless(&entry_dir).await.map(|_| ())
    }

    /// Current cache usage, as accounted per entry.
    pub async fn used_bytes(&self) -> u64 {
        self.candidates().await.iter().map(|c| c.size_bytes).sum()
    }
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
    (meta.cred_fingerprint == fingerprint).then_some(meta)
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

    fn refresh() -> Freshness {
        Freshness::Refresh {
            max_staleness: Duration::from_mins(5),
        }
    }

    fn always_fetch() -> Freshness {
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
        assert_eq!(
            meta.cred_fingerprint,
            creds().fingerprint(),
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
        if let Err(e) =
            crate::engine::read::blobs::prefetch(f.store.runner(), guard.git_dir(), &[head], &creds())
                .await
        {
            panic!("prefetch: {e}");
        }
        drop(guard);

        (f, k, skeleton)
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

        let reader = match f.store.open(&k, &creds(), Freshness::Pinned { generation: 1 }).await {
            Ok(g) => g,
            Err(e) => panic!("pinned open: {e}"),
        };
        f.store.purge_if_drifted(&k).await;

        assert_eq!(
            dir_size(&entry_dir.join("repo.git")),
            inflated,
            "repack deletes packs; it must never run under a reader"
        );
        drop(reader);
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

        let pinned = match f
            .store
            .open(&k, &creds(), Freshness::Pinned { generation })
            .await
        {
            Ok(g) => g,
            Err(e) => panic!("pinned open: {e}"),
        };
        assert_ne!(
            head_of(pinned.git_dir()),
            head_of(&f.root.join("origin").join(".git")),
            "a continuation page must not contact origin"
        );
        drop(pinned);

        open_until_ready(&f, &k, always_fetch()).await;
        let stale_page = f
            .store
            .open(&k, &creds(), Freshness::Pinned { generation })
            .await;
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
        assert_eq!(
            meta.cred_fingerprint,
            intruder.fingerprint(),
            "the fingerprint tracks whoever last proved access"
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
                    assert_eq!(
                        meta.cred_fingerprint,
                        who.fingerprint(),
                        "served a snapshot proved by someone else's credentials"
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
        let refusal =
            crate::engine::read::blobs::prefetch(f.store.runner(), &git_dir, &[newest], &creds())
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

        let _ = f.store.admit().await;
        assert!(
            guard.git_dir().is_dir(),
            "a repository with a live reader must never be deleted"
        );
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
