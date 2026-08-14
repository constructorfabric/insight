use sha2::{Digest, Sha256};

use crate::engine::url::CloneUrl;

/// Cache identity of one repository: `(tenant, source, clone URL)`. Identical
/// clone URLs under two sources are two isolated entries — access rights
/// differ per source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub tenant_id: String,
    pub source_id: String,
    pub clone_url: CloneUrl,
}

impl CacheKey {
    /// On-disk directory name: hex sha256 over the NUL-joined key parts.
    /// Repo names / URL fragments never become paths (traversal,
    /// case-insensitive FS collisions).
    #[must_use]
    pub fn dir_name(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.tenant_id.as_bytes());
        hasher.update([0u8]);
        hasher.update(self.source_id.as_bytes());
        hasher.update([0u8]);
        hasher.update(self.clone_url.as_str().as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(tenant: &str, source: &str, url: &str) -> CacheKey {
        let Ok(clone_url) = CloneUrl::parse(url, crate::engine::url::CloneUrlPolicy::http_only())
        else {
            panic!("fixture url must parse: {url}")
        };
        CacheKey {
            tenant_id: tenant.to_owned(),
            source_id: source.to_owned(),
            clone_url,
        }
    }

    #[test]
    fn dir_name_is_stable_hex() {
        let name = key("t", "s", "https://example.com/a.git").dir_name();
        assert_eq!(name.len(), 64, "sha256 hex is 64 chars");
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(name, key("t", "s", "https://example.com/a.git").dir_name());
    }

    #[test]
    fn distinct_keys_map_to_distinct_dirs() {
        let base = key("t", "s", "https://example.com/a.git");
        let cases = vec![
            (
                "tenant differs",
                key("t2", "s", "https://example.com/a.git"),
            ),
            (
                "source differs",
                key("t", "s2", "https://example.com/a.git"),
            ),
            ("url differs", key("t", "s", "https://example.com/b.git")),
            // NUL joining keeps concatenation injective across field borders.
            (
                "field boundary shifts",
                key("ts", "", "https://example.com/a.git"),
            ),
        ];
        for (name, other) in cases {
            assert_ne!(base.dir_name(), other.dir_name(), "case: {name}");
        }
    }

    #[test]
    fn hostile_repo_names_never_reach_the_path() {
        let name = key("t", "s", "https://example.com/../../etc/passwd").dir_name();
        assert!(!name.contains('/'), "dir name must be a single component");
        assert!(!name.contains('.'), "dir name must be pure hex");
    }
}
