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
    /// Identifies THIS clone of the entry, for the entry's whole life on disk.
    ///
    /// The generation cannot do that job alone: every clone starts at `1`, so
    /// an entry evicted and re-cloned between two pages hands the second page
    /// the same generation over a different repository state, and the
    /// continuation is sliced against a snapshot it never saw. An empty value
    /// is a pre-incarnation entry and matches no token.
    #[serde(default)]
    pub incarnation: String,
    /// Fingerprints of the credentials that have proved origin access for
    /// THIS clone — a set, not a slot: two callers holding different valid
    /// tokens (a rotation mid-sync) would otherwise evict each other's proof
    /// on every fetch and ping-pong a full origin fetch per page.
    /// INVARIANT: a warm read is served only to a caller presenting matching
    /// credentials — the cache key alone is not an authorization claim.
    /// Reset by a re-clone; bounded, oldest proof dropped first.
    #[serde(default, alias = "cred_fingerprint", deserialize_with = "one_or_many")]
    pub cred_fingerprints: Vec<String>,
    /// Set once the entry was promoted out of a partial clone because origin
    /// refuses to serve explicitly requested objects. Purged blobs cannot be
    /// fetched back on such an entry, so it skips the blob-purge reclaim tier
    /// and is only ever evicted whole.
    #[serde(default)]
    pub full_clone: bool,
}

const META_FILE: &str = "meta.json";

/// Proofs kept per entry. Rotation scenarios need two; the bound exists so a
/// caller cycling many tokens cannot grow the document without limit.
const MAX_PROVEN_FINGERPRINTS: usize = 8;

/// A document written before the set existed carries one fingerprint as a
/// plain string; accepting it keeps every warm entry warm across the deploy.
fn one_or_many<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(single) => vec![single],
        OneOrMany::Many(many) => many,
    })
}

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

    /// Whether `fingerprint` has proved origin access for this clone.
    #[must_use]
    pub fn proven(&self, fingerprint: &str) -> bool {
        self.cred_fingerprints
            .iter()
            .any(|proof| proof == fingerprint)
    }

    /// The prior proofs plus `fingerprint`, newest last, oldest dropped past
    /// the bound.
    #[must_use]
    pub fn proofs_with(previous: Option<&Self>, fingerprint: String) -> Vec<String> {
        let mut proofs: Vec<String> = previous
            .map(|meta| {
                meta.cred_fingerprints
                    .iter()
                    .filter(|proof| **proof != fingerprint)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        proofs.push(fingerprint);
        if proofs.len() > MAX_PROVEN_FINGERPRINTS {
            proofs.drain(..proofs.len() - MAX_PROVEN_FINGERPRINTS);
        }
        proofs
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
        {
            use std::io::Write as _;
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&bytes)?;
            // The rename can survive a power loss whose data blocks did not;
            // torn JSON at least fails to parse (safe re-clone), but paying
            // one fsync beats paying a re-clone.
            file.sync_all()?;
        }

        std::fs::rename(&tmp, entry_dir.join(META_FILE)).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; names carry pid/thread/counter and hold no secrets
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
            incarnation: "inc0".to_owned(),
            clone_url: "https://example.com/a.git".to_owned(),
            tenant_id: "t".to_owned(),
            source_id: "s".to_owned(),
            last_fetched_at_epoch_s: 100,
            last_accessed_at_epoch_s: 200,
            size_bytes: 42,
            skeleton_bytes: 40,
            generation: 3,
            cred_fingerprints: vec!["deadbeef".to_owned()],
            full_clone: false,
        }
    }

    #[test]
    fn a_pre_set_document_with_one_fingerprint_still_proves_access() {
        // Written by the release before proofs became a set: the field is a
        // plain string under the old name. Refusing it would cold-refetch
        // every warm entry on deploy day.
        let dir = temp_dir("legacy-fingerprint");
        let legacy = r#"{
            "clone_url": "https://example.com/a.git",
            "tenant_id": "t",
            "source_id": "s",
            "last_fetched_at_epoch_s": 100,
            "last_accessed_at_epoch_s": 200,
            "size_bytes": 42,
            "skeleton_bytes": 40,
            "generation": 3,
            "incarnation": "inc0",
            "cred_fingerprint": "deadbeef"
        }"#;
        if let Err(e) = std::fs::write(dir.join("meta.json"), legacy) {
            panic!("write legacy meta: {e}");
        }
        let Some(meta) = RepoMeta::load(&dir) else {
            panic!("a legacy document must load")
        };
        assert!(meta.proven("deadbeef"), "the old single proof must count");
        assert!(!meta.proven("other"), "and only that one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn proofs_accumulate_and_the_oldest_falls_off() {
        let mut meta = sample();
        for n in 0..12 {
            meta.cred_fingerprints = RepoMeta::proofs_with(Some(&meta), format!("fp{n}"));
        }
        assert!(!meta.proven("deadbeef"), "the oldest proof must fall off");
        assert!(meta.proven("fp11"), "the newest must be present");
        assert!(meta.proven("fp4"), "the bound keeps the last eight");
        assert!(!meta.proven("fp3"), "and nothing before them");

        let repeated = RepoMeta::proofs_with(Some(&meta), "fp11".to_owned());
        assert_eq!(
            repeated.iter().filter(|p| *p == "fp11").count(),
            1,
            "re-proving must not duplicate"
        );
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
                incarnation: "inc0".to_owned(),
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
