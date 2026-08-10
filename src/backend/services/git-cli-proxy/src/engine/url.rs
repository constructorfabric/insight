/// A clone URL that has been proved safe to hand to `git` as an argument.
///
/// INVARIANT: only `http://` and `https://` origins are constructible outside
/// tests. `git` treats the URL as a transport selector, and `ext::` runs an
/// arbitrary shell command; a raw path reaches the local filesystem. Neither is
/// reachable through the API, so the boundary is parsed once, here, and the
/// rest of the service carries the proof rather than the string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CloneUrl(String);

/// Whether `file://` origins may be constructed. Off in every shipped
/// configuration; the hermetic test suite clones from `file://` fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneUrlPolicy {
    pub allow_file: bool,
}

impl CloneUrlPolicy {
    #[must_use]
    pub const fn http_only() -> Self {
        Self { allow_file: false }
    }

    #[must_use]
    pub const fn with_file_origins() -> Self {
        Self { allow_file: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloneUrlError {
    #[error("`repo` is empty")]
    Empty,
    #[error("`repo` starts with `-` and would be read as a git option")]
    LeadingDash,
    #[error("`repo` contains whitespace or a control character")]
    ControlCharacter,
    #[error("`repo` must be an http(s) URL, got scheme `{0}`")]
    UnsupportedScheme(String),
    #[error("`repo` has no host")]
    MissingAuthority,
    #[error("`repo` must not embed credentials")]
    Userinfo,
}

const HTTP: &str = "http://";
const HTTPS: &str = "https://";
const FILE: &str = "file://";

impl CloneUrl {
    /// # Errors
    ///
    /// [`CloneUrlError`] when the value is not an origin this service is
    /// willing to clone from.
    pub fn parse(raw: &str, policy: CloneUrlPolicy) -> Result<Self, CloneUrlError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(CloneUrlError::Empty);
        }
        if trimmed.starts_with('-') {
            return Err(CloneUrlError::LeadingDash);
        }
        if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(CloneUrlError::ControlCharacter);
        }

        let lower = trimmed.to_ascii_lowercase();
        let authority = if lower.starts_with(HTTP) {
            &trimmed[HTTP.len()..]
        } else if lower.starts_with(HTTPS) {
            &trimmed[HTTPS.len()..]
        } else if policy.allow_file && lower.starts_with(FILE) {
            // A file origin is a path, not an authority: the emptiness check
            // below is the only one that applies.
            if trimmed[FILE.len()..].is_empty() {
                return Err(CloneUrlError::MissingAuthority);
            }
            return Ok(Self(trimmed.to_owned()));
        } else {
            return Err(CloneUrlError::UnsupportedScheme(scheme_of(trimmed)));
        };

        let host = authority.split(['/', '?', '#']).next().unwrap_or("");
        if host.is_empty() {
            return Err(CloneUrlError::MissingAuthority);
        }
        // Credentials travel as `http.extraheader` in the child env; an
        // in-URL userinfo would silently override that and reach git's stderr.
        if host.contains('@') {
            return Err(CloneUrlError::Userinfo);
        }

        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn scheme_of(raw: &str) -> String {
    match raw.split_once("://") {
        Some((scheme, _)) => scheme.to_ascii_lowercase(),
        // `ext::`, `git@host:path` and bare paths have no `://` at all; report
        // enough to be actionable without echoing the whole value back.
        None => raw.split_once(':').map_or_else(
            || "none".to_owned(),
            |(scheme, _)| scheme.to_ascii_lowercase(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https_case_insensitively() {
        let cases = [
            "http://example.com/a.git",
            "https://example.com/a.git",
            "HTTPS://Example.COM/a.git",
            "https://example.com:8443/group/sub/a.git",
        ];
        for raw in cases {
            let parsed = CloneUrl::parse(raw, CloneUrlPolicy::http_only());
            assert!(parsed.is_ok(), "should accept: {raw:?} ({parsed:?})");
        }
    }

    #[test]
    fn preserves_the_url_verbatim_so_cache_entries_keep_their_names() {
        let raw = "https://example.com/a.git";
        let Ok(parsed) = CloneUrl::parse(raw, CloneUrlPolicy::http_only()) else {
            panic!("should accept")
        };
        assert_eq!(parsed.as_str(), raw);
    }

    #[test]
    fn rejects_every_non_http_transport() {
        let cases = [
            "ext::sh -c id",
            "file:///tmp/x",
            "git://example.com/a.git",
            "ssh://example.com/a.git",
            "git@example.com:group/a.git",
            "/etc/passwd",
            "./relative",
            "example.com/a.git",
        ];
        for raw in cases {
            let parsed = CloneUrl::parse(raw, CloneUrlPolicy::http_only());
            assert!(parsed.is_err(), "should reject: {raw:?}");
        }
    }

    #[test]
    fn rejects_leading_dash_and_control_characters() {
        let cases = [
            "-u/tmp/payload",
            "--upload-pack=id",
            "https://example.com/a.git\nhttps://evil/b.git",
            "https://example.com/a b.git",
        ];
        for raw in cases {
            let parsed = CloneUrl::parse(raw, CloneUrlPolicy::http_only());
            assert!(parsed.is_err(), "should reject: {raw:?}");
        }
    }

    #[test]
    fn rejects_userinfo() {
        let cases = [
            "https://user:token@example.com/a.git",
            "http://user@example.com/a.git",
        ];
        for raw in cases {
            assert_eq!(
                CloneUrl::parse(raw, CloneUrlPolicy::http_only()),
                Err(CloneUrlError::Userinfo),
                "should reject: {raw:?}"
            );
        }
    }

    #[test]
    fn rejects_a_missing_host() {
        for raw in ["https://", "https:///a.git", "https://?x=1"] {
            assert_eq!(
                CloneUrl::parse(raw, CloneUrlPolicy::http_only()),
                Err(CloneUrlError::MissingAuthority),
                "should reject: {raw:?}"
            );
        }
    }

    #[test]
    fn file_url_allowed_only_under_the_escape_hatch() {
        let raw = "file:///tmp/fixture.git";
        assert_eq!(
            CloneUrl::parse(raw, CloneUrlPolicy::http_only()),
            Err(CloneUrlError::UnsupportedScheme("file".to_owned()))
        );
        assert!(CloneUrl::parse(raw, CloneUrlPolicy::with_file_origins()).is_ok());
    }

    #[test]
    fn the_escape_hatch_does_not_widen_anything_else() {
        for raw in ["ext::sh -c id", "ssh://example.com/a.git", "/etc/passwd"] {
            let parsed = CloneUrl::parse(raw, CloneUrlPolicy::with_file_origins());
            assert!(parsed.is_err(), "should still reject: {raw:?}");
        }
    }

    #[test]
    fn empty_is_its_own_error() {
        for raw in ["", "   "] {
            assert_eq!(
                CloneUrl::parse(raw, CloneUrlPolicy::http_only()),
                Err(CloneUrlError::Empty)
            );
        }
    }
}
