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
    #[error("origin refuses to serve explicitly requested objects")]
    PromisorRefused,
    #[error("the cache cannot take more disk right now")]
    AdmissionRejected,
    #[error("git timed out after {0:?}")]
    TimedOut(Duration),
    #[error("git failed: {0}")]
    Failed(String),
    #[error("failed to spawn git: {0}")]
    Io(#[from] std::io::Error),
    #[error("repository exceeds the per-repository size cap of {cap_bytes} bytes")]
    TooLarge { cap_bytes: u64 },
}

/// Spawns git subprocesses with a hermetic environment: explicit `--git-dir`
/// or working dir (process cwd is never relied on), no user/system gitconfig,
/// no interactive prompts, credentials via env only.
#[derive(Debug, Clone)]
pub struct GitRunner {
    timeout: Duration,
    /// How often a capped run re-measures. Trades overshoot against the cost
    /// of walking a tree that is actively being written.
    cap_poll: Duration,
    /// PEM bundle for origins whose TLS chain is not in the system store
    /// (a self-hosted vendor behind a private CA). Empty = system store only.
    ca_cert_path: Option<String>,
}

const STDERR_TAIL_BYTES: usize = 4096;
/// How often a capped run re-measures the tree it is filling. The cap can be
/// overshot by one interval's worth of download; the post-hoc check is what
/// catches that remainder.
const CAP_POLL_INTERVAL: Duration = Duration::from_secs(5);

impl GitRunner {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            cap_poll: CAP_POLL_INTERVAL,
            ca_cert_path: None,
        }
    }

    #[must_use]
    pub fn with_cap_poll(mut self, interval: Duration) -> Self {
        self.cap_poll = interval;
        self
    }

    #[must_use]
    pub fn with_ca_cert(mut self, path: Option<String>) -> Self {
        self.ca_cert_path = path.filter(|value| !value.is_empty());
        self
    }

    /// Run `git <args>` against `git_dir` (None for `clone`, which creates
    /// the dir). Non-zero exit is classified into [`GitError`].
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

        let waited = tokio::time::timeout(self.timeout, command.output()).await;
        let output = match waited {
            Ok(result) => result?,
            Err(_elapsed) => return Err(GitError::TimedOut(self.timeout)),
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

        let output = match tokio::time::timeout(self.timeout, capped).await {
            Ok(result) => result?,
            Err(_elapsed) => return Err(GitError::TimedOut(self.timeout)),
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

        let (right_output, left_output) = match tokio::time::timeout(self.timeout, joined).await {
            Ok(result) => result?,
            Err(_elapsed) => return Err(GitError::TimedOut(self.timeout)),
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
        pairs
    }
}

/// Map a failed git invocation to a typed error by stderr fingerprints. The
/// stored remote URL is credential-free, so quoting stderr leaks no secrets.
fn classify_failure(output: &Output) -> GitError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lower = stderr.to_lowercase();

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

    let missing = [
        "repository not found",
        "http 404",
        "does not appear to be a git repository",
        "could not read from remote repository",
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
        let plain = GitRunner::new(Duration::from_secs(1));
        assert!(
            plain.config_pairs(None).is_empty(),
            "no creds and no CA means no git config at all"
        );

        let with_creds = plain.config_pairs(Some(&creds));
        assert_eq!(with_creds.len(), 1);
        assert_eq!(with_creds[0].0, "http.extraheader");

        let with_ca =
            GitRunner::new(Duration::from_secs(1)).with_ca_cert(Some("/certs/ca.pem".to_owned()));
        let both = with_ca.config_pairs(Some(&creds));
        assert_eq!(both.len(), 2, "on-prem origins need the CA pair too");
        assert!(
            both.iter()
                .any(|(k, v)| *k == "http.sslCAInfo" && v == "/certs/ca.pem"),
            "got: {both:?}"
        );
    }

    #[test]
    fn empty_ca_path_is_treated_as_unset() {
        let runner = GitRunner::new(Duration::from_secs(1)).with_ca_cert(Some(String::new()));
        assert!(
            runner.config_pairs(None).is_empty(),
            "an empty CA path must not become a git config value"
        );
    }

    #[tokio::test]
    async fn run_reports_version() {
        let runner = GitRunner::new(Duration::from_secs(10));
        let output = match runner.run(None, &["--version"], None).await {
            Ok(o) => o,
            Err(e) => panic!("git --version failed: {e}"),
        };
        assert!(String::from_utf8_lossy(&output.stdout).contains("git version"));
    }

    #[tokio::test]
    async fn piped_producer_failure_carries_its_stderr() {
        let dir = std::env::temp_dir().join(format!(
            "git-cli-proxy-piped-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            panic!("create temp dir: {e}");
        }

        let runner = GitRunner::new(Duration::from_secs(10));
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
        let runner =
            GitRunner::new(Duration::from_mins(1)).with_cap_poll(Duration::from_millis(1));
        let result = runner
            .run_capped(
                None,
                &["clone", "--bare", "--quiet", &url, &target.to_string_lossy()],
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

        let runner =
            GitRunner::new(Duration::from_mins(1)).with_cap_poll(Duration::from_millis(1));
        if let Err(e) = runner
            .run_capped(
                None,
                &["clone", "--bare", "--quiet", &url, &target.to_string_lossy()],
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
    async fn run_times_out_and_kills() {
        let runner = GitRunner::new(Duration::from_millis(200));
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
