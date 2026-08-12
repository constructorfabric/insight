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
pub struct CloneUrlPolicy<'a> {
    pub allow_file: bool,
    /// Exceptions to the built-in refusals, and nothing more. Adding one host
    /// never removes another: tenants add sources without an operator, so a
    /// list that also had to enumerate every PUBLIC vendor would turn every
    /// new source into a redeploy.
    pub allowed_hosts: &'a [String],
}

impl CloneUrlPolicy<'_> {
    #[must_use]
    pub const fn http_only() -> Self {
        Self {
            allow_file: false,
            allowed_hosts: &[],
        }
    }

    #[must_use]
    pub const fn with_file_origins() -> Self {
        Self {
            allow_file: true,
            allowed_hosts: &[],
        }
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
    #[error("`repo` host `{0}` is not an origin this service may reach")]
    ForbiddenHost(String),
}

/// Suffixes a pod's own resolver puts on names that only exist inside the
/// cluster or the local link.
const INTERNAL_SUFFIXES: [&str; 5] = [".local", ".internal", ".svc", ".localhost", ".home.arpa"];

const HTTP: &str = "http://";
const HTTPS: &str = "https://";
const FILE: &str = "file://";

impl CloneUrl {
    /// # Errors
    ///
    /// [`CloneUrlError`] when the value is not an origin this service is
    /// willing to clone from.
    pub fn parse(raw: &str, policy: CloneUrlPolicy<'_>) -> Result<Self, CloneUrlError> {
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

        let name = hostname_of(host);
        if !permitted_host(name, policy.allowed_hosts) {
            return Err(CloneUrlError::ForbiddenHost(name.to_owned()));
        }

        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The host out of an authority, without its port or IPv6 brackets.
fn hostname_of(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    authority.split(':').next().unwrap_or(authority)
}

/// Whether this service may open a connection to `host`.
///
/// `repo` is set by whoever configures a source — a tenant, not an operator —
/// so it is attacker-influenceable by design. The bearer token authenticates a
/// caller CLASS rather than a tenant, and the `NetworkPolicy` restricts
/// ingress only, so without this the proxy is a general-purpose probe of
/// anything its pod can reach. The case that matters is the cloud metadata
/// endpoint on `169.254.169.254`: on a node with an instance profile, one
/// source configuration would otherwise become credential theft.
///
/// What is refused is a fixed set of ranges that only ever name something
/// inside the deployment. `allowed_hosts` adds exceptions for a self-hosted
/// vendor sitting in one of them; it never subtracts, so every public vendor
/// works with no configuration at all and a tenant adding a source never waits
/// on a redeploy.
///
/// Names are judged, not resolved addresses — resolve-then-connect is a race
/// git would lose anyway.
fn permitted_host(host: &str, allowed: &[String]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if allowed
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(&host))
    {
        return true;
    }

    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return is_public_address(address);
    }

    // A name with no dot is a cluster-internal service; the reserved suffixes
    // are the ones Kubernetes and mDNS put on the pod's own resolver path.
    if !host.contains('.') {
        return false;
    }
    !INTERNAL_SUFFIXES
        .iter()
        .any(|suffix| host.ends_with(suffix))
}

/// Whether a literal address is outside every range that only ever names
/// something inside the deployment.
fn is_public_address(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                // 100.64.0.0/10, the carrier-grade NAT range kubelets use.
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1])))
        }
        std::net::IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // fe80::/10 link-local and fc00::/7 unique-local.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || (v6.segments()[0] & 0xfe00) == 0xfc00)
        }
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
    fn refuses_every_destination_inside_the_deployment() {
        // The bearer token authenticates a caller CLASS, not a tenant, and the
        // NetworkPolicy restricts ingress only — so an unrestricted host turns
        // this service into a probe of anything its pod can reach.
        let cases = [
            ("cloud metadata", "http://169.254.169.254/latest/meta-data/"),
            ("loopback v4", "http://127.0.0.1:8080/a.git"),
            ("loopback name", "https://localhost/a.git"),
            ("loopback v6", "http://[::1]:8080/a.git"),
            ("private 10/8", "https://10.1.2.3/a.git"),
            ("private 172.16/12", "https://172.20.0.5/a.git"),
            ("private 192.168/16", "https://192.168.1.10/a.git"),
            ("carrier-grade NAT", "https://100.64.0.1/a.git"),
            ("unique-local v6", "http://[fc00::1]/a.git"),
            ("link-local v6", "http://[fe80::1]/a.git"),
            ("unspecified", "http://0.0.0.0/a.git"),
            ("cluster service", "http://clickhouse/a.git"),
            (
                "cluster DNS",
                "http://svc.namespace.svc.cluster.local/a.git",
            ),
            ("mDNS", "http://printer.local/a.git"),
        ];
        for (name, raw) in cases {
            match CloneUrl::parse(raw, CloneUrlPolicy::http_only()) {
                Err(CloneUrlError::ForbiddenHost(_)) => {}
                other => panic!("{name}: {raw:?} must be refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_allowlist_only_ever_widens() {
        let allowed = vec!["gitlab.internal".to_owned(), "10.1.2.3".to_owned()];
        let policy = CloneUrlPolicy {
            allow_file: false,
            allowed_hosts: &allowed,
        };

        // Named exceptions become reachable, by name or by address.
        for raw in [
            "https://gitlab.internal/g/a.git",
            "https://10.1.2.3/g/a.git",
        ] {
            assert!(
                CloneUrl::parse(raw, policy).is_ok(),
                "an allowlisted host must be reachable: {raw}"
            );
        }

        // INVARIANT: naming an exception must not make every other vendor a
        // deployment change. Tenants add sources; operators own this list.
        for raw in [
            "https://github.com/o/a.git",
            "https://gitlab.com/g/a.git",
            "https://bitbucket.org/w/a.git",
        ] {
            assert!(
                CloneUrl::parse(raw, policy).is_ok(),
                "a public vendor must need no configuration: {raw}"
            );
        }

        // What the list does NOT do is widen anything it did not name.
        match CloneUrl::parse("http://169.254.169.254/latest.git", policy) {
            Err(CloneUrlError::ForbiddenHost(_)) => {}
            other => panic!("metadata must stay refused, got {other:?}"),
        }
    }

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
