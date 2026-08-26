use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};

/// Git credentials for one invocation. They exist only in the child process
/// environment — never in argv, never on disk, never in the stored remote URL
/// (that is what makes token rotation free).
#[derive(Clone)]
pub struct GitCredentials {
    pub username: String,
    pub token: String,
}

impl std::fmt::Debug for GitCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitCredentials")
            .field("username", &self.username)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl GitCredentials {
    /// `Authorization: Basic …` header value, injected via the env-config
    /// mechanism (`GIT_CONFIG_KEY_n`) — invisible in `ps`, absent from disk.
    fn basic_header(&self) -> String {
        let pair = format!("{}:{}", self.username, self.token);
        format!("Authorization: Basic {}", BASE64.encode(pair))
    }

    /// Stable one-way fingerprint of these credentials, stored alongside the
    /// cache entry so a warm read can require the caller to present the
    /// credentials that proved origin access. The token itself is never
    /// written to disk; recovering it from the digest is infeasible for
    /// high-entropy tokens.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.username.as_bytes());
        hasher.update([0u8]);
        hasher.update(self.token.as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("origin rejected the supplied git credentials")]
    AuthRejected,
    #[error("repository not found at origin")]
    NotFound,
    #[error("origin declines to serve the repository")]
    OriginUnavailable,
    #[error("origin refuses to serve explicitly requested objects")]
    PromisorRefused,
    #[error("the cache cannot take more disk right now")]
    AdmissionRejected,
    #[error("the entry is over its cap until purged blobs are reclaimed")]
    TransientlyOverCap,
    #[error("origin is throttling this client")]
    Throttled,
    #[error("git timed out after {0:?}")]
    TimedOut(Duration),
    #[error("git failed: {0}")]
    Failed(String),
    #[error("failed to spawn git: {0}")]
    Io(#[from] std::io::Error),
    #[error("repository exceeds the per-repository size cap of {cap_bytes} bytes")]
    TooLarge { cap_bytes: u64 },
}

/// How long each class of git invocation may take.
///
/// One budget for all of them cannot work. A read holds the entry's READ
/// lock, so a stalled one blocks fetch and eviction for its whole budget
/// while every other stream 429-loops past the connector's own ceiling and
/// fails the sync. A clone genuinely needs half an hour.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// Local plumbing: `log`, `for-each-ref`, `rev-list`, `patch-id`. No
    /// network, so anything approaching this is a stall — most likely a lazy
    /// promisor fetch behind what looks like a local read.
    pub read: Duration,
    /// The per-page blob prefetch. Network, but bounded by one page.
    pub prefetch: Duration,
    /// Clone, fetch, repack, promotion: whole-repository work.
    pub heavy: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            read: READ_TIMEOUT,
            prefetch: PREFETCH_TIMEOUT,
            heavy: HEAVY_OP_TIMEOUT,
        }
    }
}

/// Spawns git subprocesses with a hermetic environment: explicit `--git-dir`
/// or working dir (process cwd is never relied on), no user/system gitconfig,
/// no interactive prompts, credentials via env only.
#[derive(Debug, Clone)]
pub struct GitRunner {
    timeouts: Timeouts,
    /// How often a capped run re-measures. Trades overshoot against the cost
    /// of walking a tree that is actively being written.
    cap_poll: Duration,
    /// PEM bundle for origins whose TLS chain is not in the system store
    /// (a self-hosted vendor behind a private CA). Empty = system store only.
    ca_cert_path: Option<String>,
}

const STDERR_TAIL_BYTES: usize = 4096;
/// Delta-search threads per git invocation. Deliberately small and fixed:
/// git's default is one per host CPU, which ignores the container's own CPU
/// limit, so the default multiplies the per-thread memory budget below by a
/// number this service does not control.
const PACK_THREADS: usize = 2;
/// Per-thread delta window budget. Peak pack memory is roughly
/// `PACK_THREADS * PACK_WINDOW_MEMORY_MB + PACK_DELTA_CACHE_MB` plus object
/// bookkeeping, which keeps the whole operation inside a low-single-digit-GiB
/// pod limit on any repository size.
const PACK_WINDOW_MEMORY_MB: usize = 256;
/// Delta cache budget, shared across threads.
const PACK_DELTA_CACHE_MB: usize = 256;
const READ_TIMEOUT: Duration = Duration::from_mins(5);
const PREFETCH_TIMEOUT: Duration = Duration::from_mins(10);
const HEAVY_OP_TIMEOUT: Duration = Duration::from_mins(30);
/// How often a capped run re-measures the tree it is filling. The cap can be
/// overshot by one interval's worth of download; the post-hoc check is what
/// catches that remainder.
const CAP_POLL_INTERVAL: Duration = Duration::from_secs(5);

