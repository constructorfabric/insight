use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Notify, OwnedRwLockReadGuard, RwLock, Semaphore, SemaphorePermit, watch};
use toolkit_security::crypto::hash::Sha256;

use super::author_index::{AuthorIndex, Refresh};
use super::disk::{Budget, Candidate, Reclaim, dir_size, needs_consolidation};
use super::key::CacheKey;
use super::meta::{RepoMeta, now_epoch_s};
use super::metrics::{self, DiskGauges, EvictionTier, FetchResult};
use super::runner::{GitCredentials, GitError, GitRunner};

pub(crate) const PREPARATION_WAIT: Duration = Duration::from_mins(55);
const ADMISSION_WAIT: Duration = Duration::from_mins(50);
const _: () = assert!(ADMISSION_WAIT.as_secs() < PREPARATION_WAIT.as_secs());
const ADMISSION_POLL: Duration = Duration::from_secs(15);
const MIN_HINTED_RESERVATION: u64 = 64 * 1024 * 1024;
const HINT_OVERHEAD_FACTOR: u64 = 2;
pub(crate) const COLD_RETRY_AFTER: Duration = Duration::from_secs(30);
const MEASURE_WAIT: Duration = Duration::from_secs(15);
const REPROOF_ATTEMPTS: usize = 2;
const BARE_REFSPEC: &str = "+refs/heads/*:refs/heads/*";
const DRIFT_CHECK_INTERVAL: Duration = Duration::from_mins(1);
const PURGE_ESCALATION_AFTER: u32 = 3;
const PURGE_FAILURES_BEFORE_EVICTION: u32 = 2;
const PURGE_PRESSURE_CAP_DIVISOR: u64 = 2;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlobPurge {
    Purged,
    Evicted,
    Skipped,
}

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

    #[must_use]
    pub fn incarnation(&self) -> &str {
        &self.incarnation
    }
}

type RefreshResult = Result<u64, RefreshFailure>;

#[derive(Debug)]
pub struct Reservation<'a> {
    reserved: &'a AtomicU64,
    freed: &'a Notify,
    bytes: u64,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.reserved.fetch_sub(self.bytes, Ordering::Relaxed);
        self.freed.notify_waiters();
    }
}

#[derive(Debug)]
struct DriftState {
    checked: Instant,
    losses: u32,
    purge_failures: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RefreshKind {
    Sync,
    Promote,
}

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
    drift: Mutex<HashMap<String, DriftState>>,
    reserved_bytes: AtomicU64,
    admission: Mutex<()>,
    space_freed: Notify,
    authors: AuthorIndex,
}

impl RepoStore {
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

    #[cfg(test)]
    pub(crate) fn with_heavy_timeout(mut self, heavy: Duration) -> Self {
        self.runner = self.runner.with_timeouts(super::runner::Timeouts {
            heavy,
            ..super::runner::Timeouts::default()
        });
        self
    }

