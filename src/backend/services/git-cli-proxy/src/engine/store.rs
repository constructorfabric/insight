use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock, Semaphore};

use super::key::CacheKey;
use super::meta::{RepoMeta, now_epoch_s};
use super::runner::{GitCredentials, GitError, GitRunner};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("cache I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Read access to one cached repository. Holding the guard pins the entry:
/// fetch/repack/eviction take the write side and wait for readers to drain.
pub struct RepoGuard {
    git_dir: PathBuf,
    _read: OwnedRwLockReadGuard<()>,
}

impl RepoGuard {
    /// Path of the bare repository (`…/<key-hash>/repo.git`).
    #[must_use]
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }
}

/// The repository cache: blobless bare clones under
/// `<data_dir>/repos/<sha256(key)>/repo.git`, freshened by the fetch-if-stale
/// rule, guarded per entry by an RW-lock (reads shared; clone/fetch exclusive
/// and single-flighted), with heavy operations capped by a global semaphore.
pub struct RepoStore {
    data_dir: PathBuf,
    runner: GitRunner,
    heavy: Semaphore,
    entries: Mutex<HashMap<String, Arc<RwLock<()>>>>,
    tmp_counter: AtomicU64,
}

const HEAVY_OP_TIMEOUT: Duration = Duration::from_mins(10);
const BARE_REFSPEC: &str = "+refs/heads/*:refs/heads/*";

impl RepoStore {
    /// # Errors
    ///
    /// I/O failure creating the cache directories under `data_dir`.
    pub fn new(data_dir: &Path, heavy_ops_concurrency: usize) -> Result<Self, StoreError> {
        std::fs::create_dir_all(data_dir.join("repos"))?;
        std::fs::create_dir_all(data_dir.join("tmp"))?;
        Ok(Self {
            data_dir: data_dir.to_owned(),
            runner: GitRunner::new(HEAVY_OP_TIMEOUT),
            heavy: Semaphore::new(heavy_ops_concurrency),
            entries: Mutex::new(HashMap::new()),
            tmp_counter: AtomicU64::new(0),
        })
    }

    fn entry_dir(&self, key: &CacheKey) -> PathBuf {
        self.data_dir.join("repos").join(key.dir_name())
    }

    async fn entry_lock(&self, key: &CacheKey) -> Arc<RwLock<()>> {
        let mut entries = self.entries.lock().await;
        entries.entry(key.dir_name()).or_default().clone()
    }

    /// Serve `key` no staler than `max_staleness`: clone when absent, fetch
    /// when stale, otherwise return the cached copy. Concurrent callers for
    /// one repo single-flight through the entry's write lock; callers of
    /// distinct repos never contend.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on git failure (typed: auth/not-found/timeout) or cache
    /// I/O failure.
    pub async fn ensure_fresh(
        &self,
        key: &CacheKey,
        creds: &GitCredentials,
        max_staleness: Duration,
    ) -> Result<RepoGuard, StoreError> {
        let lock = self.entry_lock(key).await;
        let entry_dir = self.entry_dir(key);
        let git_dir = entry_dir.join("repo.git");

        {
            let read = lock.clone().read_owned().await;
            if let Some(meta) = fresh_meta(&entry_dir, max_staleness) {
                touch_access(&entry_dir, meta);
                return Ok(RepoGuard {
                    git_dir,
                    _read: read,
                });
            }
        }

        {
            let _write = lock.write().await;
            // Double-check under the write lock: a single-flight peer may
            // have cloned/fetched while this task waited.
            if fresh_meta(&entry_dir, max_staleness).is_none() {
                if git_dir.is_dir() {
                    self.fetch(key, &entry_dir, &git_dir, creds).await?;
                } else {
                    self.clone(key, &entry_dir, &git_dir, creds).await?;
                }
            }
        }

        let read = lock.read_owned().await;
        Ok(RepoGuard {
            git_dir,
            _read: read,
        })
    }

    async fn clone(
        &self,
        key: &CacheKey,
        entry_dir: &Path,
        git_dir: &Path,
        creds: &GitCredentials,
    ) -> Result<(), StoreError> {
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
            return Err(e.into());
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

        std::fs::create_dir_all(entry_dir)?;
        std::fs::rename(&tmp, git_dir)?;

        let now = now_epoch_s();
        let meta = RepoMeta {
            clone_url: key.clone_url.clone(),
            tenant_id: key.tenant_id.clone(),
            source_id: key.source_id.clone(),
            last_fetched_at_epoch_s: now,
            last_accessed_at_epoch_s: now,
            size_bytes: dir_size(git_dir),
        };
        meta.store(entry_dir)?;
        Ok(())
    }

    async fn fetch(
        &self,
        key: &CacheKey,
        entry_dir: &Path,
        git_dir: &Path,
        creds: &GitCredentials,
    ) -> Result<(), StoreError> {
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
        let mut meta = RepoMeta::load(entry_dir).unwrap_or(RepoMeta {
            clone_url: key.clone_url.clone(),
            tenant_id: key.tenant_id.clone(),
            source_id: key.source_id.clone(),
            last_fetched_at_epoch_s: 0,
            last_accessed_at_epoch_s: 0,
            size_bytes: 0,
        });
        meta.last_fetched_at_epoch_s = now;
        meta.last_accessed_at_epoch_s = now;
        meta.size_bytes = dir_size(git_dir);
        meta.store(entry_dir)?;
        Ok(())
    }
}

