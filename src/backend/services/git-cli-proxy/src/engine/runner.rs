use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rustix::process::{Pid, Signal, kill_process_group};
use toolkit_security::crypto::hash::Sha256;

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
    fn basic_header(&self) -> String {
        let pair = format!("{}:{}", self.username, self.token);
        format!("Authorization: Basic {}", BASE64.encode(pair))
    }

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

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub read: Duration,
    pub prefetch: Duration,
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

#[derive(Debug, Clone)]
pub struct GitRunner {
    timeouts: Timeouts,
    cap_poll: Duration,
    ca_cert_path: Option<String>,
    git_binary: std::path::PathBuf,
}

const STDERR_TAIL_BYTES: usize = 4096;
// git's default is one delta-search thread per host CPU, which ignores the
// container's own CPU limit.
const PACK_THREADS: usize = 2;
const PACK_WINDOW_MEMORY_MB: usize = 256;
const PACK_DELTA_CACHE_MB: usize = 256;
const READ_TIMEOUT: Duration = Duration::from_mins(30);
const PREFETCH_TIMEOUT: Duration = Duration::from_mins(10);
const HEAVY_OP_TIMEOUT: Duration = Duration::from_mins(30);
const STALLED_TRANSFER_BYTES_PER_SEC: u32 = 1;
const STALLED_TRANSFER_SECS: u32 = 300;
const CAP_POLL_INTERVAL: Duration = Duration::from_secs(5);
const KILL_GRACE: Duration = Duration::from_secs(5);

impl Default for GitRunner {
    fn default() -> Self {
        Self {
            timeouts: Timeouts::default(),
            cap_poll: CAP_POLL_INTERVAL,
            ca_cert_path: None,
            git_binary: std::path::PathBuf::from("git"),
        }
    }
}

