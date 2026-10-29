use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use crate::engine::runner::{GitCredentials, GitError, GitRunner};

const FETCH_BATCH: usize = 500;
const MIN_OID_LEN: usize = 7;
const FETCH_ATTEMPTS: u32 = 2;

/// Whether a raw-diff field names a real object. The all-zero OID marks an
/// absent side (added or deleted path) and is not fetchable — its length
/// varies with the repository's hash algorithm, so it is recognized by shape,
/// not by comparison with one fixed constant.
pub(super) fn is_object_oid(field: &str) -> bool {
    field.len() >= MIN_OID_LEN
        && field.chars().all(|c| c.is_ascii_hexdigit())
        && field.chars().any(|c| c != '0')
}

/// Fetch the blobs a window of commits touches, in batches.
///
/// A blobless clone would otherwise make git lazily fetch each blob on its own
/// round trip while computing numstat/patches. Batching is what git's own
/// `backfill` does; the OIDs come from tree diffs, which need no blobs.
///
/// # Errors
///
/// [`GitError`] when listing the OIDs or a batch fetch fails.
pub async fn prefetch(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
    cap_bytes: u64,
) -> Result<usize, GitError> {
    if shas.is_empty() {
        return Ok(0);
    }

    let oids = needed_oids(runner, git_dir, shas, creds).await?;
    if oids.is_empty() {
        return Ok(0);
    }

    // Consecutive pages share blobs — the src side of a file changed on this
    // page is the dst side fetched on the last one — so whenever the purge
    // has not run in between, most of the window is already local. Fetching
    // it again pays origin round trips for nothing and lands duplicate
    // copies in yet another pack.
    let wanted = oids.len();
    let ordered = missing_oids(runner, git_dir, oids, creds).await?;
    for batch in ordered.chunks(FETCH_BATCH) {
        // WORKAROUND: with stderr not a terminal git asks the origin for no
        // progress, leaving only keepalives, which sit at the runner's
        // stalled-transfer floor of one byte per second.
        let mut args = vec!["fetch", "--no-write-fetch-head", "--progress", "origin"];
        args.extend(batch.iter().map(String::as_str));
        fetch_batch(
            runner,
            git_dir,
            &args,
            creds,
            cap_bytes,
            wanted,
            batch.len(),
        )
        .await?;
    }
    Ok(ordered.len())
}

async fn fetch_batch(
    runner: &GitRunner,
    git_dir: &Path,
    args: &[&str],
    creds: &GitCredentials,
    cap_bytes: u64,
    wanted: usize,
    batch_len: usize,
) -> Result<(), GitError> {
    let entry = git_dir
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut attempt = 1;
    loop {
        let started = Instant::now();
        // Capped mid-transfer like every other operation that grows the
        // entry: a window whose blobs would take the repository past its cap
        // must be refused, not discovered afterwards on a full volume.
        let fetched = runner
            .run_prefetch_capped(git_dir, args, creds, cap_bytes)
            .await;
        let elapsed_ms = started.elapsed().as_millis();

        match fetched {
            Ok(_) => {
                tracing::info!(
                    entry,
                    wanted,
                    batch_len,
                    attempt,
                    elapsed_ms,
                    "prefetched a blob batch"
                );
                return Ok(());
            }
            Err(GitError::Failed(stderr)) if attempt < FETCH_ATTEMPTS => {
                let last_line = stderr.lines().last().unwrap_or_default();
                tracing::warn!(
                    entry,
                    wanted,
                    batch_len,
                    attempt,
                    elapsed_ms,
                    error = last_line,
                    "blob batch fetch failed; retrying it"
                );
                attempt += 1;
            }
            Err(e) => {
                tracing::warn!(entry, wanted, batch_len, attempt, elapsed_ms, error = %e, "blob batch fetch failed");
                return Err(e);
            }
        }
    }
}

/// The subset of `oids` absent from the local object store.
///
/// `rev-list --missing=print` is the partial-clone-safe form: it REPORTS a
/// missing object (`?<oid>`) instead of lazily fetching it, which is exactly
/// the behaviour a presence check must have — `cat-file` would trigger the
/// one-round-trip-per-blob fetches this module exists to prevent.
async fn missing_oids(
    runner: &GitRunner,
    git_dir: &Path,
    oids: BTreeSet<String>,
    creds: &GitCredentials,
) -> Result<Vec<String>, GitError> {
    let ordered: Vec<String> = oids.into_iter().collect();
    let mut missing = Vec::new();
    for batch in ordered.chunks(FETCH_BATCH) {
        let mut args = vec![
            "rev-list",
            "--objects",
            "--missing=print",
            "--no-object-names",
        ];
        args.extend(batch.iter().map(String::as_str));

        let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
        let listed = String::from_utf8_lossy(&output.stdout);
        missing.extend(
            listed
                .lines()
                .filter_map(|line| line.strip_prefix('?'))
                .map(str::to_owned),
        );
    }
    Ok(missing)
}