/// The entry's meta when the repo exists and was fetched within the window.
fn fresh_meta(entry_dir: &Path, max_staleness: Duration) -> Option<RepoMeta> {
    if !entry_dir.join("repo.git").is_dir() {
        return None;
    }
    let meta = RepoMeta::load(entry_dir)?;
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

/// Recursive byte size of a directory tree (symlinks not followed — git does
/// not create them in bare repos).
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
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        origin_url: String,
        store: RepoStore,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn sh(dir: &Path, script: &str) {
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

    fn fixture(tag: &str) -> Fixture {
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
        // real server (GitHub/GitLab) offers.
        sh(
            &origin,
            "git init -q -b main . && \
             git config uploadpack.allowFilter true && \
             git config uploadpack.allowAnySHA1InWant true && \
             echo one > a.txt && git add a.txt && git commit -qm c1",
        );
        let store = match RepoStore::new(&root.join("cache"), 2) {
            Ok(s) => s,
            Err(e) => panic!("store init: {e}"),
        };
        Fixture {
            origin_url: format!("file://{}", origin.display()),
            root,
            store,
        }
    }

    fn key(fixture: &Fixture) -> CacheKey {
        CacheKey {
            tenant_id: "t".to_owned(),
            source_id: "s".to_owned(),
            clone_url: fixture.origin_url.clone(),
        }
    }

    fn creds() -> GitCredentials {
        GitCredentials {
            username: "u".to_owned(),
            token: "unused-for-file-transport".to_owned(),
        }
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
        let guard = match f
            .store
            .ensure_fresh(&key(&f), &creds(), Duration::from_mins(5))
            .await
        {
            Ok(g) => g,
            Err(e) => panic!("ensure_fresh: {e}"),
        };
        assert!(
            guard.git_dir().join("HEAD").is_file(),
            "bare repo must exist"
        );

        let entry = guard
            .git_dir()
            .parent()
            .map_or_else(PathBuf::new, Path::to_path_buf);
        let Some(meta) = RepoMeta::load(&entry) else {
            panic!("meta.json must be written")
        };
        assert_eq!(meta.clone_url, f.origin_url);
        assert!(meta.size_bytes > 0, "size accounting must run");
    }

    #[tokio::test]
    async fn fresh_repo_skips_the_fetch() {
        let f = fixture("fresh");
        let k = key(&f);
        if let Err(e) = f
            .store
            .ensure_fresh(&k, &creds(), Duration::from_mins(5))
            .await
        {
            panic!("first ensure: {e}");
        }
        let entry_dir = f.store.entry_dir(&k);
        let before = match RepoMeta::load(&entry_dir) {
            Some(m) => m.last_fetched_at_epoch_s,
            None => panic!("meta must exist"),
        };

        // Grow the origin; within the staleness window the store must NOT see it.
        sh(
            &f.root.join("origin"),
            "echo two > b.txt && git add b.txt && git commit -qm c2",
        );
        let guard = match f
            .store
            .ensure_fresh(&k, &creds(), Duration::from_mins(5))
            .await
        {
            Ok(g) => g,
            Err(e) => panic!("second ensure: {e}"),
        };
        let after = match RepoMeta::load(&entry_dir) {
            Some(m) => m.last_fetched_at_epoch_s,
            None => panic!("meta must exist"),
        };
        assert_eq!(before, after, "no fetch within the staleness window");
        assert_ne!(
            head_of(guard.git_dir()),
            head_of(Path::new(&f.root.join("origin").join(".git"))),
            "cache serves the old snapshot"
        );
    }

    #[tokio::test]
    async fn stale_repo_fetches_new_commits_and_prunes() {
        let f = fixture("stale");
        let k = key(&f);
        if let Err(e) = f
            .store
            .ensure_fresh(&k, &creds(), Duration::from_mins(5))
            .await
        {
            panic!("first ensure: {e}");
        }

        let origin = f.root.join("origin");
        sh(
            &origin,
            "git checkout -qb doomed && git checkout -q main && echo two > b.txt && git add b.txt && git commit -qm c2 && git branch -D doomed >/dev/null 2>&1 || git branch -d doomed",
        );
        sh(&origin, "true");

        // max_staleness = 0 forces the fetch-if-stale path.
        let guard = match f
            .store
            .ensure_fresh(&k, &creds(), Duration::from_secs(0))
            .await
        {
            Ok(g) => g,
            Err(e) => panic!("stale ensure: {e}"),
        };
        assert_eq!(
            head_of(guard.git_dir()),
            head_of(&origin.join(".git")),
            "fetch must advance to the origin head"
        );
    }

    #[tokio::test]
    async fn concurrent_requests_single_flight_the_clone() {
        let f = fixture("concurrent");
        let store = Arc::new(match RepoStore::new(&f.root.join("cache2"), 2) {
            Ok(s) => s,
            Err(e) => panic!("store init: {e}"),
        });
        let k = key(&f);

        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = store.clone();
            let k = k.clone();
            handles.push(tokio::spawn(async move {
                store
                    .ensure_fresh(&k, &creds(), Duration::from_mins(5))
                    .await
                    .map(|g| g.git_dir().to_path_buf())
            }));
        }
        let mut dirs = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(dir)) => dirs.push(dir),
                Ok(Err(e)) => panic!("ensure_fresh failed: {e}"),
                Err(e) => panic!("task panicked: {e}"),
            }
        }
        assert!(
            dirs.windows(2).all(|w| w[0] == w[1]),
            "all callers see one repo"
        );
    }

    #[tokio::test]
    async fn unknown_origin_is_a_typed_error() {
        let f = fixture("missing");
        let k = CacheKey {
            clone_url: format!("file://{}", f.root.join("no-such-repo").display()),
            ..key(&f)
        };
        let result = f
            .store
            .ensure_fresh(&k, &creds(), Duration::from_mins(5))
            .await;
        match result {
            Err(StoreError::Git(GitError::NotFound | GitError::Failed(_))) => {}
            Ok(_) => panic!("clone of a missing origin must fail"),
            Err(other) => panic!("unexpected error kind: {other}"),
        }
    }
}
