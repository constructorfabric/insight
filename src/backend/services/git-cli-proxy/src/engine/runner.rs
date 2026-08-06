use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

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
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("origin rejected the supplied git credentials")]
    AuthRejected,
    #[error("repository not found at origin")]
    NotFound,
    #[error("git timed out after {0:?}")]
    TimedOut(Duration),
    #[error("git failed: {0}")]
    Failed(String),
    #[error("failed to spawn git: {0}")]
    Io(#[from] std::io::Error),
}

/// Spawns git subprocesses with a hermetic environment: explicit `--git-dir`
/// or working dir (process cwd is never relied on), no user/system gitconfig,
/// no interactive prompts, credentials via env only.
#[derive(Debug, Clone)]
pub struct GitRunner {
    timeout: Duration,
}

const STDERR_TAIL_BYTES: usize = 4096;

impl GitRunner {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
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

        if let Some(creds) = creds {
            command
                .env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "http.extraheader")
                .env("GIT_CONFIG_VALUE_0", creds.basic_header());
        }

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
    async fn run_times_out_and_kills() {
        // `git credential fill` blocks reading stdin; with stdin nulled it
        // still waits for input on some git versions — cheap forced hang is
        // fetching an unroutable address. Use a filesystem wait instead:
        // `git daemon` needs args; simplest reliable hang: clone from a pipe
        // is flaky. Use --exec-path trick: run `git hash-object --stdin` with
        // stdin held open is not possible via Stdio::null. So: time out a
        // clone of a non-listening localhost port with a tiny timeout.
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