impl GitRunner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    #[cfg(test)]
    fn with_cap_poll(mut self, interval: Duration) -> Self {
        self.cap_poll = interval;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_git_binary(mut self, binary: std::path::PathBuf) -> Self {
        self.git_binary = binary;
        self
    }

    #[must_use]
    pub fn with_ca_cert(mut self, path: Option<String>) -> Self {
        self.ca_cert_path = path.filter(|value| !value.is_empty());
        self
    }

    pub async fn run(
        &self,
        git_dir: Option<&Path>,
        args: &[&str],
        creds: Option<&GitCredentials>,
    ) -> Result<Output, GitError> {
        self.run_within(self.timeouts.read, git_dir, args, creds)
            .await
    }

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
        let mut command = self.base_command(creds, &[]);
        if let Some(dir) = git_dir {
            command.arg("--git-dir").arg(dir);
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = command.spawn()?;
        let groups = [process_group_of(&child)];
        let output = within_budget(budget, &groups, async {
            child.wait_with_output().await.map_err(GitError::Io)
        })
        .await?;

        if output.status.success() {
            return Ok(output);
        }
        Err(classify_failure(&output))
    }

    pub async fn run_capped(
        &self,
        git_dir: Option<&Path>,
        args: &[&str],
        creds: Option<&GitCredentials>,
        watch: &Path,
        cap_bytes: u64,
    ) -> Result<Output, GitError> {
        self.run_capped_within(CappedRun::Heavy, git_dir, args, creds, watch, cap_bytes)
            .await
    }

    pub async fn run_prefetch_capped(
        &self,
        git_dir: &Path,
        args: &[&str],
        creds: &GitCredentials,
        cap_bytes: u64,
    ) -> Result<Output, GitError> {
        self.run_capped_within(
            CappedRun::Prefetch,
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
        run: CappedRun,
        git_dir: Option<&Path>,
        args: &[&str],
        creds: Option<&GitCredentials>,
        watch: &Path,
        cap_bytes: u64,
    ) -> Result<Output, GitError> {
        let (budget, extra_config) = match run {
            CappedRun::Heavy => (self.timeouts.heavy, Vec::new()),
            CappedRun::Prefetch => (self.timeouts.prefetch, stalled_transfer_abort().to_vec()),
        };
        let mut command = self.base_command(creds, &extra_config);
        if let Some(dir) = git_dir {
            command.arg("--git-dir").arg(dir);
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = command.spawn()?;
        let groups = [process_group_of(&child)];
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
                            kill_and_reap(&groups, &mut wait).await;
                            return Err(GitError::TooLarge { cap_bytes });
                        }
                    }
                }
            }
        };

        let output = within_budget(budget, &groups, capped).await?;

        if output.status.success() {
            return Ok(output);
        }
        Err(classify_failure(&output))
    }

    pub async fn run_piped(
        &self,
        git_dir: &Path,
        producer: &[&str],
        consumer: &[&str],
        creds: &GitCredentials,
    ) -> Result<Vec<u8>, GitError> {
        let mut left = self.base_command(Some(creds), &[]);
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

        let mut right = self.base_command(Some(creds), &[]);
        right
            .arg("--git-dir")
            .arg(git_dir)
            .args(consumer)
            .stdin(Stdio::from(left_stdout.into_owned_fd()?))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let right_child = right.spawn()?;
        let groups = [
            process_group_of(&left_child),
            process_group_of(&right_child),
        ];

        // INVARIANT: both sides are drained concurrently. Awaiting the consumer
        // to completion first would deadlock the producer once it writes past
        // the stderr pipe buffer, and the call would surface as a timeout.
        let joined = async {
            tokio::try_join!(
                right_child.wait_with_output(),
                left_child.wait_with_output()
            )
            .map_err(GitError::Io)
        };

        let budget = self.timeouts.read;
        let (right_output, left_output) = within_budget(budget, &groups, joined).await?;

        if !left_output.status.success() {
            return Err(classify_failure(&left_output));
        }
        if !right_output.status.success() {
            return Err(classify_failure(&right_output));
        }
        Ok(right_output.stdout)
    }

    fn base_command(
        &self,
        creds: Option<&GitCredentials>,
        extra_config: &[(&'static str, String)],
    ) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.git_binary);
        // Its own group, so a budget overrun can take git's own children (a
        // repack's pack-objects) down with it instead of killing the parent
        // alone and leaving them writing into the entry.
        command.process_group(0);
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

        let mut pairs = self.config_pairs(creds);
        pairs.extend_from_slice(extra_config);
        for (index, (key, value)) in pairs.iter().enumerate() {
            command.env(format!("GIT_CONFIG_KEY_{index}"), key);
            command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
        if !pairs.is_empty() {
            command.env("GIT_CONFIG_COUNT", pairs.len().to_string());
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
        // page-serve once enough packs accumulate.
        pairs.push(("maintenance.auto", "false".to_owned()));
        pairs.push(("gc.auto", "0".to_owned()));
        pairs.push(("pack.threads", PACK_THREADS.to_string()));
        pairs.push(("pack.windowMemory", format!("{PACK_WINDOW_MEMORY_MB}m")));
        pairs.push(("pack.deltaCacheSize", format!("{PACK_DELTA_CACHE_MB}m")));
        pairs
    }
}

#[derive(Debug, Clone, Copy)]
enum CappedRun {
    Heavy,
    Prefetch,
}

fn stalled_transfer_abort() -> [(&'static str, String); 2] {
    [
        (
            "http.lowSpeedLimit",
            STALLED_TRANSFER_BYTES_PER_SEC.to_string(),
        ),
        ("http.lowSpeedTime", STALLED_TRANSFER_SECS.to_string()),
    ]
}

fn classify_failure(output: &Output) -> GitError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lower = stderr.to_lowercase();

    // Before `auth`: a vendor signals throttling with 403 as readily as with
    // 429 (GitHub's secondary rate limits do exactly that).
    let throttled = [
        "rate limit",
        "too many requests",
        "http 429",
        "returned error: 429",
    ];
    if throttled.iter().any(|m| lower.contains(m)) {
        return GitError::Throttled;
    }

    // Before `auth`: each vendor announces a repository it will never serve to
    // this client with a remote banner plus a bare 403.
    let unavailable = [
        "this repository is currently not available",
        "access to this repository has been disabled",
        "you are not allowed to download code from this project",
    ];
    if unavailable.iter().any(|m| lower.contains(m)) {
        return GitError::OriginUnavailable;
    }

    let auth = [
        "authentication failed",
        "http 401",
        "401 unauthorized",
        "could not read username",
        "you are not allowed to download code.",
        "has enabled or enforced saml sso",
        "ip allow list",
        "pulling over http is not allowed",
    ];
    if auth.iter().any(|m| lower.contains(m)) {
        return GitError::AuthRejected;
    }

    let forbidden = ["returned error: 403", "403 forbidden"];
    if forbidden.iter().any(|m| lower.contains(m)) {
        return GitError::OriginUnavailable;
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
    // refused connection. A genuinely absent repository over http(s) always
    // carries one of the specific lines below.
    let missing = [
        "repository not found",
        "http 404",
        "returned error: 404",
        "does not appear to be a git repository",
        "a repository for this project does not exist yet",
        "the project you were looking for could not be found",
        "no longer exists in this workspace",
    ];
    if missing.iter().any(|m| lower.contains(m)) || names_missing_repository(&lower) {
        return GitError::NotFound;
    }

    let tail_start = stderr.len().saturating_sub(STDERR_TAIL_BYTES);
    let mut cut = tail_start;
    while cut < stderr.len() && !stderr.is_char_boundary(cut) {
        cut += 1;
    }
    GitError::Failed(stderr[cut..].trim().to_owned())
}

// WORKAROUND: on an HTTP 404 git's transport prints the URL between
// `repository` and `not found`, so no fixed substring matches the line.
fn names_missing_repository(lower: &str) -> bool {
    lower.lines().any(|line| {
        let line = line.trim();
        line.starts_with("fatal: repository '") && line.ends_with("' not found")
    })
}

fn process_group_of(child: &tokio::process::Child) -> Option<Pid> {
    child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .and_then(Pid::from_raw)
}

async fn within_budget<T>(
    budget: Duration,
    groups: &[Option<Pid>],
    work: impl Future<Output = Result<T, GitError>>,
) -> Result<T, GitError> {
    tokio::pin!(work);
    tokio::select! {
        finished = &mut work => finished,
        () = tokio::time::sleep(budget) => {
            kill_and_reap(groups, &mut work).await;
            Err(GitError::TimedOut(budget))
        }
    }
}

async fn kill_and_reap(groups: &[Option<Pid>], work: impl Future) {
    for group in groups.iter().flatten() {
        let _ = kill_process_group(*group, Signal::KILL);
    }
    if tokio::time::timeout(KILL_GRACE, work).await.is_err() {
        tracing::warn!("a killed git child did not exit after SIGKILL");
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::process::ExitStatus;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::engine::fake_git::write_fake_git;

    #[tokio::test]
    async fn only_the_prefetch_aborts_a_stalled_transfer() {
        let name = format!("git-cli-proxy-stall-config-{}", std::process::id());
        // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; the name carries pid and holds no secrets
        let root = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&root);
        if let Err(e) = std::fs::create_dir_all(&root) {
            panic!("scratch dir: {e}");
        }
        let script = root.join("git");
        let seen = root.join("env");
        write_fake_git(&script, &format!("env > {}", seen.display()));
        let runner = GitRunner::new().with_git_binary(script);
        let creds = GitCredentials {
            username: "u".to_owned(),
            token: "t".to_owned(),
        };
        let configured = || std::fs::read_to_string(&seen).unwrap_or_default();

        let prefetched = runner
            .run_prefetch_capped(&root, &["fetch"], &creds, u64::MAX)
            .await;
        assert!(prefetched.is_ok(), "fake prefetch: {prefetched:?}");
        assert!(
            configured().contains("=http.lowSpeedLimit"),
            "the prefetch must carry the stall abort"
        );

        let heavy = runner
            .run_capped(Some(&root), &["fetch"], Some(&creds), &root, u64::MAX)
            .await;
        assert!(heavy.is_ok(), "fake heavy run: {heavy:?}");
        assert!(
            !configured().contains("lowSpeed"),
            "a clone or fetch must not inherit the prefetch's stall abort"
        );

        let read = runner.run(Some(&root), &["log"], Some(&creds)).await;
        assert!(read.is_ok(), "fake read: {read:?}");
        assert!(!configured().contains("lowSpeed"), "nor may a local read");

        let _ = std::fs::remove_dir_all(&root);
    }

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
                matches!(e, GitError::OriginUnavailable)
            }),
            (
                "fatal: could not read Username for 'https://x': terminal prompts disabled",
                |e| matches!(e, GitError::AuthRejected),
            ),
            ("remote: Repository not found.", |e| {
                matches!(e, GitError::NotFound)
            }),
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
            (
                "remote: You have exceeded a secondary rate limit\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::Throttled),
            ),
            (
                "fatal: unable to access 'https://x/': The requested URL returned error: 429",
                |e| matches!(e, GitError::Throttled),
            ),
            (
                "remote: This repository is currently not available.\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
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
    fn classifies_vendor_banners_by_who_can_fix_them() {
        let cases: Vec<(&str, Check)> = vec![
            ("fatal: repository 'https://x/g/p.git/' not found", |e| {
                matches!(e, GitError::NotFound)
            }),
            (
                "remote: A repository for this project does not exist yet.\n\
                 fatal: repository 'https://x/g/p.git/' not found",
                |e| matches!(e, GitError::NotFound),
            ),
            (
                "remote: The project you were looking for could not be found or you don't have permission to view it.\n\
                 fatal: repository 'https://x/g/p.git/' not found",
                |e| matches!(e, GitError::NotFound),
            ),
            (
                "remote: You may not have access to this repository or it no longer exists in this workspace. If you think this repository exists and you have access, make sure you are authenticated.\n\
                 fatal: repository 'https://x/w/r.git/' not found",
                |e| matches!(e, GitError::NotFound),
            ),
            (
                "remote: Access to this repository has been disabled by GitHub staff due to excessive resource use.\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
            (
                "remote: You are not allowed to download code from this project.\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
            (
                "remote: You are not allowed to download code.\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::AuthRejected),
            ),
            (
                "remote: The `acme` organization has enabled or enforced SAML SSO. To access this repository, you must re-authorize the OAuth Application.\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::AuthRejected),
            ),
            (
                "remote: Although you appear to have the correct authorization credentials, the `acme` organization has an IP allow list enabled, and your IP address is not permitted to access this resource.\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::AuthRejected),
            ),
            (
                "remote: Pulling over HTTP is not allowed.\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::AuthRejected),
            ),
            (
                "fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
        ];
        for (stderr, check) in cases {
            let err = classify_failure(&failed_output(stderr));
            assert!(check(&err), "stderr {stderr:?} classified as {err:?}");
        }
    }

    #[test]
    fn vendor_banners_classify_without_the_transport_line() {
        let cases: Vec<(&str, Check)> = vec![
            (
                "remote: A repository for this project does not exist yet.",
                |e| matches!(e, GitError::NotFound),
            ),
            (
                "remote: The project you were looking for could not be found or you don't have permission to view it.",
                |e| matches!(e, GitError::NotFound),
            ),
            (
                "remote: You may not have access to this repository or it no longer exists in this workspace.",
                |e| matches!(e, GitError::NotFound),
            ),
            ("remote: This repository is currently not available.", |e| {
                matches!(e, GitError::OriginUnavailable)
            }),
            (
                "remote: Access to this repository has been disabled by GitHub staff.",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
            (
                "remote: You are not allowed to download code from this project.",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
            ("remote: You are not allowed to download code.", |e| {
                matches!(e, GitError::AuthRejected)
            }),
            (
                "remote: The `acme` organization has enabled or enforced SAML SSO.",
                |e| matches!(e, GitError::AuthRejected),
            ),
            (
                "remote: the `acme` organization has an IP allow list enabled, and your IP address is not permitted to access this resource.",
                |e| matches!(e, GitError::AuthRejected),
            ),
            ("remote: Pulling over HTTP is not allowed.", |e| {
                matches!(e, GitError::AuthRejected)
            }),
            ("error: repository 'https://x/g/p.git/' not found", |e| {
                matches!(e, GitError::Failed(_))
            }),
            (
                "fatal: repository 'https://x/g/p.git/' not found in the local cache",
                |e| matches!(e, GitError::Failed(_)),
            ),
            ("fatal: couldn't find remote ref 'refs/heads/main'", |e| {
                matches!(e, GitError::Failed(_))
            }),
        ];
        for (stderr, check) in cases {
            let err = classify_failure(&failed_output(stderr));
            assert!(check(&err), "stderr {stderr:?} classified as {err:?}");
        }
    }

    #[test]
    fn earlier_fingerprints_win_over_later_ones() {
        let cases: Vec<(&str, Check)> = vec![
            (
                "remote: This repository is currently not available.\n\
                 remote: rate limit exceeded",
                |e| matches!(e, GitError::Throttled),
            ),
            (
                "fatal: Authentication failed for 'https://x/'\n\
                 remote: too many requests",
                |e| matches!(e, GitError::Throttled),
            ),
            (
                "remote: You are not allowed to download code from this project.\n\
                 fatal: Authentication failed for 'https://x/'",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
            (
                "fatal: Authentication failed for 'https://x/'\n\
                 remote: Access to this repository has been disabled by GitHub staff.",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
            (
                "fatal: unable to access 'https://x/': The requested URL returned error: 403\n\
                 remote: The `acme` organization has enabled or enforced SAML SSO.",
                |e| matches!(e, GitError::AuthRejected),
            ),
            (
                "error: https://x/a.git did not send all necessary objects\n\
                 fatal: unable to access 'https://x/': The requested URL returned error: 403",
                |e| matches!(e, GitError::OriginUnavailable),
            ),
            (
                "error: https://x/a.git did not send all necessary objects\n\
                 fatal: repository 'https://x/a.git/' not found",
                |e| matches!(e, GitError::PromisorRefused),
            ),
            (
                "remote: Repository not found.\n\
                 fatal: Authentication failed for 'https://x/'",
                |e| matches!(e, GitError::AuthRejected),
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
    async fn no_read_budget_outlives_the_heavy_ceiling() {
        let defaults = Timeouts::default();
        assert!(
            defaults.read <= defaults.heavy && defaults.prefetch <= defaults.heavy,
            "no budget may exceed the heavy ceiling: {defaults:?}"
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

    struct FakeGit {
        dir: PathBuf,
        script: PathBuf,
        heartbeat: PathBuf,
    }

    const HEARTBEAT_QUIET: Duration = Duration::from_millis(200);

    static FAKE_GIT_SEQ: AtomicU64 = AtomicU64::new(0);

    impl FakeGit {
        fn new(body: impl FnOnce(&Path) -> String) -> Self {
            // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; the name carries pid and a per-process counter and holds no secrets
            let dir = std::env::temp_dir().join(format!(
                "git-cli-proxy-fake-git-{}-{}",
                std::process::id(),
                FAKE_GIT_SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            if let Err(e) = std::fs::create_dir_all(&dir) {
                panic!("fake git dir: {e}");
            }
            let heartbeat = dir.join("heartbeat");
            let script = dir.join("git");
            let body = body(&dir);
            let source = format!(
                "(while :; do echo beat >> {beat}; sleep 0.01; done) &\n\
                 until [ -s {beat} ]; do sleep 0.01; done\n\
                 {body}\n\
                 exec sleep 30",
                beat = heartbeat.display()
            );
            write_fake_git(&script, &source);
            Self {
                dir,
                script,
                heartbeat,
            }
        }

        fn heartbeat_bytes(&self) -> u64 {
            std::fs::metadata(&self.heartbeat).map_or(0, |m| m.len())
        }

        // A zombie still answers `kill(-pgid, 0)` until PID 1 reaps it, so the
        // oracle is that the beating has stopped, not the process table.
        async fn assert_nothing_still_writing(&self, ended_by: &str) {
            let before = self.heartbeat_bytes();
            assert!(
                before > 0,
                "the fake git must have started beating before {ended_by}"
            );

            tokio::time::sleep(HEARTBEAT_QUIET).await;
            let after = self.heartbeat_bytes();
            assert_eq!(
                before, after,
                "every process of a launch ended by {ended_by} must be dead; the heartbeat is still growing"
            );
        }
    }

    impl Drop for FakeGit {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn a_timed_out_launch_leaves_no_process_behind() {
        let fake = FakeGit::new(|_| String::new());
        let runner = GitRunner::new()
            .with_git_binary(fake.script.clone())
            .with_timeouts(Timeouts {
                read: Duration::from_millis(300),
                ..Timeouts::default()
            });
        match runner.run(None, &["anything"], None).await {
            Err(GitError::TimedOut(_)) => {}
            other => panic!("expected the budget to expire, got {other:?}"),
        }

        fake.assert_nothing_still_writing("the budget").await;
    }

    #[tokio::test]
    async fn a_launch_killed_by_the_cap_leaves_no_process_behind() {
        let fake = FakeGit::new(|dir| {
            format!(
                "echo grown > {}",
                dir.join("watched").join("blob").display()
            )
        });
        let watch = fake.dir.join("watched");
        if let Err(e) = std::fs::create_dir_all(&watch) {
            panic!("watched dir: {e}");
        }

        let runner = GitRunner::new()
            .with_git_binary(fake.script.clone())
            .with_cap_poll(Duration::from_millis(1));
        match runner
            .run_capped(None, &["anything"], None, &watch, 0)
            .await
        {
            Err(GitError::TooLarge { cap_bytes }) => assert_eq!(cap_bytes, 0),
            other => panic!("expected the cap to fire, got {other:?}"),
        }

        fake.assert_nothing_still_writing("the cap").await;
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
