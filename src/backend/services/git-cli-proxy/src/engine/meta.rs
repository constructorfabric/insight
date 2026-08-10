use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Per-repo bookkeeping stored as `meta.json` next to `repo.git`. Losing it is
/// equivalent to evicting the entry (the cache is rebuildable by design).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoMeta {
    pub clone_url: String,
    pub tenant_id: String,
    pub source_id: String,
    pub last_fetched_at_epoch_s: u64,
    pub last_accessed_at_epoch_s: u64,
    pub size_bytes: u64,
    /// Size right after a clone or a blob purge — the blobless skeleton.
    /// Anything above it is transient blob weight a purge can reclaim.
    pub skeleton_bytes: u64,
    /// Bumped on every successful clone/fetch. Page tokens carry it so a
    /// paginating caller is pinned to one ref snapshot.
    pub generation: u64,
    /// Fingerprint of the credentials that last proved origin access.
    /// INVARIANT: a warm read is served only to a caller presenting matching
    /// credentials — the cache key alone is not an authorization claim.
    pub cred_fingerprint: String,
}

const META_FILE: &str = "meta.json";

/// INVARIANT: every in-flight `store` targets a distinct temporary path.
/// Concurrent writers to one entry (LRU access bumps run under the read lock)
/// would otherwise interleave into a shared `meta.json.tmp.<pid>` and rename a
/// spliced document into place, which `load` rejects and the caller re-clones.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Seconds since the UNIX epoch; saturates at zero on a pre-epoch clock.
#[must_use]
pub fn now_epoch_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl RepoMeta {
    /// Read `meta.json` from the entry dir. A missing or unparsable file is
    /// `None` — the entry is then treated as absent and re-cloned.
    #[must_use]
    pub fn load(entry_dir: &Path) -> Option<Self> {
        let bytes = std::fs::read(entry_dir.join(META_FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Atomically persist to `meta.json` (tmp + rename — a crashed writer
    /// never leaves a truncated file behind).
    ///
    /// # Errors
    ///
    /// I/O failure writing or renaming inside `entry_dir`.
    pub fn store(&self, entry_dir: &Path) -> std::io::Result<()> {
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = entry_dir.join(format!("{META_FILE}.tmp.{}.{seq}", std::process::id()));

        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(&tmp, bytes)?;

        std::fs::rename(&tmp, entry_dir.join(META_FILE)).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "git-cli-proxy-meta-{tag}-{}-{}",
            std::process::id(),
            now_epoch_s()
        ));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            panic!("create temp dir: {e}");
        }
        dir
    }

    fn sample() -> RepoMeta {
        RepoMeta {
            clone_url: "https://example.com/a.git".to_owned(),
            tenant_id: "t".to_owned(),
            source_id: "s".to_owned(),
            last_fetched_at_epoch_s: 100,
            last_accessed_at_epoch_s: 200,
            size_bytes: 42,
            skeleton_bytes: 40,
            generation: 3,
            cred_fingerprint: "deadbeef".to_owned(),
        }
    }

    #[test]
    fn store_then_load_roundtrips() {
        let dir = temp_dir("roundtrip");
        if let Err(e) = sample().store(&dir) {
            panic!("store: {e}");
        }
        assert_eq!(RepoMeta::load(&dir), Some(sample()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_and_corrupt_files_load_as_none() {
        let dir = temp_dir("corrupt");
        assert_eq!(RepoMeta::load(&dir), None, "missing file");
        if let Err(e) = std::fs::write(dir.join("meta.json"), b"{not json") {
            panic!("write: {e}");
        }
        assert_eq!(RepoMeta::load(&dir), None, "corrupt file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_stores_never_publish_a_mixed_document() {
        let dir = temp_dir("concurrent");
        let writers = 32;

        std::thread::scope(|scope| {
            for generation in 0..writers {
                let dir = &dir;
                scope.spawn(move || {
                    let meta = RepoMeta {
                        generation,
                        ..sample()
                    };
                    if let Err(e) = meta.store(dir) {
                        panic!("store: {e}");
                    }
                });
            }
        });

        let Some(published) = RepoMeta::load(&dir) else {
            panic!("a spliced document was renamed into place")
        };
        assert!(
            published.generation < writers,
            "generation must be one writer's own value"
        );
        assert_eq!(
            published,
            RepoMeta {
                generation: published.generation,
                ..sample()
            },
            "the published document must be exactly one writer's, never a splice"
        );

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary files left behind: {leftovers:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn now_epoch_s_is_recent() {
        assert!(now_epoch_s() > 1_700_000_000, "clock must be past 2023");
    }
}