async fn needed_oids(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<BTreeSet<String>, GitError> {
    // Batched like every other window reader: the runner materialises the
    // child's whole stdout, and a wide window would buffer it all at once.
    // The oid SET is kept across batches — it is what the fetch needs, and it
    // is bounded by unique blobs rather than by raw-output bytes.
    let mut oids = BTreeSet::new();
    for batch in shas.chunks(ENUMERATE_BATCH) {
        // `log --no-walk` (not `diff-tree`) is the multi-commit form:
        // diff-tree with several revisions diffs BETWEEN them instead of per
        // commit. `--no-abbrev` is required — raw output abbreviates OIDs by
        // default, and `--full-index` only affects patch headers, not raw
        // lines. `--no-renames` is required too: `log` detects renames by
        // default, and rename detection compares blob CONTENT, which would
        // make the enumeration step itself trigger the lazy fetches it exists
        // to prevent. A rename then lists as a delete plus an add, which
        // still names both sides' oids.
        let mut args = vec![
            "log",
            "--no-walk",
            "--raw",
            "--no-renames",
            "--no-abbrev",
            "--no-color",
            "--root",
            "--pretty=format:",
        ];
        args.extend(batch.iter().map(String::as_str));

        let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
        let raw = String::from_utf8_lossy(&output.stdout);
        oids.append(&mut parse_raw_oids(&raw));
    }
    Ok(oids)
}

/// Commits per raw-diff enumeration; bounds the child's buffered stdout.
const ENUMERATE_BATCH: usize = 128;

/// Tree entry mode of a submodule reference.
const GITLINK_MODE: &str = "160000";

/// Collect both sides of every changed path from `--raw` output:
/// `:<src_mode> <dst_mode> <src_oid> <dst_oid> <status>\t<path>`.
///
/// A side whose mode is [`GITLINK_MODE`] is skipped. Its object id names a
/// commit in the SUBMODULE, which the superproject's origin has never heard
/// of: asking for it makes the whole batch fetch fail with "not our ref",
/// which this service reads as an origin refusing promisor wants and answers
/// by rebuilding the entry as a full clone. One submodule would therefore
/// force every repository containing it out of the blobless regime for good.
fn parse_raw_oids(raw: &str) -> BTreeSet<String> {
    let mut oids = BTreeSet::new();
    for line in raw.lines() {
        let Some(rest) = line.strip_prefix(':') else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let src_mode = fields.next();
        let dst_mode = fields.next();
        let src = fields.next();
        let dst = fields.next();

        for (mode, oid) in [(src_mode, src), (dst_mode, dst)] {
            let (Some(mode), Some(oid)) = (mode, oid) else {
                continue;
            };
            if mode != GITLINK_MODE && is_object_oid(oid) {
                oids.insert(oid.to_owned());
            }
        }
    }
    oids
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::engine::runner::Timeouts;

    const PATIENT: Duration = Duration::from_secs(30);

    /// A scratch entry whose `git` runs `first` on its first call and `then`
    /// on every call, counting its calls.
    struct FakeOrigin {
        root: PathBuf,
        git_dir: PathBuf,
        runner: GitRunner,
    }

    impl FakeOrigin {
        fn new(tag: &str, first: &str, then: &str, budget: Duration) -> Self {
            // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; the name carries pid and tag and holds no secrets
            let root = std::env::temp_dir()
                .join(format!("git-cli-proxy-blobs-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let git_dir = root.join("entry").join("repo.git");
            if let Err(e) = std::fs::create_dir_all(&git_dir) {
                panic!("scratch entry: {e}");
            }
            let script = root.join("git");
            let source = format!(
                "#!/bin/sh\n\
                 echo call >> {calls}\n\
                 if [ \"$(wc -l < {calls})\" -eq 1 ]; then {first}; fi\n\
                 {then}\n",
                calls = root.join("calls").display()
            );
            if let Err(e) = std::fs::write(&script, source) {
                panic!("write fake git: {e}");
            }
            let executable = std::os::unix::fs::PermissionsExt::from_mode(0o755);
            if let Err(e) = std::fs::set_permissions(&script, executable) {
                panic!("chmod fake git: {e}");
            }
            let runner = GitRunner::new()
                .with_git_binary(script)
                .with_timeouts(Timeouts {
                    read: Duration::from_secs(5),
                    prefetch: budget,
                    heavy: Duration::from_secs(5),
                });
            Self {
                root,
                git_dir,
                runner,
            }
        }

        fn calls(&self) -> usize {
            std::fs::read_to_string(self.root.join("calls")).map_or(0, |c| c.lines().count())
        }

        async fn fetch(&self) -> Result<(), GitError> {
            let creds = GitCredentials {
                username: "u".to_owned(),
                token: "t".to_owned(),
            };
            fetch_batch(
                &self.runner,
                &self.git_dir,
                &["fetch", "origin", "a".repeat(40).as_str()],
                &creds,
                u64::MAX,
                1,
                1,
            )
            .await
        }
    }

    impl Drop for FakeOrigin {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    async fn a_batch_that_fails_in_transfer_is_fetched_again_once() {
        let origin = FakeOrigin::new(
            "retry",
            "echo 'fatal: early EOF' >&2; exit 128",
            "exit 0",
            PATIENT,
        );
        let fetched = origin.fetch().await;
        assert!(fetched.is_ok(), "the second attempt succeeds: {fetched:?}");
        assert_eq!(origin.calls(), 2);
    }

    #[tokio::test]
    async fn a_batch_that_keeps_failing_is_given_up_after_its_second_attempt() {
        let origin = FakeOrigin::new(
            "retry-exhausted",
            "true",
            "echo 'fatal: early EOF' >&2; exit 128",
            PATIENT,
        );
        assert!(matches!(origin.fetch().await, Err(GitError::Failed(_))));
        assert_eq!(origin.calls(), FETCH_ATTEMPTS as usize);
    }

    #[tokio::test]
    async fn a_batch_failure_with_a_known_cause_is_never_retried() {
        let cases = [
            ("timeout", "exec sleep 30", Duration::from_secs(2)),
            (
                "throttled",
                "echo 'error: 429 Too Many Requests' >&2; exit 128",
                PATIENT,
            ),
            (
                "promisor",
                "echo 'fatal: not our ref' >&2; exit 128",
                PATIENT,
            ),
            (
                "auth",
                "echo 'fatal: Authentication failed' >&2; exit 128",
                PATIENT,
            ),
        ];
        for (name, first, budget) in cases {
            let origin = FakeOrigin::new(&format!("no-retry-{name}"), first, "exit 0", budget);
            assert!(origin.fetch().await.is_err(), "case {name} must surface");
            assert_eq!(origin.calls(), 1, "case {name} must not be retried");
        }
    }

    #[test]
    fn a_submodule_reference_is_never_requested_from_origin() {
        // `160000` is a gitlink: the id is a commit in the submodule, which
        // the superproject's origin does not have. Asking for it fails the
        // whole batch with "not our ref", which this service classifies as a
        // promisor refusal and answers by promoting the entry to a full clone.
        let raw = concat!(
            ":000000 100644 0000000000000000000000000000000000000000 ",
            "1111111111111111111111111111111111111111 A\to.txt\n",
            ":000000 160000 0000000000000000000000000000000000000000 ",
            "2222222222222222222222222222222222222222 A\tsub\n",
            ":160000 160000 3333333333333333333333333333333333333333 ",
            "4444444444444444444444444444444444444444 M\tsub\n",
        );
        let oids = parse_raw_oids(raw);
        assert_eq!(
            oids.into_iter().collect::<Vec<_>>(),
            vec!["1111111111111111111111111111111111111111".to_owned()],
            "only the real blob may be requested"
        );
    }

    #[test]
    fn collects_both_sides_of_a_modification() {
        let raw = ":100644 100644 aaa1111 bbb2222 M\tsrc/app.rs\n";
        let oids = parse_raw_oids(raw);
        assert!(oids.contains("aaa1111"), "source side must be fetched");
        assert!(oids.contains("bbb2222"), "target side must be fetched");
    }

    #[test]
    fn skips_absent_sides_and_noise() {
        let zero = "0".repeat(40);
        let raw = format!(
            ":000000 100644 {zero} ccc3333 A\tnew.rs\n\
             :100644 000000 ddd4444 {zero} D\tgone.rs\n\
             not a raw line\n\
             \n"
        );
        let oids = parse_raw_oids(&raw);
        assert_eq!(
            oids,
            ["ccc3333".to_owned(), "ddd4444".to_owned()]
                .into_iter()
                .collect::<BTreeSet<_>>(),
            "the all-zero OID is not an object"
        );
    }

    #[test]
    fn deduplicates_across_commits() {
        let raw = ":100644 100644 aaa1111 bbb2222 M\ta.rs\n\
                   :100644 100644 aaa1111 bbb2222 M\tb.rs\n";
        assert_eq!(parse_raw_oids(raw).len(), 2, "each OID is fetched once");
    }

    #[test]
    fn absent_sides_are_recognized_by_shape_not_length() {
        let cases = vec![
            ("sha1 zeros", "0".repeat(40), false),
            ("sha256 zeros", "0".repeat(64), false),
            ("short zeros", "0000000".to_owned(), false),
            ("real oid", "a".repeat(40), true),
            ("too short", "abc".to_owned(), false),
            ("not hex", "z".repeat(40), false),
        ];
        for (name, field, expected) in cases {
            assert_eq!(is_object_oid(&field), expected, "case: {name}");
        }
    }

    #[test]
    fn rejects_non_hex_fields() {
        let raw = ":100644 100644 zzzzzzz bbb2222 M\ta.rs\n";
        let oids = parse_raw_oids(raw);
        assert_eq!(oids.len(), 1, "only the hex OID survives: {oids:?}");
    }
}