impl Default for GitRunner {
    fn default() -> Self {
        Self {
            timeouts: Timeouts::default(),
            cap_poll: CAP_POLL_INTERVAL,
            ca_cert_path: None,
        }
    }
}

impl GitRunner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    #[cfg(test)]
    fn with_cap_poll(mut self, interval: Duration) -> Self {
        self.cap_poll = interval;
        self
    }

    #[must_use]
    pub fn with_ca_cert(mut self, path: Option<String>) -> Self {
        self.ca_cert_path = path.filter(|value| !value.is_empty());
        self
    }

    /// Run local git plumbing against `git_dir` under the read budget.
    ///
    /// # Errors
    ///
    /// [`GitError`] on spawn failure, timeout, or non-zero exit.
    pub async fn run(
        &self,
        git_dir: Option<&Path>,
        args: &[&str],
        creds: Option<&GitCredentials>,
    ) -> Result<Output, GitError> {
        self.run_within(self.timeouts.read, git_dir, args, creds)
            .await
    }

    /// Run the per-page blob prefetch under its own budget.
    ///
    /// # Errors
    ///
    /// [`GitError`] on spawn failure, timeout, or non-zero exit.
    pub async fn run_prefetch(
        &self,
        git_dir: &Path,
        args: &[&str],
        creds: &GitCredentials,
    ) -> Result<Output, GitError> {
        self.run_within(self.timeouts.prefetch, Some(git_dir), args, Some(creds))
            .await
    }

    /// Run whole-repository work — repack, promotion — under the heavy budget.
    ///
    /// # Errors
    ///
    /// [`GitError`] on spawn failure, timeout, or non-zero exit.
    pub async fn run_heavy(
        &self,
        git_dir: Option<&Path>,
        args: &[&str],
        creds: Option<&GitCredentials>,
    ) -> Result<Output, GitError> {
        self.run_within(self.timeouts.heavy, git_dir, args, creds)
            .await
    }

    async fn run_within(
        &self,
        budget: Duration,
        git_dir: Option<&Path>,
        args: &[&str],
        creds: Option<&GitCredentials>,
    ) -> Result<Output, GitError> {
        let mut command = self.base_command(creds);
        if let Some(dir) = git_dir {
            command.arg("--git-dir").arg(dir);
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let waited = tokio::time::timeout(budget, command.output()).await;
        let output = match waited {
            Ok(result) => result?,
            Err(_elapsed) => return Err(GitError::TimedOut(budget)),
        };

        if output.status.success() {
            return Ok(output);
        }
        Err(classify_failure(&output))
    }

    /// Run `git <args>` while watching `watch` grow, killing the child as soon
    /// as it passes `cap_bytes`.
    ///
    /// Measuring only after the command returns is too late: the disk the cap
    /// exists to protect has already been spent, and a repository an order of
    /// magnitude over the cap can fill the volume before anything objects.
    /// The caller keeps its post-hoc check for the remainder a poll interval
    /// can miss.
    ///
    /// # Errors
    ///
    /// [`GitError`] on spawn failure, timeout, non-zero exit, or
    /// [`GitError::TooLarge`] when `watch` passes the cap mid-run.
    pub async fn run_capped(
        &self,
        git_dir: Option<&Path>,
        args: &[&str],
        creds: Option<&GitCredentials>,
        watch: &Path,
        cap_bytes: u64,
    ) -> Result<Output, GitError> {
        self.run_capped_within(self.timeouts.heavy, git_dir, args, creds, watch, cap_bytes)
            .await
    }

    /// The per-page blob prefetch, under its own budget and the same watcher.
    ///
    /// # Errors
    ///
    /// [`GitError`] on spawn failure, timeout, non-zero exit, or
    /// [`GitError::TooLarge`] when `watch` passes the cap mid-run.
    pub async fn run_prefetch_capped(
        &self,
        git_dir: &Path,
        args: &[&str],
        creds: &GitCredentials,
        cap_bytes: u64,
    ) -> Result<Output, GitError> {
        self.run_capped_within(
            self.timeouts.prefetch,
            Some(git_dir),
            args,
            Some(creds),
            git_dir,
            cap_bytes,
        )
        .await
    }

    async fn run_capped_within(
        &self,
        budget: Duration,
        git_dir: Option<&Path>,
        args: &[&str],
        creds: Option<&GitCredentials>,
        watch: &Path,
        cap_bytes: u64,
    ) -> Result<Output, GitError> {
        let mut command = self.base_command(creds);
        if let Some(dir) = git_dir {
            command.arg("--git-dir").arg(dir);
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // INVARIANT: this is what enforces the cap. Returning early drops
            // the wait future, which owns the child, which kills it.
            .kill_on_drop(true);

        let child = command.spawn()?;
        let watch = watch.to_path_buf();
        let cap_poll = self.cap_poll;

        let capped = async move {
            let wait = child.wait_with_output();
            tokio::pin!(wait);

            let mut poll = tokio::time::interval(cap_poll);
            poll.tick().await;

            loop {
                tokio::select! {
                    finished = &mut wait => return finished.map_err(GitError::Io),
                    _ = poll.tick() => {
                        let measured = {
                            let watch = watch.clone();
                            tokio::task::spawn_blocking(move || super::disk::dir_size(&watch))
                                .await
                                .unwrap_or(0)
                        };
                        if measured > cap_bytes {
                            return Err(GitError::TooLarge { cap_bytes });
                        }
                    }
                }
            }
        };

        let output = match tokio::time::timeout(budget, capped).await {
            Ok(result) => result?,
            Err(_elapsed) => return Err(GitError::TimedOut(budget)),
        };

        if output.status.success() {
            return Ok(output);
        }
        Err(classify_failure(&output))
    }

    /// Run `git <producer> | git <consumer>` inside `git_dir`, returning the
    /// consumer's stdout. Used for `log --patch | patch-id --stable`, the
    /// canonical batch form — piping keeps whole-history diffs out of memory.
    ///
    /// # Errors
    ///
    /// [`GitError`] on spawn failure, timeout, or a non-zero exit from either
    /// side of the pipe.
    pub async fn run_piped(
        &self,
        git_dir: &Path,
        producer: &[&str],
        consumer: &[&str],
        creds: &GitCredentials,
    ) -> Result<Vec<u8>, GitError> {
        let mut left = self.base_command(Some(creds));
        left.arg("--git-dir")
            .arg(git_dir)
            .args(producer)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut left_child = left.spawn()?;

        let left_stdout = left_child
            .stdout
            .take()
            .ok_or_else(|| GitError::Failed("pipe producer exposed no stdout".to_owned()))?;

        let mut right = self.base_command(Some(creds));
        right
            .arg("--git-dir")
            .arg(git_dir)
            .args(consumer)
            .stdin(Stdio::from(left_stdout.into_owned_fd()?))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let right_child = right.spawn()?;

        // INVARIANT: both sides are drained concurrently. Awaiting the consumer
        // to completion first would deadlock the producer once it writes past
        // the stderr pipe buffer, and the call would surface as a timeout.
        let joined = async {
            tokio::try_join!(
                right_child.wait_with_output(),
                left_child.wait_with_output()
            )
        };

        let budget = self.timeouts.read;
        let (right_output, left_output) = match tokio::time::timeout(budget, joined).await {
            Ok(result) => result?,
            Err(_elapsed) => return Err(GitError::TimedOut(budget)),
        };

        if !left_output.status.success() {
            return Err(classify_failure(&left_output));
        }
        if !right_output.status.success() {
            return Err(classify_failure(&right_output));
        }
        Ok(right_output.stdout)
    }

    fn base_command(&self, creds: Option<&GitCredentials>) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("git");
        command.env_clear();
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C");

        // Config travels as env pairs, never argv: the credential header must
        // stay invisible in `ps`, and the CA path rides along the same way.
        for (index, (key, value)) in self.config_pairs(creds).into_iter().enumerate() {
            command.env(format!("GIT_CONFIG_KEY_{index}"), key);
            command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
        let count = self.config_pairs(creds).len();
        if count > 0 {
            command.env("GIT_CONFIG_COUNT", count.to_string());
        }
        command
    }

    fn config_pairs(&self, creds: Option<&GitCredentials>) -> Vec<(&'static str, String)> {
        let mut pairs: Vec<(&'static str, String)> = Vec::new();
        if let Some(creds) = creds {
            pairs.push(("http.extraheader", creds.basic_header()));
        }
        if let Some(path) = self.ca_cert_path.as_ref() {
            pairs.push(("http.sslCAInfo", path.clone()));
        }
        // Since git 2.29 every fetch ends with `maintenance run --auto`; on
        // the per-page prefetch that means a whole-repository gc forking mid
        // page-serve once enough packs accumulate. Consolidation is owned by
        // the store's own repack, on its schedule.
        pairs.push(("maintenance.auto", "false".to_owned()));
        pairs.push(("gc.auto", "0".to_owned()));
        // Delta search is what makes a repack of a large repository expensive,
        // and its budget is PER THREAD — with `pack.threads` unset git uses
        // one per host CPU, which a container limit does not shrink. Left
        // unbounded, a single `git` process on a large-object repository can
        // outgrow any memory limit the pod is given and take the service with
        // it. These caps trade pack density and repack time, neither of which
        // a blob-purge cache depends on, for a bounded peak.
        pairs.push(("pack.threads", PACK_THREADS.to_string()));
        pairs.push(("pack.windowMemory", format!("{PACK_WINDOW_MEMORY_MB}m")));
        pairs.push(("pack.deltaCacheSize", format!("{PACK_DELTA_CACHE_MB}m")));
        pairs
    }
}

/// Map a failed git invocation to a typed error by stderr fingerprints. The
/// stored remote URL is credential-free, so quoting stderr leaks no secrets.
fn classify_failure(output: &Output) -> GitError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lower = stderr.to_lowercase();

    // Before `auth`: a vendor signals throttling with 403 as readily as with
    // 429 (GitHub's secondary rate limits do exactly that), and calling that a
    // credential failure fails the whole sync as a config error instead of
    // backing off and retrying.
    let throttled = [
        "rate limit",
        "too many requests",
        "http 429",
        "returned error: 429",
    ];
    if throttled.iter().any(|m| lower.contains(m)) {
        return GitError::Throttled;
    }

    // Before `auth`: Bitbucket announces a suspended or disabled repository
    // with this remote line plus a bare 403; reading that as a credential
    // failure would fail the whole sync over one repository no retry can fix.
    if lower.contains("this repository is currently not available") {
        return GitError::OriginUnavailable;
    }

    let auth = [
        "authentication failed",
        "http 401",
        "401 unauthorized",
        "403 forbidden",
        "could not read username",
    ];
    if auth.iter().any(|m| lower.contains(m)) {
        return GitError::AuthRejected;
    }

    // Before `missing`: an origin that refuses explicit object requests also
    // prints "could not read from remote repository", which would otherwise
    // classify a healable repository as permanently absent.
    let promisor = ["did not send all necessary objects", "not our ref"];
    if promisor.iter().any(|m| lower.contains(m)) {
        return GitError::PromisorRefused;
    }

    // `could not read from remote repository` is deliberately NOT here. Git
    // prints it after any transport failure — a hang-up, a proxy fault, a
    // refused connection — and calling those `404` tells the connector the
    // parent record is stale when the repository is fine. A genuinely absent
    // repository over http(s) always carries one of the specific lines below.
    let missing = [
        "repository not found",
        "http 404",
        "returned error: 404",
        "does not appear to be a git repository",
    ];
    if missing.iter().any(|m| lower.contains(m)) {
        return GitError::NotFound;
    }

    let tail_start = stderr.len().saturating_sub(STDERR_TAIL_BYTES);
    let mut cut = tail_start;
    while cut < stderr.len() && !stderr.is_char_boundary(cut) {
        cut += 1;
    }
    GitError::Failed(stderr[cut..].trim().to_owned())
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    use super::*;

    fn failed_output(stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    type Check = fn(&GitError) -> bool;

    #[test]
    fn classifies_stderr_fingerprints() {
        let cases: Vec<(&str, Check)> = vec![
            ("fatal: Authentication failed for 'https://x/'", |e| {
                matches!(e, GitError::AuthRejected)
            }),
            ("The requested URL returned error: 403 Forbidden", |e| {
                matches!(e, GitError::AuthRejected)
            }),
            (
                "fatal: could not read Username for 'https://x': terminal prompts disabled",
                |e| matches!(e, GitError::AuthRejected),
            ),
            ("remote: Repository not found.", |e| {
                matches!(e, GitError::NotFound)
            }),
            // An origin refusing an explicit promisor want. Both shapes are
            // real: the first is what a pooled GitLab repository returns, the
            // second what the git transport returns directly.
            (
                "fatal: remote error: upload-pack: not our ref f719efd4",
                |e| matches!(e, GitError::PromisorRefused),
            ),
            (
                "error: https://example.com/a.git did not send all necessary objects\n\
                 fatal: could not read from remote repository",
                |e| matches!(e, GitError::PromisorRefused),
            ),
            ("fatal: 'x' does not appear to be a git repository", |e| {
                matches!(e, GitError::NotFound)
            }),
            ("fatal: something completely different", |e| {
                matches!(e, GitError::Failed(_))
            }),
            // Throttling wins over the 403 that carries it: calling this a
            // credential failure fails the sync as a config error instead of
            // backing off.
            (
                "remote: You have exceeded a secondary rate limit\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::Throttled),
            ),
            (
                "fatal: unable to access 'https://x/': The requested URL returned error: 429",
                |e| matches!(e, GitError::Throttled),
            ),
            // A suspended or disabled repository: the origin refuses it with
            // a bare 403, which is neither a credential failure nor a rate
            // limit.
            (
                "remote: This repository is currently not available.\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
            // A transport fault is not a missing repository: answering `404`
            // tells the connector its parent record is stale.
            (
                "fatal: unable to access 'https://x/': Failed to connect to x port 443: Connection refused\n\
                 fatal: could not read from remote repository",
                |e| matches!(e, GitError::Failed(_)),
            ),
        ];
        for (stderr, check) in cases {
            let err = classify_failure(&failed_output(stderr));
            assert!(check(&err), "stderr {stderr:?} classified as {err:?}");
        }
    }

    #[test]
    fn failed_keeps_only_the_stderr_tail() {
        let noise = "x".repeat(10_000);
        let err = classify_failure(&failed_output(&noise));
        match err {
            GitError::Failed(tail) => assert!(tail.len() <= STDERR_TAIL_BYTES),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn basic_header_encodes_user_and_token() {
        let creds = GitCredentials {
            username: "oauth2".to_owned(),
            token: "tok".to_owned(),
        };
        assert_eq!(
            creds.basic_header(),
            format!("Authorization: Basic {}", BASE64.encode("oauth2:tok"))
        );
    }

    #[test]
    fn debug_never_prints_the_token() {
        let creds = GitCredentials {
            username: "u".to_owned(),
            token: "sup3r-secret".to_owned(),
        };
        let rendered = format!("{creds:?}");
        assert!(!rendered.contains("sup3r-secret"), "leaked: {rendered}");
    }

    #[test]
    fn config_pairs_carry_credentials_and_ca_path() {
        let creds = GitCredentials {
            username: "oauth2".to_owned(),
            token: "tok".to_owned(),
        };
        let plain = GitRunner::new();
        assert!(
            !plain
                .config_pairs(None)
                .iter()
                .any(|(k, _)| *k == "http.extraheader" || *k == "http.sslCAInfo"),
            "no creds and no CA means neither pair is present"
        );

        let with_creds = plain.config_pairs(Some(&creds));
        assert!(
            with_creds.iter().any(|(k, _)| *k == "http.extraheader"),
            "got: {with_creds:?}"
        );

        let with_ca = GitRunner::new().with_ca_cert(Some("/certs/ca.pem".to_owned()));
        let both = with_ca.config_pairs(Some(&creds));
        assert!(
            both.iter()
                .any(|(k, v)| *k == "http.sslCAInfo" && v == "/certs/ca.pem"),
            "on-prem origins need the CA pair too, got: {both:?}"
        );
    }

    #[test]
    fn empty_ca_path_is_treated_as_unset() {
        let runner = GitRunner::new().with_ca_cert(Some(String::new()));
        assert!(
            !runner
                .config_pairs(None)
                .iter()
                .any(|(k, _)| *k == "http.sslCAInfo"),
            "an empty CA path must not become a git config value"
        );
    }

    #[test]
    fn every_invocation_disables_gits_own_maintenance() {
        // Auto-maintenance forks a whole-repository gc at the end of a fetch;
        // on the per-page prefetch that is a gc storm mid page-serve.
        // Consolidation belongs to the store's repack, on its schedule.
        let pairs = GitRunner::new().config_pairs(None);
        for (key, expected) in [("maintenance.auto", "false"), ("gc.auto", "0")] {
            assert!(
                pairs.iter().any(|(k, v)| *k == key && v == expected),
                "{key} must be pinned, got: {pairs:?}"
            );
        }
    }

    #[test]
    fn every_invocation_bounds_pack_memory() {
        // An unbounded delta search on a large-object repository grows past
        // any pod memory limit and the kernel kills the whole service, not
        // just the git process.
        let pairs = GitRunner::new().config_pairs(None);
        for key in ["pack.threads", "pack.windowMemory", "pack.deltaCacheSize"] {
            assert!(
                pairs.iter().any(|(k, v)| *k == key && !v.is_empty()),
                "{key} must be pinned, got: {pairs:?}"
            );
        }
    }

    #[test]
    fn credentials_never_print_their_token() {
        // §3.7: nothing in this service logs a request header, so the tokens
        // it does hold must not leak through the one thing that does reach a
        // log line — a `Debug` render inside a `tracing` event.
        let creds = GitCredentials {
            username: "x-token-auth".to_owned(),
            token: "s3cret-value".to_owned(),
        };
        let rendered = format!("{creds:?}");
        assert!(
            !rendered.contains("s3cret-value"),
            "Debug leaked the token: {rendered}"
        );
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(
            !format!("{:?}", creds.basic_header()).contains("s3cret-value"),
            "the header value is base64, but the token must not survive verbatim"
        );
    }

    #[tokio::test]
    async fn run_reports_version() {
        let runner = GitRunner::new();
        let output = match runner.run(None, &["--version"], None).await {
            Ok(o) => o,
            Err(e) => panic!("git --version failed: {e}"),
        };
        assert!(String::from_utf8_lossy(&output.stdout).contains("git version"));
    }

    #[tokio::test]
    async fn piped_producer_failure_carries_its_stderr() {
        // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; names carry pid/thread/counter and hold no secrets
        let dir = std::env::temp_dir().join(format!(
            "git-cli-proxy-piped-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            panic!("create temp dir: {e}");
        }

        let runner = GitRunner::new();
        let init = tokio::process::Command::new("git")
            .args(["init", "--bare", "-q"])
            .arg(&dir)
            .output()
            .await;
        if let Err(e) = init {
            panic!("git init: {e}");
        }

        let creds = GitCredentials {
            username: "u".to_owned(),
            token: "t".to_owned(),
        };
        let result = runner
            .run_piped(
                &dir,
                &["log", "--no-walk", "refs/heads/no-such-ref"],
                &["patch-id", "--stable"],
                &creds,
            )
            .await;

        // The producer's stderr is only reachable if it was drained; an
        // undrained pipe surfaces as TimedOut with no diagnosis at all.
        match result {
            Err(GitError::Failed(message)) => assert!(
                !message.is_empty(),
                "producer stderr must survive to the caller"
            ),
            Err(GitError::NotFound) => {}
            other => panic!("expected a classified producer failure, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An origin holding one incompressible blob, so a clone of it takes long
    /// enough to be interrupted and large enough to breach a small cap.
    fn heavy_origin(tag: &str) -> std::path::PathBuf {
        // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; names carry pid/thread/counter and hold no secrets
        let root = std::env::temp_dir().join(format!(
            "git-cli-proxy-cap-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let origin = root.join("origin");
        if let Err(e) = std::fs::create_dir_all(&origin) {
            panic!("create origin: {e}");
        }
        let script = "git init -q -b main . && \
             dd if=/dev/urandom of=big.bin bs=1024 count=16384 status=none && \
             git add big.bin && git commit -qm big";
        let output = std::process::Command::new("sh")
            .arg("-ec")
            .arg(script)
            .current_dir(&origin)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output();
        match output {
            Ok(o) if o.status.success() => {}
            Ok(o) => panic!("origin setup: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => panic!("origin setup: {e}"),
        }
        root
    }

    #[tokio::test]
    async fn a_run_past_the_cap_is_killed_mid_flight() {
        let root = heavy_origin("kill");
        let target = root.join("clone.git");
        let url = format!("file://{}", root.join("origin").display());

        // The watched tree is over the cap from the first poll, which is the
        // shape a fetch presents: the entry already holds a clone. Timing the
        // breach against how fast git happens to write would make this a race,
        // not a test.
        let runner = GitRunner::new().with_cap_poll(Duration::from_millis(1));
        let result = runner
            .run_capped(
                None,
                &[
                    "clone",
                    "--bare",
                    "--quiet",
                    &url,
                    &target.to_string_lossy(),
                ],
                None,
                &root,
                1024 * 1024,
            )
            .await;

        // `run_capped` has no post-hoc check: only the watcher can produce
        // this, and only by killing the child.
        match result {
            Err(GitError::TooLarge { cap_bytes }) => assert_eq!(cap_bytes, 1024 * 1024),
            other => panic!("a run past the cap must be killed, got {other:?}"),
        }
        assert!(
            !target.join("packed-refs").is_file(),
            "the clone must have died before it finished"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_run_under_the_cap_is_left_alone() {
        let root = heavy_origin("allow");
        let target = root.join("clone.git");
        let url = format!("file://{}", root.join("origin").display());

        let runner = GitRunner::new().with_cap_poll(Duration::from_millis(1));
        if let Err(e) = runner
            .run_capped(
                None,
                &[
                    "clone",
                    "--bare",
                    "--quiet",
                    &url,
                    &target.to_string_lossy(),
                ],
                None,
                &root,
                1024 * 1024 * 1024,
            )
            .await
        {
            panic!("a run inside the cap must finish: {e}");
        }
        assert!(
            target.join("HEAD").is_file(),
            "the watcher must not disturb a clone under the cap"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_read_gives_up_long_before_whole_repository_work() {
        let defaults = Timeouts::default();
        assert!(
            defaults.read < defaults.prefetch && defaults.prefetch < defaults.heavy,
            "a read holds the entry's read lock; it must give up first: {defaults:?}"
        );

        let root = heavy_origin("budgets");
        let url = format!("file://{}", root.join("origin").display());
        let runner = GitRunner::new().with_timeouts(Timeouts {
            read: Duration::from_millis(1),
            prefetch: Duration::from_mins(1),
            heavy: Duration::from_mins(1),
        });

        let as_read = runner
            .run(
                None,
                &[
                    "clone",
                    "--bare",
                    "--quiet",
                    &url,
                    &root.join("read.git").to_string_lossy(),
                ],
                None,
            )
            .await;
        match as_read {
            Err(GitError::TimedOut(budget)) => assert_eq!(budget, Duration::from_millis(1)),
            other => panic!("a read must expire on the read budget, got {other:?}"),
        }

        if let Err(e) = runner
            .run_heavy(
                None,
                &[
                    "clone",
                    "--bare",
                    "--quiet",
                    &url,
                    &root.join("heavy.git").to_string_lossy(),
                ],
                None,
            )
            .await
        {
            panic!("the same work must fit inside the heavy budget: {e}");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn run_times_out_and_kills() {
        let runner = GitRunner::new().with_timeouts(Timeouts {
            read: Duration::from_millis(200),
            ..Timeouts::default()
        });
        let result = runner
            .run(
                None,
                &["clone", "http://127.0.0.1:1/x.git", "/nonexistent-target"],
                None,
            )
            .await;
        match result {
            Err(GitError::TimedOut(_) | GitError::Failed(_) | GitError::NotFound) => {}
            other => panic!("expected timeout/failure, got {other:?}"),
        }
    }
}