    pub fn open_cache(
        data_dir: &Path,
        heavy_ops_concurrency: usize,
        ca_cert_path: Option<String>,
        budget: Budget,
        max_repo_bytes: u64,
    ) -> Result<Self, StoreError> {
        std::fs::create_dir_all(data_dir.join("repos"))?;
        let tmp = data_dir.join("tmp");
        if tmp.is_dir()
            && let Err(e) = std::fs::remove_dir_all(&tmp)
        {
            tracing::warn!(error = %e, "could not clear staging leftovers; stranded clones stay invisible to the budget");
        }
        std::fs::create_dir_all(&tmp)?;

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
        let authors = AuthorIndex::open(data_dir)?;
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
            space_freed: Notify::new(),
            authors,
        })
    }

    #[must_use]
    pub fn runner(&self) -> &GitRunner {
        &self.runner
    }

    #[must_use]
    pub fn authors(&self) -> &AuthorIndex {
        &self.authors
    }

    #[must_use]
    pub fn gauges(&self) -> &Arc<DiskGauges> {
        &self.gauges
    }

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

    async fn heavy_permit(&self) -> tokio::sync::SemaphorePermit<'_> {
        match self.heavy.acquire().await {
            Ok(permit) => permit,
            Err(_) => unreachable!("the heavy semaphore is never closed"),
        }
    }

    pub async fn open(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
        freshness: Freshness,
    ) -> Result<RepoGuard, StoreError> {
        self.open_with_size_hint(key, creds, freshness, None).await
    }

    pub async fn open_with_size_hint(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
        freshness: Freshness,
        size_hint: Option<u64>,
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
                let read = read_within(&lock, PREPARATION_WAIT).await?;
                let meta = usable_meta(&entry_dir, &fingerprint)
                    .ok_or(StoreError::SnapshotChanged { current: 0 })?;
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
                    let read = read_within(&lock, PREPARATION_WAIT).await?;
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

                for _ in 0..REPROOF_ATTEMPTS {
                    self.await_refresh(key, creds, max_staleness, RefreshKind::Sync, size_hint)
                        .await?;

                    let read = read_within(&lock, PREPARATION_WAIT).await?;
                    if let Some(meta) = usable_meta(&entry_dir, &fingerprint) {
                        return Ok(RepoGuard {
                            git_dir,
                            incarnation: meta.incarnation,
                            generation: meta.generation,
                            _read: read,
                        });
                    }
                }

                Err(StoreError::Busy {
                    retry_after: COLD_RETRY_AFTER,
                })
            }
        }
    }

    async fn await_refresh(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
        max_staleness: Duration,
        kind: RefreshKind,
        size_hint: Option<u64>,
    ) -> Result<u64, StoreError> {
        let mut receiver = self
            .refresh_task(key, creds, max_staleness, kind, size_hint)
            .await;

        let waited =
            tokio::time::timeout(PREPARATION_WAIT, receiver.wait_for(Option::is_some)).await;
        match waited {
            Ok(Ok(seen)) => match seen.clone() {
                Some(Ok(generation)) => Ok(generation),
                Some(Err(failure)) => Err(failure.into()),
                None => Err(StoreError::Busy {
                    retry_after: COLD_RETRY_AFTER,
                }),
            },
            Ok(Err(_)) | Err(_) => Err(StoreError::Busy {
                retry_after: COLD_RETRY_AFTER,
            }),
        }
    }

    async fn refresh_task(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
        max_staleness: Duration,
        kind: RefreshKind,
        size_hint: Option<u64>,
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
            let work = tokio::spawn({
                let store = Arc::clone(&store);
                let key = key.clone();
                let creds = creds.clone();
                async move {
                    match kind {
                        RefreshKind::Sync => {
                            store.refresh(&key, &creds, max_staleness, size_hint).await
                        }
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
        size_hint: Option<u64>,
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
        if git_dir.is_dir() && RepoMeta::load(&entry_dir).is_none() {
            match std::fs::remove_dir_all(&entry_dir) {
                Ok(()) => {
                    self.drift.lock().await.remove(&key.dir_name());
                    tracing::warn!(dir = %key.dir_name(), "removed an entry without readable metadata; re-cloning");
                }
                Err(e) => {
                    tracing::error!(error = %e, dir = %key.dir_name(), "could not remove a metadata-less entry; it stays unserveable");
                }
            }
        }
        let refreshed = if git_dir.is_dir() {
            self.fetch(key, &entry_dir, &git_dir, creds).await
        } else {
            self.clone(key, &entry_dir, &git_dir, creds, size_hint)
                .await
        };
        if matches!(refreshed, Err(GitError::NotFound))
            && let Err(e) = self
                .authors
                .forget(
                    &key.tenant_id,
                    &key.source_id,
                    &key.dir_name(),
                    &fingerprint,
                )
                .await
        {
            tracing::warn!(error = %e, dir = %key.dir_name(), "could not forget a vanished repository's authors");
        }
        refreshed
    }

    async fn clone(
        &self,
        key: &CacheKey,
        entry_dir: &Path,
        git_dir: &Path,
        creds: &GitCredentials,
        size_hint: Option<u64>,
    ) -> Result<u64, GitError> {
        // INVARIANT: the permit and the reservation both span the whole clone —
        // the semaphore IS the global heavy-ops cap, and a reservation dropped
        // early admits a concurrent caller against headroom this one has not
        // finished consuming.
        let (_permit, _reserved) = self.admit_within(entry_dir, size_hint).await?;

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
        if git_dir.is_dir() {
            tracing::warn!(dir = %git_dir.display(), "removing a leftover repository before installing the clone");
            remove_tree_off_reactor(git_dir.to_path_buf())
                .await
                .map_err(GitError::Io)?;
        }
        std::fs::rename(&tmp, git_dir).map_err(GitError::Io)?;

        self.build_page_index(git_dir, 1, creds).await;
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
            skeleton_packs: pack_names(git_dir).into_iter().collect(),
            generation: 1,
            incarnation: self.mint_incarnation(),
            cred_fingerprints: vec![creds.fingerprint()],
            full_clone: false,
        };
        self.index_authors(key, git_dir, creds, &meta).await;
        meta.store(entry_dir).map_err(GitError::Io)?;
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
        // INVARIANT: the permit and the reservation both span the whole fetch —
        // the semaphore IS the global heavy-ops cap, and a reservation dropped
        // early admits a concurrent caller against headroom this one has not
        // finished consuming.
        let (permit, _reserved) = self.admit_within(entry_dir, None).await?;

        if let Err(e) = self.repack_if_drifted(entry_dir, &permit).await {
            tracing::warn!(error = %e, "pre-fetch purge failed; the cap check may see transient weight");
        }

        let before = self.ref_digest(git_dir).await;
        let previous = RepoMeta::load(entry_dir);
        let packs_before = pack_names(git_dir);

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
            unpark_meta(entry_dir);
            metrics::record_fetch(FetchResult::Error);
            return Err(e);
        }

        self.track_origin_head(git_dir, creds).await;

        let after = self.ref_digest(git_dir).await;
        let unchanged = before.is_some() && before == after;

        let fetched_bytes = dir_size_off_reactor(git_dir.to_path_buf()).await;
        if fetched_bytes > self.max_repo_bytes {
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
            skeleton_bytes: previous
                .as_ref()
                .map_or(fetched_bytes, |m| m.skeleton_bytes.min(fetched_bytes)),
            generation,
            incarnation: previous
                .as_ref()
                .map_or_else(|| self.mint_incarnation(), |m| m.incarnation.clone()),
            cred_fingerprints: RepoMeta::proofs_with(previous.as_ref(), creds.fingerprint()),
            full_clone: previous.as_ref().is_some_and(|m| m.full_clone),
            skeleton_packs: skeleton_packs_after_fetch(
                previous
                    .as_ref()
                    .map_or(&[], |m| m.skeleton_packs.as_slice()),
                &packs_before,
                &pack_names(git_dir),
            ),
        };
        self.index_authors(key, git_dir, creds, &meta).await;
        publish_meta(&meta, entry_dir)?;
        discard_parked_meta(entry_dir);
        Ok(generation)
    }

    pub async fn promote_to_full_clone(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
    ) -> Result<u64, StoreError> {
        self.await_refresh(key, creds, Duration::ZERO, RefreshKind::Promote, None)
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

        // INVARIANT: the permit and the reservation both span the whole
        // promotion — the semaphore IS the global heavy-ops cap, and a
        // reservation dropped early admits a concurrent caller against headroom
        // this one has not finished consuming.
        let (_permit, _reserved) = self.admit_within(&entry_dir, None).await?;

        park_meta(&entry_dir);

        // Both must go BEFORE the refetch, or it re-applies the blob filter.
        for key_name in ["remote.origin.promisor", "remote.origin.partialclonefilter"] {
            let _ = self
                .runner
                .run(Some(&git_dir), &["config", "--unset-all", key_name], None)
                .await;
        }

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
            skeleton_bytes: promoted_bytes,
            generation,
            incarnation: previous
                .as_ref()
                .map_or_else(|| self.mint_incarnation(), |m| m.incarnation.clone()),
            cred_fingerprints: RepoMeta::proofs_with(previous.as_ref(), creds.fingerprint()),
            full_clone: true,
            skeleton_packs: pack_names(&git_dir).into_iter().collect(),
        };
        self.index_authors(key, &git_dir, creds, &meta).await;
        publish_meta(&meta, &entry_dir)?;
        discard_parked_meta(&entry_dir);
        Ok(generation)
    }

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

        let (measured, meta) = {
            let Ok(_read) = read_within(&lock, MEASURE_WAIT).await else {
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

        let packs = pack_count(&git_dir);
        if meta.full_clone || !needs_consolidation(measured, meta.skeleton_bytes, packs) {
            self.settle_purge_debt(&key.dir_name()).await;
            return;
        }

        let under_pressure = measured >= self.max_repo_bytes / PURGE_PRESSURE_CAP_DIVISOR;
        let _write = if let Ok(guard) = lock.try_write() {
            guard
        } else {
            if !under_pressure && !self.purge_debt_due(&key.dir_name()).await {
                return;
            }
            metrics::record_purge_escalation();
            tracing::info!(dir = %key.dir_name(), under_pressure, "purge starved by readers; queueing for the entry lock");
            lock.write().await
        };
        self.settle_purge_debt(&key.dir_name()).await;

        let shed = match self.shed_window_packs(&entry_dir).await {
            WindowShed::Settled { freed: 0 } => {
                self.settle_purge_failures(&key.dir_name()).await;
                tracing::debug!(dir = %key.dir_name(), "no window to shed; skeleton size re-baselined");
                return;
            }
            WindowShed::Settled { freed } => {
                self.settle_purge_failures(&key.dir_name()).await;
                metrics::record_eviction(EvictionTier::Blob);
                tracing::info!(dir = %key.dir_name(), freed_bytes = freed, "purged a served window");
                return;
            }
            WindowShed::RepackDue { freed } => freed,
            WindowShed::Untracked => 0,
        };

        let permit = self.heavy_permit().await;
        match self.repack_blobless(&entry_dir, &permit).await {
            Ok(freed) => {
                self.settle_purge_failures(&key.dir_name()).await;
                metrics::record_eviction(EvictionTier::Blob);
                tracing::info!(dir = %key.dir_name(), freed_bytes = shed + freed, "purged a served window and consolidated the skeleton");
            }
            Err(e) => {
                let failures = self.record_purge_failure(&key.dir_name()).await;
                if failures < PURGE_FAILURES_BEFORE_EVICTION {
                    tracing::warn!(error = %e, dir = %key.dir_name(), failures, "post-serve purge failed");
                    return;
                }
                tracing::warn!(error = %e, dir = %key.dir_name(), failures, "purge cannot shed this entry; evicting it so the next open re-clones the skeleton");
                self.evict_locked(
                    &key.dir_name(),
                    entry_dir,
                    measured,
                    EvictionTier::PurgeExhausted,
                )
                .await;
            }
        }
    }

    async fn record_purge_failure(&self, dir_name: &str) -> u32 {
        let mut drift = self.drift.lock().await;
        match drift.get_mut(dir_name) {
            Some(state) => {
                state.purge_failures += 1;
                state.purge_failures
            }
            None => 1,
        }
    }

    async fn settle_purge_failures(&self, dir_name: &str) {
        if let Some(state) = self.drift.lock().await.get_mut(dir_name) {
            state.purge_failures = 0;
        }
    }

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

    fn purge_under_pressure(self: &Arc<Self>, key: &CacheKey) -> tokio::task::JoinHandle<()> {
        let store = Arc::clone(self);
        let key = key.clone();
        tokio::spawn(async move {
            {
                let mut drift = store.drift.lock().await;
                let state = drift.entry(key.dir_name()).or_insert_with(|| DriftState {
                    checked: Instant::now(),
                    losses: 0,
                    purge_failures: 0,
                });
                if let Some(due) = Instant::now().checked_sub(DRIFT_CHECK_INTERVAL) {
                    state.checked = due;
                }
                state.losses = state.losses.max(PURGE_ESCALATION_AFTER - 1);
            }
            store.purge_if_drifted(&key).await;
        })
    }

    async fn has_headroom(&self) -> bool {
        if !self.budget.is_bounded() {
            return true;
        }
        let candidates = self.candidates().await;
        let accounted: u64 = candidates.iter().map(|c| c.size_bytes).sum();
        let used = self.effective_used(accounted) + self.reserved_bytes.load(Ordering::Relaxed);
        self.gauges
            .set(used, self.budget.total_bytes, candidates.len() as u64);
        !self.budget.over_high_watermark(used)
    }

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
        if let WindowShed::Settled { .. } = self.shed_window_packs(entry_dir).await {
            return Ok(());
        }
        self.repack_blobless(entry_dir, permit).await.map(|_| ())
    }

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
                        purge_failures: 0,
                    },
                );
                true
            }
        }
    }

    // Two git behaviours make the obvious invocation free nothing at all:
    //
    // - `repack` repacks promisor packs SEPARATELY and never applies
    //   `--filter` to them. In a blobless clone every pack is a promisor pack
    //   — the clone's own and one per lazy fetch — so the filter has nothing
    //   to act on. The markers come off first, and go back on after, because
    //   the objects did come from origin and git must keep tolerating the
    //   ones this purge is about to drop.
    // - `--filter` alone writes the filtered-out objects to a second pack
    //   beside the first. `--filter-to` is what puts them somewhere the
    //   purge can delete.
    //
    // `--no-write-bitmap-index` is required, not cosmetic: the filter splits
    // objects across packs, and bitmap writing assumes a single pack — with
    // bitmaps enabled the repack fails and the blobs stay on disk.
    async fn repack_blobless(
        &self,
        entry_dir: &Path,
        _permit: &tokio::sync::SemaphorePermit<'_>,
    ) -> Result<u64, StoreError> {
        let git_dir = entry_dir.join("repo.git");
        let before = dir_size_off_reactor(git_dir.clone()).await;

        let evicted = self.data_dir.join("tmp").join(format!(
            "evicted-{}",
            self.tmp_counter.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&evicted)?;
        // WORKAROUND: git uses --filter-to as a pack base name, not a
        // directory — packs land as `<value>-<sha>.pack` siblings of it.
        // Pointing the base inside the directory keeps them collectable.
        let filter_to = format!("--filter-to={}", evicted.join("pack").display());

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
            meta.skeleton_packs = pack_names(&git_dir).into_iter().collect();
            if let Err(e) = meta.store(entry_dir) {
                tracing::warn!(error = %e, "could not record the purged size; the planner will overstate this entry");
            }
        }
        Ok(before.saturating_sub(purged))
    }
}

