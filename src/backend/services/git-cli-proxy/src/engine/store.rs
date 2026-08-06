use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock, Semaphore, watch};

use super::key::CacheKey;
use super::meta::{RepoMeta, now_epoch_s};
use super::runner::{GitCredentials, GitError, GitRunner};

const HEAVY_OP_TIMEOUT: Duration = Duration::from_mins(30);
const INLINE_WAIT: Duration = Duration::from_secs(15);
const COLD_RETRY_AFTER: Duration = Duration::from_secs(30);
const BARE_REFSPEC: &str = "+refs/heads/*:refs/heads/*";

/// Why a refresh failed, in a form that survives being broadcast to every
/// waiter (`GitError` is not `Clone`).
#[derive(Debug, Clone)]
pub enum RefreshFailure {
    Auth,
    NotFound,
    Timeout,
    Other(String),
}

impl From<&GitError> for RefreshFailure {
    fn from(error: &GitError) -> Self {
        match error {
            GitError::AuthRejected => Self::Auth,
            GitError::NotFound => Self::NotFound,
            GitError::TimedOut(_) => Self::Timeout,
            GitError::Failed(message) => Self::Other(message.clone()),
            GitError::Io(e) => Self::Other(e.to_string()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("origin rejected the supplied git credentials")]
    AuthRejected,
    #[error("repository not found at origin")]
    NotFound,
    #[error("repository is being prepared; retry in {}s", retry_after.as_secs())]
    Busy { retry_after: Duration },
    #[error("repository snapshot changed (current generation {current})")]
    SnapshotChanged { current: u64 },
    #[error("git failed: {0}")]
    Git(String),
    #[error("cache I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

impl From<RefreshFailure> for StoreError {
    fn from(failure: RefreshFailure) -> Self {
        match failure {
            RefreshFailure::Auth => Self::AuthRejected,
            RefreshFailure::NotFound => Self::NotFound,
            RefreshFailure::Timeout => Self::Git("git timed out".to_owned()),
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

pub struct RepoStore {
    data_dir: PathBuf,
    runner: GitRunner,
    heavy: Semaphore,
    entries: Mutex<HashMap<String, Arc<RwLock<()>>>>,
    inflight: Mutex<HashMap<String, watch::Receiver<Option<RefreshResult>>>>,
    tmp_counter: AtomicU64,
}

impl RepoStore {
    /// # Errors
    ///
    /// I/O failure creating the cache directories under `data_dir`.
    pub fn new(data_dir: &Path, heavy_ops_concurrency: usize) -> Result<Self, StoreError> {
        Self::with_ca_cert(data_dir, heavy_ops_concurrency, None)
    }

    /// # Errors
    ///
    /// I/O failure creating the cache directories under `data_dir`.
    pub fn with_ca_cert(
        data_dir: &Path,
        heavy_ops_concurrency: usize,
        ca_cert_path: Option<String>,
    ) -> Result<Self, StoreError> {
        std::fs::create_dir_all(data_dir.join("repos"))?;
        std::fs::create_dir_all(data_dir.join("tmp"))?;
        Ok(Self {
            data_dir: data_dir.to_owned(),
            runner: GitRunner::new(HEAVY_OP_TIMEOUT).with_ca_cert(ca_cert_path),
            heavy: Semaphore::new(heavy_ops_concurrency),
            entries: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            tmp_counter: AtomicU64::new(0),
        })
    }

    #[must_use]
    pub fn runner(&self) -> &GitRunner {
        &self.runner
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

                let generation = self.await_refresh(key, creds, max_staleness).await?;
                let read = lock.read_owned().await;
                Ok(RepoGuard {
                    git_dir,
                    generation,
                    _read: read,
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
    ) -> Result<u64, StoreError> {
        let mut receiver = self.refresh_task(key, creds, max_staleness).await;

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

    async fn refresh_task(
        self: &Arc<Self>,
        key: &CacheKey,
        creds: &GitCredentials,
        max_staleness: Duration,
    ) -> watch::Receiver<Option<RefreshResult>> {
        let dir_name = key.dir_name();
        let mut inflight = self.inflight.lock().await;
        if let Some(existing) = inflight.get(&dir_name) {
            return existing.clone();
        }

        let (sender, receiver) = watch::channel(None);
        inflight.insert(dir_name.clone(), receiver.clone());
        drop(inflight);

        let store = self.clone();
        let key = key.clone();
        let creds = creds.clone();
        tokio::spawn(async move {
            let outcome = store.refresh(&key, &creds, max_staleness).await;
            let published = match &outcome {
                Ok(generation) => Ok(*generation),
                Err(error) => Err(RefreshFailure::from(error)),
            };
            store.inflight.lock().await.remove(&key.dir_name());
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
        let cloned = self
            .runner
            .run(
                None,
                &[
                    "clone",
                    "--bare",
                    "--filter=blob:none",
                    &key.clone_url,
                    &tmp_str,
                ],
                Some(creds),
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

        std::fs::create_dir_all(entry_dir).map_err(GitError::Io)?;
        std::fs::rename(&tmp, git_dir).map_err(GitError::Io)?;

        let now = now_epoch_s();
        let meta = RepoMeta {
            clone_url: key.clone_url.clone(),
            tenant_id: key.tenant_id.clone(),
            source_id: key.source_id.clone(),
            last_fetched_at_epoch_s: now,
            last_accessed_at_epoch_s: now,
            size_bytes: dir_size(git_dir),
            generation: 1,
            cred_fingerprint: creds.fingerprint(),
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
        // INVARIANT: the permit spans the whole fetch — the semaphore IS the
        // global heavy-ops cap.
        let _permit = self.heavy.acquire().await;

        self.runner
            .run(
                Some(git_dir),
                &["fetch", "--prune", "origin", BARE_REFSPEC],
                Some(creds),
            )
            .await?;

        let now = now_epoch_s();
        let previous = RepoMeta::load(entry_dir);
        let generation = previous.as_ref().map_or(0, |m| m.generation) + 1;
        let meta = RepoMeta {
            clone_url: key.clone_url.clone(),
            tenant_id: key.tenant_id.clone(),
            source_id: key.source_id.clone(),
            last_fetched_at_epoch_s: now,
            last_accessed_at_epoch_s: now,
            size_bytes: dir_size(git_dir),
            generation,
            cred_fingerprint: creds.fingerprint(),
        };
        meta.store(entry_dir).map_err(GitError::Io)?;
        Ok(generation)
    }

    /// Drop fetched blobs, returning the entry to its blobless skeleton.
    ///
    /// `--no-write-bitmap-index` is required, not cosmetic: `--filter` splits
    /// objects across packs, and bitmap writing assumes a single pack — with
    /// bitmaps enabled the repack fails and the blobs stay on disk.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the repack fails.
    pub async fn purge_blobs(&self, key: &CacheKey) -> Result<(), StoreError> {
        let lock = self.entry_lock(key).await;
        let entry_dir = self.entry_dir(key);
        let git_dir = entry_dir.join("repo.git");
        if !git_dir.is_dir() {
            return Ok(());
        }

        // INVARIANT: repack DELETES packs — it must run with zero readers, so
        // it takes the write side even though it changes no refs.
        let _write = lock.write().await;
        let _permit = self.heavy.acquire().await;

        self.runner
            .run(
                Some(&git_dir),
                &[
                    "repack",
                    "-a",
                    "-d",
                    "--filter=blob:none",
                    "--no-write-bitmap-index",
                ],
                None,
            )
            .await?;

        if let Some(mut meta) = RepoMeta::load(&entry_dir) {
            meta.size_bytes = dir_size(&git_dir);
            let _ = meta.store(&entry_dir);
        }
        Ok(())
    }
}

/// The entry's meta when the repo exists and the caller's credentials match
/// the ones that proved origin access.
fn usable_meta(entry_dir: &Path, fingerprint: &str) -> Option<RepoMeta> {
    if !entry_dir.join("repo.git").is_dir() {
        return None;
    }
    let meta = RepoMeta::load(entry_dir)?;
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

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            total += dir_size(&entry.path());
        } else if file_type.is_file() {
            total += entry.metadata().map_or(0, |m| m.len());
        }
    }
    total
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
            clone_url: fixture.origin_url.clone(),
        }
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
        // origin accepts anyone, so the proof succeeds and lands on a NEW
        // generation — against a real vendor the same path is where the
        // caller gets rejected.
        let outcome = f.store.open(&k, &intruder, refresh()).await;
        match outcome {
            Err(StoreError::Busy { .. }) => {}
            Ok(guard) => assert!(
                guard.generation() > warm_generation,
                "served the warm snapshot (generation {}) without re-proving access",
                guard.generation()
            ),
            Err(e) => panic!("unexpected error: {e}"),
        }

        let Some(meta) = RepoMeta::load(&f.store.entry_dir(&k)) else {
            panic!("meta must exist")
        };
        assert_eq!(
            meta.cred_fingerprint,
            intruder.fingerprint(),
            "the fingerprint tracks whoever last proved access"
        );
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
            clone_url: format!("file://{}", f.root.join("no-such-repo").display()),
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
    async fn purge_blobs_succeeds_with_bitmaps_configured() {
        let f = fixture("purge");
        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let git_dir = guard.git_dir().to_path_buf();
        drop(guard);

        // Bitmap writing on + `--filter` is exactly the combination that fails
        // without `--no-write-bitmap-index`.
        sh(&git_dir, "git config repack.writeBitmaps true");
        if let Err(e) = f.store.purge_blobs(&k).await {
            panic!("purge must survive repack.writeBitmaps=true: {e}");
        }
    }
}
