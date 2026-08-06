use std::path::Path;
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
}

const META_FILE: &str = "meta.json";

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
        let tmp = entry_dir.join(format!("{META_FILE}.tmp.{}", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, entry_dir.join(META_FILE))
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
    fn now_epoch_s_is_recent() {
        assert!(now_epoch_s() > 1_700_000_000, "clock must be past 2023");
    }
}