impl RepoStore {
    async fn admit_within(
        &self,
        entry_dir: &Path,
        size_hint: Option<u64>,
    ) -> Result<(SemaphorePermit<'_>, Reservation<'_>), GitError> {
        let deadline = tokio::time::Instant::now() + ADMISSION_WAIT;
        loop {
            // Registered before the check so a release between the check and
            // the wait is not missed.
            let freed = self.space_freed.notified();
            tokio::pin!(freed);
            freed.as_mut().enable();

            let permit = self.heavy_permit().await;
            if let Some(reserved) = self.admit(entry_dir, size_hint).await {
                return Ok((permit, reserved));
            }
            drop(permit);

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(GitError::AdmissionRejected);
            }
            let _ = tokio::time::timeout(remaining.min(ADMISSION_POLL), freed).await;
        }
    }

    async fn admit(&self, entry_dir: &Path, size_hint: Option<u64>) -> Option<Reservation<'_>> {
        let want = self.headroom_for(entry_dir, size_hint).await;

        let _decision = self.admission.lock().await;

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
                Reclaim::PurgeBlobs { dir_name, frees } => {
                    match self.purge_blobs_by_dir(&dir_name).await {
                        BlobPurge::Purged => {
                            metrics::record_eviction(EvictionTier::Blob);
                            tracing::info!(dir = %dir_name, freed_bytes = frees, "purged blobs");
                        }
                        BlobPurge::Evicted | BlobPurge::Skipped => {}
                    }
                }
                Reclaim::Evict { dir_name, frees } => {
                    self.evict_dir(&dir_name, frees).await;
                }
            }
        }

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

    async fn evict_dir(&self, dir_name: &str, frees: u64) {
        let path = self.data_dir.join("repos").join(dir_name);
        let lock = self.lock_for_dir(dir_name).await;
        let Ok(_write) = lock.try_write() else {
            return;
        };
        self.evict_locked(dir_name, path, frees, EvictionTier::Full)
            .await;
    }

    async fn evict_locked(&self, dir_name: &str, path: PathBuf, frees: u64, tier: EvictionTier) {
        match remove_tree_off_reactor(path).await {
            Ok(()) => {
                self.drift.lock().await.remove(dir_name);
                metrics::record_eviction(tier);
                self.space_freed.notify_waiters();
                tracing::info!(dir = %dir_name, freed_bytes = frees, tier = tier.as_str(), "evicted repo");
            }
            Err(e) => tracing::warn!(error = %e, dir = %dir_name, "eviction failed"),
        }
    }

    async fn headroom_for(&self, entry_dir: &Path, size_hint: Option<u64>) -> u64 {
        if !self.budget.is_bounded() || self.max_repo_bytes == u64::MAX {
            return 0;
        }
        let git_dir = entry_dir.join("repo.git");
        if !git_dir.is_dir()
            && let Some(hint) = size_hint
        {
            return hinted_reservation(hint, self.max_repo_bytes);
        }
        let measured = dir_size_off_reactor(git_dir).await;
        self.max_repo_bytes.saturating_sub(measured)
    }

    fn reserve(&self, bytes: u64) -> Reservation<'_> {
        self.reserved_bytes.fetch_add(bytes, Ordering::Relaxed);
        Reservation {
            reserved: &self.reserved_bytes,
            freed: &self.space_freed,
            bytes,
        }
    }

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

    async fn index_authors(
        &self,
        key: &CacheKey,
        git_dir: &Path,
        creds: &GitCredentials,
        meta: &RepoMeta,
    ) {
        let refresh = Refresh {
            tenant_id: key.tenant_id.clone(),
            source_id: key.source_id.clone(),
            dir_name: key.dir_name(),
            clone_url: key.clone_url.as_str().to_owned(),
            incarnation: meta.incarnation.clone(),
            generation: meta.generation,
            fingerprints: meta.cred_fingerprints.clone(),
        };

        let authors = if self.authors.needs_fold(&refresh).await {
            match crate::engine::read::authors::read(&self.runner, git_dir, creds, None).await {
                Ok(rows) => Some(rows),
                Err(e) => {
                    metrics::record_author_index_failure(metrics::AuthorIndexFailure::Fold);
                    tracing::warn!(error = %e, dir = %refresh.dir_name, "author fold failed; the entry's next refresh retries it");
                    return;
                }
            }
        } else {
            None
        };

        if let Err(e) = self.authors.record(refresh, authors).await {
            tracing::warn!(error = %e, dir = %key.dir_name(), "author index not persisted; its next update persists it");
        }
    }

    // A fetch does not move `HEAD`, so a default-branch rename at origin
    // would otherwise leave it on a branch `--prune` has just deleted.
    //
    // `git remote set-head --auto` cannot do this job here: it writes
    // `refs/remotes/origin/HEAD`, and this mirror keeps branches under
    // `refs/heads/*` with no remote-tracking namespace at all, so it fails
    // with "Not a valid ref" and changes nothing.
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

    fn effective_used(&self, accounted: u64) -> u64 {
        let available = super::disk::volume_available_bytes(&self.data_dir);
        self.budget.effective_used(accounted, available)
    }

    async fn candidates(&self) -> Vec<Candidate> {
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

    pub(crate) async fn purge_blobs_by_dir(&self, dir_name: &str) -> BlobPurge {
        let entry_dir = self.data_dir.join("repos").join(dir_name);
        if !entry_dir.join("repo.git").is_dir() {
            return BlobPurge::Skipped;
        }

        let Some(meta) = RepoMeta::load(&entry_dir).filter(|meta| !meta.full_clone) else {
            return BlobPurge::Skipped;
        };

        let lock = self.lock_for_dir(dir_name).await;
        let Ok(_write) = lock.try_write() else {
            return BlobPurge::Skipped;
        };

        match self.shed_window_packs(&entry_dir).await {
            WindowShed::Untracked => {
                self.evict_locked(dir_name, entry_dir, meta.size_bytes, EvictionTier::Full)
                    .await;
                BlobPurge::Evicted
            }
            WindowShed::Settled { freed: 0 } | WindowShed::RepackDue { freed: 0 } => {
                BlobPurge::Skipped
            }
            WindowShed::Settled { .. } | WindowShed::RepackDue { .. } => {
                self.space_freed.notify_waiters();
                BlobPurge::Purged
            }
        }
    }

    pub async fn used_bytes(&self) -> u64 {
        self.candidates().await.iter().map(|c| c.size_bytes).sum()
    }
}

