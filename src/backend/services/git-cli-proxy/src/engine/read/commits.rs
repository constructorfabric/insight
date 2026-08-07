use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::engine::runner::{GitCredentials, GitError, GitRunner};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CommitRow {
    pub sha: String,
    pub message: String,
    pub authored_date: String,
    pub committed_date: String,
    pub author_name: String,
    pub author_email: String,
    pub committer_name: String,
    pub committer_email: String,
    pub parent_hashes: Vec<String>,
    pub is_merge: bool,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub branch_names: Vec<String>,
    pub patch_id: Option<String>,
}

/// Commit identity plus walk-order key, before the expensive per-commit work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitHeader {
    pub sha: String,
    pub committed_date: String,
    pub authored_date: String,
    pub author_name: String,
    pub author_email: String,
    pub committer_name: String,
    pub committer_email: String,
    pub parent_hashes: Vec<String>,
    pub message: String,
}

impl CommitHeader {
    #[must_use]
    pub fn is_merge(&self) -> bool {
        self.parent_hashes.len() > 1
    }
}

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// Every commit reachable from any branch with `committed_date >= since`,
/// ordered ascending by `(committed_date, sha)` — the walk order the page
/// tokens depend on.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn headers(
    runner: &GitRunner,
    git_dir: &Path,
    since: Option<&str>,
    creds: &GitCredentials,
) -> Result<Vec<CommitHeader>, GitError> {
    let format = format!(
        "--pretty=format:{RECORD}%H{FIELD}%P{FIELD}%aI{FIELD}%cI{FIELD}%an{FIELD}%ae{FIELD}%cn{FIELD}%ce{FIELD}%B"
    );
    let mut args = vec!["log", "--all", "--no-color", &format];

    let since_arg = since.map(|value| format!("--since={value}"));
    if let Some(arg) = since_arg.as_deref() {
        args.push(arg);
    }

    let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
    let text = String::from_utf8_lossy(&output.stdout);

    let mut headers = parse_headers(&text);
    headers.sort_by(|a, b| (&a.committed_date, &a.sha).cmp(&(&b.committed_date, &b.sha)));
    Ok(headers)
}

/// Branch names containing each of `shas`.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn branch_membership(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<HashMap<String, Vec<String>>, GitError> {
    let mut membership: HashMap<String, Vec<String>> =
        shas.iter().map(|sha| (sha.clone(), Vec::new())).collect();
    if shas.is_empty() {
        return Ok(membership);
    }

    for sha in shas {
        let output = runner
            .run(
                Some(git_dir),
                &["branch", "--format=%(refname:short)", "--contains", sha],
                Some(creds),
            )
            .await?;
        let listing = String::from_utf8_lossy(&output.stdout);
        let names: Vec<String> = listing
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        membership.insert(sha.clone(), names);
    }
    Ok(membership)
}

/// Canonical patch ids (`git patch-id --stable`) for `shas`, keyed by sha.
/// Merge commits have no single diff and are absent from the map.
///
/// # Errors
///
/// [`GitError`] when either side of the pipe fails.
pub async fn patch_ids(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<HashMap<String, String>, GitError> {
    if shas.is_empty() {
        return Ok(HashMap::new());
    }

    let mut producer = vec!["log", "--no-walk", "--patch", "--no-color", "--root"];
    producer.extend(shas.iter().map(String::as_str));

    let stdout = runner
        .run_piped(git_dir, &producer, &["patch-id", "--stable"], creds)
        .await?;
    Ok(parse_patch_ids(&String::from_utf8_lossy(&stdout)))
}

fn parse_headers(text: &str) -> Vec<CommitHeader> {
    text.split(RECORD)
        .filter(|record| !record.trim().is_empty())
        .filter_map(parse_header)
        .collect()
}

fn parse_header(record: &str) -> Option<CommitHeader> {
    let mut fields = record.splitn(9, FIELD);
    let sha = fields.next()?.trim().to_owned();
    let parents = fields.next()?;
    let authored_date = fields.next()?.to_owned();
    let committed_date = fields.next()?.to_owned();
    let author_name = fields.next()?.to_owned();
    let author_email = fields.next()?.to_owned();
    let committer_name = fields.next()?.to_owned();
    let committer_email = fields.next()?.to_owned();
    let message = fields.next().unwrap_or_default().trim_end().to_owned();

    if sha.is_empty() {
        return None;
    }

    Some(CommitHeader {
        sha,
        committed_date,
        authored_date,
        author_name,
        author_email,
        committer_name,
        committer_email,
        parent_hashes: parents.split_whitespace().map(ToOwned::to_owned).collect(),
        message,
    })
}

/// `git patch-id` emits `<patch-id> <commit-sha>` per commit.
fn parse_patch_ids(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let patch_id = parts.next()?;
            let sha = parts.next()?;
            Some((sha.to_owned(), patch_id.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(sha: &str, parents: &str, committed: &str, message: &str) -> String {
        format!(
            "{RECORD}{sha}{FIELD}{parents}{FIELD}2026-08-01T09:00:00+00:00{FIELD}{committed}{FIELD}A{FIELD}a@example.com{FIELD}C{FIELD}c@example.com{FIELD}{message}"
        )
    }

    #[test]
    fn parses_all_fields_including_multiline_messages() {
        let text = record(
            "aaa",
            "bbb ccc",
            "2026-08-01T10:00:00+00:00",
            "subject line\n\nbody paragraph\n",
        );
        let headers = parse_headers(&text);

        assert_eq!(headers.len(), 1);
        let header = &headers[0];
        assert_eq!(header.sha, "aaa");
        assert_eq!(header.parent_hashes, vec!["bbb", "ccc"]);
        assert_eq!(header.committed_date, "2026-08-01T10:00:00+00:00");
        assert_eq!(header.author_email, "a@example.com");
        assert_eq!(header.committer_email, "c@example.com");
        assert_eq!(header.message, "subject line\n\nbody paragraph");
        assert!(header.is_merge(), "two parents is a merge");
    }

    #[test]
    fn root_commit_has_no_parents_and_is_not_a_merge() {
        let text = record("aaa", "", "2026-08-01T10:00:00+00:00", "init");
        let headers = parse_headers(&text);
        assert!(headers[0].parent_hashes.is_empty());
        assert!(!headers[0].is_merge());
    }

    #[test]
    fn skips_empty_and_malformed_records() {
        let cases = vec![
            ("empty text", String::new()),
            ("only separators", format!("{RECORD}{RECORD}")),
            ("too few fields", format!("{RECORD}aaa{FIELD}bbb")),
        ];
        for (name, text) in cases {
            assert!(parse_headers(&text).is_empty(), "must skip: {name}");
        }
    }

    #[test]
    fn parses_patch_id_pairs() {
        let text = "p1 aaa\np2 bbb\n";
        let ids = parse_patch_ids(text);
        assert_eq!(ids.get("aaa").map(String::as_str), Some("p1"));
        assert_eq!(ids.get("bbb").map(String::as_str), Some("p2"));
    }

    #[test]
    fn ignores_incomplete_patch_id_lines() {
        let ids = parse_patch_ids("only-one-field\n\n p2 bbb\n");
        assert_eq!(ids.len(), 1, "got: {ids:?}");
        assert_eq!(ids.get("bbb").map(String::as_str), Some("p2"));
    }
}