fn index_len(git_dir: &Path, generation: u64) -> u64 {
    std::fs::metadata(super::index::index_path(git_dir, generation)).map_or(0, |m| m.len())
}

async fn dir_size_off_reactor(path: PathBuf) -> u64 {
    tokio::task::spawn_blocking(move || dir_size(&path))
        .await
        .unwrap_or(0)
}

async fn remove_tree_off_reactor(path: PathBuf) -> std::io::Result<()> {
    match tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&path)).await {
        Ok(result) => result,
        Err(joined) => Err(std::io::Error::other(joined)),
    }
}

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

fn parse_head_symref(listing: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let rest = line.strip_prefix("ref: ")?;
        let (reference, name) = rest.split_once('\t')?;
        (name.trim() == "HEAD" && reference.starts_with("refs/heads/"))
            .then(|| reference.to_owned())
    })
}

const PARKED_META: &str = "meta.json.parked";

fn park_meta(entry_dir: &Path) {
    let _ = std::fs::rename(entry_dir.join("meta.json"), entry_dir.join(PARKED_META));
}

fn unpark_meta(entry_dir: &Path) {
    let _ = std::fs::rename(entry_dir.join(PARKED_META), entry_dir.join("meta.json"));
}

fn discard_parked_meta(entry_dir: &Path) {
    let _ = std::fs::remove_file(entry_dir.join(PARKED_META));
}

#[derive(Debug)]
enum WindowShed {
    Settled { freed: u64 },
    RepackDue { freed: u64 },
    Untracked,
}

impl RepoStore {
    // A promisor fetch always indexes what it receives into a pack of its own,
    // so the blobs of a window never share a file with the skeleton.
    async fn shed_window_packs(&self, entry_dir: &Path) -> WindowShed {
        let Some(mut meta) = RepoMeta::load(entry_dir) else {
            return WindowShed::Untracked;
        };
        if meta.skeleton_packs.is_empty() {
            return WindowShed::Untracked;
        }

        let git_dir = entry_dir.join("repo.git");
        let before = dir_size_off_reactor(git_dir.clone()).await;
        let doomed = window_pack_files(&git_dir, &meta.skeleton_packs);
        let remaining = {
            let git_dir = git_dir.clone();
            tokio::task::spawn_blocking(move || {
                for path in doomed {
                    let _ = std::fs::remove_file(path);
                }
                dir_size(&git_dir)
            })
            .await
            .unwrap_or(before)
        };

        meta.size_bytes = remaining;
        meta.skeleton_bytes = remaining;
        if let Err(e) = meta.store(entry_dir) {
            tracing::warn!(error = %e, "could not record the shed size; the planner will overstate this entry");
        }

        let freed = before.saturating_sub(remaining);
        if needs_consolidation(remaining, remaining, pack_count(&git_dir)) {
            WindowShed::RepackDue { freed }
        } else {
            WindowShed::Settled { freed }
        }
    }
}

fn skeleton_packs_after_fetch(
    recorded: &[String],
    before: &BTreeSet<String>,
    after: &BTreeSet<String>,
) -> Vec<String> {
    if recorded.is_empty() {
        return Vec::new();
    }
    recorded
        .iter()
        .cloned()
        .chain(after.difference(before).cloned())
        .collect()
}

fn window_pack_files(git_dir: &Path, skeleton_packs: &[String]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(git_dir.join("objects").join("pack")) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            if name.starts_with("tmp_") {
                return true;
            }
            let stem = name.split('.').next().unwrap_or(name);
            stem.starts_with("pack-") && !skeleton_packs.iter().any(|kept| kept == stem)
        })
        .collect()
}

fn pack_names(git_dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(git_dir.join("objects").join("pack")).map_or_else(
        |_| BTreeSet::new(),
        |entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "pack"))
                .filter_map(|path| {
                    path.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .collect()
        },
    )
}

fn pack_count(git_dir: &Path) -> usize {
    pack_names(git_dir).len()
}

fn publish_meta(meta: &RepoMeta, entry_dir: &Path) -> Result<(), GitError> {
    meta.store(entry_dir).map_err(|e| {
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

// Without this git still treats the packs as promisor packs and keeps
// deferring to the origin for objects it now has locally.
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

// `--` is what keeps a hostile URL from being read as a git option.
fn clone_argv<'a>(url: &'a str, target: &'a str) -> [&'a str; 7] {
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

fn usable_meta(entry_dir: &Path, fingerprint: &str) -> Option<RepoMeta> {
    if !entry_dir.join("repo.git").is_dir() {
        return None;
    }
    let meta = RepoMeta::load(entry_dir)?;
    meta.proven(fingerprint).then_some(meta)
}

fn fresh_meta(entry_dir: &Path, fingerprint: &str, max_staleness: Duration) -> Option<RepoMeta> {
    let meta = usable_meta(entry_dir, fingerprint)?;
    let age = now_epoch_s().saturating_sub(meta.last_fetched_at_epoch_s);
    (age < max_staleness.as_secs()).then_some(meta)
}

fn touch_access(entry_dir: &Path, mut meta: RepoMeta) {
    meta.last_accessed_at_epoch_s = now_epoch_s();
    let _ = meta.store(entry_dir);
}

fn hinted_reservation(hint: u64, cap_bytes: u64) -> u64 {
    hint.saturating_mul(HINT_OVERHEAD_FACTOR)
        .max(MIN_HINTED_RESERVATION)
        .min(cap_bytes)
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

    pub(crate) fn fixture_url(raw: &str) -> crate::engine::url::CloneUrl {
        let Ok(url) = crate::engine::url::CloneUrl::parse(
            raw,
            crate::engine::url::CloneUrlPolicy::with_file_origins(),
        ) else {
            panic!("fixture url must parse: {raw}")
        };
        url
    }

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

    pub(crate) fn orphan_newest_commit_at_origin(f: &Fixture) {
        sh(
            &f.root.join("origin"),
            "git reset -q --hard HEAD~1 && \
             git reflog expire --expire=now --all && \
             git gc -q --prune=now",
        );
    }

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

    pub(crate) async fn open_until_ready(
        fixture: &Fixture,
        key: &CacheKey,
        freshness: Freshness,
    ) -> RepoGuard {
        for _ in 0..1200u32 {
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

    #[tokio::test]
    async fn a_clone_publishes_over_a_leftover_at_its_destination() {
        let f = fixture("leftover-destination");
        let k = key(&f);
        let entry_dir = f.store.entry_dir(&k);
        let git_dir = entry_dir.join("repo.git");
        let pack_dir = git_dir.join("objects/pack");
        if let Err(e) = std::fs::create_dir_all(&pack_dir)
            .and_then(|()| std::fs::write(pack_dir.join("tmp_pack_stray"), b"x"))
        {
            panic!("stage the leftover: {e}");
        }

        let store: &RepoStore = &f.store;
        let published = RepoStore::clone(store, &k, &entry_dir, &git_dir, &creds(), None).await;

        match published {
            Ok(generation) => assert_eq!(generation, 1, "a clone publishes generation 1"),
            Err(e) => panic!("the clone could not publish over the leftover: {e}"),
        }
        assert!(
            git_dir.join("HEAD").is_file(),
            "the clone must be the entry now"
        );
        assert!(
            !pack_dir.join("tmp_pack_stray").exists(),
            "the leftover must be gone, not merged into the published entry"
        );
    }

    async fn entry_with_fetched_blobs(tag: &str) -> (Fixture, CacheKey, u64) {
        fetch_blobs_into(fixture(tag)).await
    }

    async fn rewind_drift_throttle(store: &RepoStore) {
        for state in store.drift.lock().await.values_mut() {
            if let Some(rewound) = Instant::now().checked_sub(DRIFT_CHECK_INTERVAL) {
                state.checked = rewound;
            }
        }
    }

    fn store_whose_repack_cannot_finish(f: &Fixture) -> Arc<RepoStore> {
        match RepoStore::open_cache(
            &f.root.join("cache"),
            2,
            None,
            Budget {
                total_bytes: u64::MAX,
            },
            u64::MAX,
        ) {
            Ok(s) => Arc::new(s.with_heavy_timeout(Duration::from_millis(1))),
            Err(e) => panic!("second store: {e}"),
        }
    }

    fn forget_skeleton_packs(entry_dir: &Path) {
        let Some(mut meta) = RepoMeta::load(entry_dir) else {
            panic!("meta must exist")
        };
        meta.skeleton_packs.clear();
        if let Err(e) = meta.store(entry_dir) {
            panic!("meta store: {e}");
        }
    }

    async fn fetch_blobs_into(f: Fixture) -> (Fixture, CacheKey, u64) {
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
        let f = fixture("meta-fail");
        let k = key(&f);
        open_until_ready(&f, &k, refresh()).await;
        let entry_dir = f.store.entry_dir(&k);

        let Some(meta) = RepoMeta::load(&entry_dir) else {
            panic!("meta must exist")
        };
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
        let f = fixture_with_budget("reserve", 1_000_000, 500_000);
        let entry_dir = f.store.entry_dir(&key(&f));

        let Some(first) = f.store.admit(&entry_dir, None).await else {
            panic!("an empty cache must admit the first caller")
        };
        assert!(
            f.store.admit(&entry_dir, None).await.is_none(),
            "the second caller must be refused against the first's reservation"
        );

        drop(first);
        assert!(
            f.store.admit(&entry_dir, None).await.is_some(),
            "and admitted again once that reservation is released"
        );
    }

    #[tokio::test]
    async fn a_size_hint_reserves_the_hinted_size_not_the_cap() {
        let f = fixture_with_budget("hint", 1_000_000_000, 500_000_000);
        let cold = f
            .root
            .join("cache-bounded")
            .join("repos")
            .join("cold-entry");
        let Some(_held) = f.store.admit(&cold, None).await else {
            panic!("an empty cache must admit the first caller")
        };
        assert!(
            f.store.admit(&cold, None).await.is_none(),
            "a second cap-sized reservation must not fit"
        );
        let hinted = f.store.admit(&cold, Some(1_000)).await;
        assert!(
            hinted.is_some(),
            "a hinted reservation is the floor, not the cap, and fits beside the first"
        );
    }

    #[test]
    fn a_hinted_reservation_carries_overhead_and_stays_inside_the_cap() {
        let cases = [
            (1_000, 10_u64.pow(9), MIN_HINTED_RESERVATION),
            (200 * 1024 * 1024, 10_u64.pow(9), 400 * 1024 * 1024),
            (900 * 1024 * 1024, 10_u64.pow(9), 10_u64.pow(9)),
            (u64::MAX, 10_u64.pow(9), 10_u64.pow(9)),
        ];
        for (hint, cap, expected) in cases {
            assert_eq!(
                hinted_reservation(hint, cap),
                expected,
                "should reserve {expected} for hint {hint} under cap {cap}"
            );
        }
    }

    #[tokio::test]
    async fn a_cold_clone_waits_for_headroom_instead_of_being_refused() {
        let f = fixture_with_budget("wait-headroom", 1_000_000, 500_000);
        let elsewhere = f
            .root
            .join("cache-bounded")
            .join("repos")
            .join("other-entry");
        let Some(held) = f.store.admit(&elsewhere, None).await else {
            panic!("an empty cache must admit the first caller")
        };

        let store = Arc::clone(&f.store);
        let k = key(&f);
        let opening = tokio::spawn(async move { store.open(&k, &creds(), refresh()).await });

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !opening.is_finished(),
            "the caller must wait for headroom, not be bounced with Busy"
        );

        drop(held);
        match opening.await {
            Ok(Ok(guard)) => assert!(
                guard.git_dir().is_dir(),
                "the clone landed once room appeared"
            ),
            Ok(Err(e)) => panic!("the waiting caller must be served, got {e}"),
            Err(e) => panic!("open task died: {e}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn headroom_that_never_appears_ends_in_busy_after_the_wait() {
        let f = fixture_with_budget("wait-exhausted", 1_000_000, 500_000);
        let elsewhere = f
            .root
            .join("cache-bounded")
            .join("repos")
            .join("other-entry");
        let Some(_held) = f.store.admit(&elsewhere, None).await else {
            panic!("an empty cache must admit the first caller")
        };

        let started = tokio::time::Instant::now();
        match f.store.open(&key(&f), &creds(), refresh()).await {
            Err(StoreError::Busy { retry_after }) => assert_eq!(retry_after, COLD_RETRY_AFTER),
            Ok(_) => panic!("nothing freed the space, so the open cannot have been served"),
            Err(e) => panic!("expected Busy once the admission wait is spent, got {e}"),
        }
        let waited = started.elapsed();
        assert!(
            waited >= ADMISSION_WAIT && waited < PREPARATION_WAIT,
            "the admission wait must be what ends it, not the caller's own timeout: {waited:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_reader_outwaits_a_long_preparation_instead_of_being_bounced() {
        let lock: Arc<RwLock<()>> = Arc::new(RwLock::new(()));
        let writer = lock.clone().write_owned().await;

        let handle = tokio::spawn({
            let lock = lock.clone();
            async move { read_within(&lock, PREPARATION_WAIT).await }
        });
        tokio::time::sleep(Duration::from_mins(20)).await;
        drop(writer);

        match handle.await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => panic!("a reader must outlast a 20-minute preparation: {e}"),
            Err(e) => panic!("reader task died: {e}"),
        }
    }

    #[tokio::test]
    async fn a_reader_behind_a_writer_is_asked_back_rather_than_left_hanging() {
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
        let (f, k, skeleton) = entry_with_fetched_blobs("fetch-purge").await;
        let entry_dir = f.store.entry_dir(&k);
        let inflated = dir_size(&entry_dir.join("repo.git"));
        assert!(inflated > skeleton * 2, "the entry must carry blob weight");

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

    async fn authors_changed_since(
        f: &Fixture,
        since_us: Option<i64>,
        fingerprint: &str,
    ) -> Vec<String> {
        f.store
            .authors()
            .changes(
                "t",
                "s",
                fingerprint,
                crate::engine::author_index::FeedQuery {
                    since_us,
                    limit: 100,
                    ..crate::engine::author_index::FeedQuery::default()
                },
            )
            .await
            .items
            .into_iter()
            .map(|change| change.email)
            .collect()
    }

    #[tokio::test]
    async fn a_refresh_feeds_new_authors_to_the_change_feed_and_only_them() {
        let f = fixture("author-feed");
        let k = key(&f);
        let fingerprint = creds().fingerprint();
        drop(open_until_ready(&f, &k, refresh()).await);

        let after_clone = f
            .store
            .authors()
            .changes(
                "t",
                "s",
                &fingerprint,
                crate::engine::author_index::FeedQuery {
                    limit: 100,
                    ..crate::engine::author_index::FeedQuery::default()
                },
            )
            .await
            .items;
        assert_eq!(
            after_clone.len(),
            1,
            "the clone's one author: {after_clone:?}"
        );
        assert_eq!(after_clone[0].email, "test@example.com");
        assert_eq!(after_clone[0].sample_url, f.origin_url);
        let mark = after_clone[0].changed_us;

        sh(
            &f.root.join("origin"),
            "echo two > b.txt && git add b.txt && \
             GIT_AUTHOR_NAME=Newcomer GIT_AUTHOR_EMAIL=new@example.com \
             GIT_AUTHOR_DATE='2025-01-01T10:00:00+0000' \
             GIT_COMMITTER_DATE='2025-01-01T10:00:00+0000' git commit -qm back-dated",
        );
        drop(open_until_ready(&f, &k, always_fetch()).await);

        assert_eq!(
            authors_changed_since(&f, Some(mark), &fingerprint).await,
            vec!["new@example.com"],
            "a back-dated commit's author is new; the unchanged one is not reported again"
        );
        assert!(
            authors_changed_since(&f, None, "another-credential")
                .await
                .is_empty(),
            "a caller that never proved access to the repository sees nobody"
        );
    }

    #[tokio::test]
    async fn a_metadata_less_entry_is_recloned_not_fetched() {
        let f = fixture("meta-less-heal");
        let k = key(&f);
        drop(open_until_ready(&f, &k, refresh()).await);

        let entry_dir = f.store.entry_dir(&k);
        let git_dir = entry_dir.join("repo.git");
        if let Err(e) = std::fs::remove_file(entry_dir.join("meta.json")) {
            panic!("stage: {e}");
        }
        if let Err(e) = std::fs::remove_dir_all(&git_dir) {
            panic!("stage: {e}");
        }
        if let Err(e) = std::fs::create_dir_all(git_dir.join("objects")) {
            panic!("stage: {e}");
        }

        let guard = open_until_ready(&f, &k, refresh()).await;
        assert!(
            guard.git_dir().join("HEAD").is_file(),
            "the entry must come back as a working clone"
        );
        assert!(
            RepoMeta::load(&entry_dir).is_some(),
            "the re-clone must publish fresh metadata"
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
    async fn a_purge_leaves_nothing_behind_in_the_staging_dir() {
        let (f, k, _) = entry_with_fetched_blobs("purge-staging").await;
        f.store.purge_if_drifted(&k).await;

        let tmp = f.root.join("cache").join("tmp");
        let leftovers: Vec<String> = std::fs::read_dir(&tmp)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            leftovers.is_empty(),
            "a purge must collect everything it staged: {leftovers:?}"
        );
    }

    #[tokio::test]
    async fn accounting_reflects_blobs_even_when_no_purge_is_warranted() {
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
        let (f, k, skeleton) = entry_with_fetched_blobs("drift-escalate").await;
        let entry_dir = f.store.entry_dir(&k);
        let inflated = dir_size(&entry_dir.join("repo.git"));

        let reader = match f.store.open(&k, &creds(), pinned(&f, &k, 1)).await {
            Ok(g) => g,
            Err(e) => panic!("pinned open: {e}"),
        };

        for lost in 1..PURGE_ESCALATION_AFTER {
            rewind_drift_throttle(&f.store).await;
            f.store.purge_if_drifted(&k).await;
            assert_eq!(
                dir_size(&entry_dir.join("repo.git")),
                inflated,
                "loss {lost} must stay opportunistic under a reader"
            );
        }

        rewind_drift_throttle(&f.store).await;
        let escalated = tokio::spawn({
            let store = Arc::clone(&f.store);
            let k = k.clone();
            async move { store.purge_if_drifted(&k).await }
        });

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
            "the purge must have run: {} vs skeleton {skeleton}",
            meta.size_bytes
        );
        assert!(
            !f.store.purge_debt_due(&k.dir_name()).await,
            "a completed repack must reset the escalation counter"
        );
    }

    #[tokio::test]
    async fn a_fetch_of_a_drifted_entry_completes_with_one_heavy_permit() {
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
    async fn enumerating_a_window_with_an_edited_rename_fetches_no_blob_on_its_own() {
        let f = fixture("prefetch-rename");
        sh(
            &f.root.join("origin"),
            "git mv a.txt b.txt && echo two >> b.txt && git add b.txt && \
             GIT_AUTHOR_DATE='2026-08-01T11:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-01T11:00:00+0000' git commit -qm rename",
        );
        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let head = head_of(guard.git_dir());

        let fetched = crate::engine::read::blobs::prefetch(
            f.store.runner(),
            guard.git_dir(),
            &[head],
            &creds(),
            u64::MAX,
        )
        .await;
        match fetched {
            Ok(count) => assert_eq!(
                count, 2,
                "both sides of the rename must still be missing when the batch fetch runs"
            ),
            Err(e) => panic!("prefetch: {e}"),
        }
    }

    #[tokio::test]
    async fn a_purge_that_keeps_failing_evicts_the_entry_instead_of_looping() {
        let (f, k, _) = entry_with_fetched_blobs("purge-exhausted").await;
        let entry_dir = f.store.entry_dir(&k);
        assert!(
            entry_dir.join("repo.git").is_dir(),
            "the fixture must hold a clone"
        );
        forget_skeleton_packs(&entry_dir);

        let failing = store_whose_repack_cannot_finish(&f);

        failing.purge_if_drifted(&k).await;
        assert!(
            entry_dir.join("repo.git").is_dir(),
            "one failed purge is a retry, not an eviction"
        );

        rewind_drift_throttle(&failing).await;
        failing.purge_if_drifted(&k).await;
        assert!(
            !entry_dir.exists(),
            "the second consecutive failure must evict the entry"
        );

        let guard = open_until_ready(&f, &k, refresh()).await;
        assert_eq!(
            guard.generation(),
            1,
            "the next open re-clones from scratch"
        );
        assert!(
            RepoMeta::load(&entry_dir).is_some(),
            "and publishes fresh metadata for the skeleton"
        );
    }

    #[tokio::test]
    async fn a_purge_sheds_the_window_by_deleting_its_packs_and_never_repacks_the_skeleton() {
        let (f, k, skeleton) = entry_with_fetched_blobs("shed-by-deletion").await;
        let entry_dir = f.store.entry_dir(&k);
        let git_dir = entry_dir.join("repo.git");
        let Some(before) = RepoMeta::load(&entry_dir) else {
            panic!("meta must exist")
        };
        assert!(
            !before.skeleton_packs.is_empty(),
            "a clone must record its skeleton packs"
        );
        assert!(
            pack_count(&git_dir) > before.skeleton_packs.len(),
            "the prefetch must have added a window pack"
        );

        store_whose_repack_cannot_finish(&f)
            .purge_if_drifted(&k)
            .await;

        assert!(git_dir.is_dir(), "a shed is not an eviction");
        let skeleton_packs: BTreeSet<String> = before.skeleton_packs.iter().cloned().collect();
        assert_eq!(
            pack_names(&git_dir),
            skeleton_packs,
            "only the skeleton packs may remain"
        );
        let Some(after) = RepoMeta::load(&entry_dir) else {
            panic!("meta must survive a shed")
        };
        assert!(
            after.size_bytes < skeleton * 2,
            "the window's weight must be gone: {} vs skeleton {skeleton}",
            after.size_bytes
        );
        assert_eq!(
            after.size_bytes,
            dir_size(&git_dir),
            "accounting must match the disk"
        );
        assert_eq!(
            after.generation, before.generation,
            "a shed changes no snapshot"
        );
    }

    #[tokio::test]
    async fn a_fetched_pack_joins_the_skeleton_and_survives_the_shed() {
        let (f, k, _) = entry_with_fetched_blobs("shed-keeps-fetch").await;
        let entry_dir = f.store.entry_dir(&k);
        let git_dir = entry_dir.join("repo.git");

        sh(
            &f.root.join("origin"),
            "dd if=/dev/urandom of=later.bin bs=1024 count=4096 status=none && \
             git add later.bin && \
             GIT_AUTHOR_DATE='2026-08-03T10:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-03T10:00:00+0000' git commit -qm later",
        );
        let guard = match f.store.open(&k, &creds(), always_fetch()).await {
            Ok(g) => g,
            Err(e) => panic!("fetch: {e}"),
        };
        let head = head_of(guard.git_dir());
        if let Err(e) = crate::engine::read::blobs::prefetch(
            f.store.runner(),
            guard.git_dir(),
            std::slice::from_ref(&head),
            &creds(),
            u64::MAX,
        )
        .await
        {
            panic!("prefetch: {e}");
        }
        drop(guard);
        let Some(meta) = RepoMeta::load(&entry_dir) else {
            panic!("meta must exist")
        };
        assert_eq!(
            meta.skeleton_packs.len(),
            2,
            "the fetch's pack must join the skeleton: {:?}",
            meta.skeleton_packs
        );

        store_whose_repack_cannot_finish(&f)
            .purge_if_drifted(&k)
            .await;

        assert_eq!(
            pack_names(&git_dir).len(),
            2,
            "the clone's and the fetch's packs stay; the windows go"
        );
        sh(&git_dir, &format!("git cat-file -e {head}^{{tree}}"));
    }

    #[tokio::test]
    async fn an_entry_that_predates_pack_tracking_is_never_shed_by_guesswork() {
        let (f, k, _) = entry_with_fetched_blobs("shed-legacy").await;
        let entry_dir = f.store.entry_dir(&k);
        let git_dir = entry_dir.join("repo.git");
        forget_skeleton_packs(&entry_dir);
        let packs = pack_names(&git_dir);

        store_whose_repack_cannot_finish(&f)
            .purge_if_drifted(&k)
            .await;

        assert!(git_dir.is_dir(), "one failed purge is a retry");
        assert_eq!(
            pack_names(&git_dir),
            packs,
            "no pack may be deleted on a guess"
        );
    }

    #[test]
    fn a_fetch_extends_a_recorded_skeleton_and_leaves_an_unrecorded_one_alone() {
        struct Case {
            rule: &'static str,
            recorded: &'static [&'static str],
            before: &'static [&'static str],
            after: &'static [&'static str],
            expected: &'static [&'static str],
        }
        let owned =
            |names: &[&str]| -> Vec<String> { names.iter().map(|n| (*n).to_owned()).collect() };
        let cases = [
            Case {
                rule: "unrecorded stays unrecorded",
                recorded: &[],
                before: &["pack-a"],
                after: &["pack-a", "pack-f"],
                expected: &[],
            },
            Case {
                rule: "the fetch's pack is added",
                recorded: &["pack-a"],
                before: &["pack-a"],
                after: &["pack-a", "pack-f"],
                expected: &["pack-a", "pack-f"],
            },
            Case {
                rule: "a window present throughout is not",
                recorded: &["pack-a"],
                before: &["pack-a", "pack-w"],
                after: &["pack-a", "pack-w", "pack-f"],
                expected: &["pack-a", "pack-f"],
            },
            Case {
                rule: "a fetch that wrote nothing adds nothing",
                recorded: &["pack-a"],
                before: &["pack-a"],
                after: &["pack-a"],
                expected: &["pack-a"],
            },
        ];
        for case in cases {
            let before: BTreeSet<String> = owned(case.before).into_iter().collect();
            let after: BTreeSet<String> = owned(case.after).into_iter().collect();
            assert_eq!(
                skeleton_packs_after_fetch(&owned(case.recorded), &before, &after),
                owned(case.expected),
                "{}",
                case.rule
            );
        }
    }

    #[test]
    fn a_window_pack_is_doomed_with_all_its_siblings_and_nothing_else() {
        let f = fixture("window-files");
        let git_dir = f.root.join("packs");
        let pack_dir = git_dir.join("objects").join("pack");
        if let Err(e) = std::fs::create_dir_all(&pack_dir) {
            panic!("pack dir: {e}");
        }
        for name in [
            "pack-a.pack",
            "pack-a.idx",
            "pack-a.promisor",
            "pack-w.pack",
            "pack-w.idx",
            "pack-w.rev",
            "pack-w.promisor",
            "multi-pack-index",
        ] {
            if let Err(e) = std::fs::write(pack_dir.join(name), b"x") {
                panic!("write {name}: {e}");
            }
        }

        let mut doomed: Vec<String> = window_pack_files(&git_dir, &["pack-a".to_owned()])
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        doomed.sort();

        assert_eq!(
            doomed,
            ["pack-w.idx", "pack-w.pack", "pack-w.promisor", "pack-w.rev"],
            "every file of the window pack and only those"
        );
    }

    #[tokio::test]
    async fn an_entry_past_half_its_cap_queues_its_purge_at_once() {
        let cap = 6_000_000;
        let (f, k, skeleton) = fetch_blobs_into(fixture_with_budget(
            "purge-pressure-half",
            1_000_000_000,
            cap,
        ))
        .await;
        let entry_dir = f.store.entry_dir(&k);
        let inflated = dir_size(&entry_dir.join("repo.git"));
        assert!(
            inflated >= cap / PURGE_PRESSURE_CAP_DIVISOR && inflated < cap,
            "the fixture must sit between half the cap and the cap: {inflated}"
        );

        let reader = match f.store.open(&k, &creds(), pinned(&f, &k, 1)).await {
            Ok(g) => g,
            Err(e) => panic!("pinned open: {e}"),
        };
        let purge = tokio::spawn({
            let store = Arc::clone(&f.store);
            let k = k.clone();
            async move { store.purge_if_drifted(&k).await }
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop(reader);
        if let Err(e) = purge.await {
            panic!("the purge task must not panic: {e}");
        }

        assert!(
            dir_size(&entry_dir.join("repo.git")) < skeleton * 2,
            "past half the cap the first probe must queue and reclaim, not yield"
        );
    }

    #[tokio::test]
    async fn a_pressure_purge_needs_no_throttle_window_and_no_repeated_losses() {
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

        let lock = f.store.entry_lock(&k).await;
        let held = lock.write_owned().await;

        let mine = f
            .store
            .refresh_task(&k, &creds(), Duration::ZERO, RefreshKind::Sync, None)
            .await;
        let mine_again = f
            .store
            .refresh_task(&k, &creds(), Duration::ZERO, RefreshKind::Sync, None)
            .await;
        assert_eq!(
            f.store.inflight.lock().await.len(),
            1,
            "the same credentials must collapse onto one flight"
        );

        let theirs = f
            .store
            .refresh_task(&k, &intruder, Duration::ZERO, RefreshKind::Sync, None)
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

        let guard = open_until_ready(
            &f,
            &k,
            Freshness::Refresh {
                max_staleness: Duration::from_mins(5),
            },
        )
        .await;

        let _ = f.store.admit(&f.store.entry_dir(&key(&f)), None).await;
        assert!(
            guard.git_dir().is_dir(),
            "a repository with a live reader must never be deleted"
        );
    }

    #[tokio::test]
    async fn a_shed_removes_what_an_interrupted_fetch_left_behind() {
        let (f, k, _) = entry_with_fetched_blobs("shed-tmp-pack").await;
        let pack_dir = f.store.entry_dir(&k).join("repo.git/objects/pack");
        let stranded = pack_dir.join("tmp_pack_abc123");
        if let Err(e) = std::fs::write(&stranded, vec![0u8; 4096]) {
            panic!("stage stranded pack: {e}");
        }

        assert_eq!(
            f.store.purge_blobs_by_dir(&k.dir_name()).await,
            BlobPurge::Purged
        );
        assert!(
            !stranded.exists(),
            "a partial pack no fetch owns must not count against the entry forever"
        );
    }

    #[tokio::test]
    async fn a_reclaim_evicts_an_entry_whose_packs_it_cannot_tell_apart() {
        let (f, k, _) = entry_with_fetched_blobs("reclaim-untracked").await;
        let entry_dir = f.store.entry_dir(&k);
        forget_skeleton_packs(&entry_dir);

        assert_eq!(
            f.store.purge_blobs_by_dir(&k.dir_name()).await,
            BlobPurge::Evicted
        );
        assert!(
            !entry_dir.exists(),
            "the entry must be gone so the next open re-clones the skeleton"
        );
    }

    #[tokio::test]
    async fn a_reclaim_leaves_a_skeleton_due_for_consolidation_to_its_post_serve_purge() {
        let f = fixture("reclaim-consolidation");
        let k = key(&f);
        drop(open_until_ready(&f, &k, refresh()).await);
        let entry_dir = f.store.entry_dir(&k);
        let git_dir = entry_dir.join("repo.git");
        sh(
            &git_dir,
            &format!(
                "for i in $(seq {}); do \
                   echo $i | git hash-object -w --stdin | \
                   git pack-objects -q objects/pack/pack >/dev/null; \
                 done",
                crate::engine::disk::PACK_CONSOLIDATION_LIMIT
            ),
        );
        let Some(mut meta) = RepoMeta::load(&entry_dir) else {
            panic!("meta must exist")
        };
        meta.skeleton_packs = pack_names(&git_dir).into_iter().collect();
        if let Err(e) = meta.store(&entry_dir) {
            panic!("meta store: {e}");
        }
        let packs = pack_names(&git_dir);

        assert_eq!(
            f.store.purge_blobs_by_dir(&k.dir_name()).await,
            BlobPurge::Skipped
        );
        assert_eq!(
            pack_names(&git_dir),
            packs,
            "no repack may run under admission"
        );
    }

    #[tokio::test]
    async fn a_post_serve_purge_succeeds_with_bitmaps_configured() {
        let (f, k, _) = entry_with_fetched_blobs("purge-bitmaps").await;
        let entry_dir = f.store.entry_dir(&k);
        sh(
            &entry_dir.join("repo.git"),
            "git config repack.writeBitmaps true",
        );
        forget_skeleton_packs(&entry_dir);

        f.store.purge_if_drifted(&k).await;

        let Some(meta) = RepoMeta::load(&entry_dir) else {
            panic!("the entry must survive its purge")
        };
        assert!(
            !meta.skeleton_packs.is_empty(),
            "a successful repack records the skeleton it left"
        );
    }
}
