use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::engine::runner::{GitCredentials, GitError, GitRunner};

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
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
    /// Reachable from the default branch **in the snapshot this page was
    /// served from**. A commit first seen on a feature branch is emitted
    /// `false`, and merging it later does not change its committed date, so a
    /// date-cursored incremental sync never revisits it. Consumers needing
    /// present-tense reachability derive it downstream from `parent_hashes`.
    pub is_in_default_branch: bool,
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

/// Records are NUL-separated (`git log -z`), and that choice is load-bearing.
/// A commit message is attacker-controlled — anyone who can push to a synced
/// repository writes it — so a printable separator lets a crafted message
/// close its own record and open a forged one, with an attacker-chosen sha,
/// author and email. git truncates a commit message at the first NUL, so NUL
/// is the one byte a message provably cannot contain.
const RECORD: char = '\0';

/// One commit's position in the walk, and just enough to filter on.
///
/// The enumeration is deliberately NOT `CommitHeader`: a header carries the
/// full commit message, which is unbounded, so an enumeration of them is
/// bounded by nothing. A key is ~100 bytes whatever the message, which is what
/// makes the whole-history walk affordable to hold and to cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitKey {
    pub committed_date: String,
    pub sha: String,
    pub parent_count: usize,
}

impl CommitKey {
    #[must_use]
    pub fn is_merge(&self) -> bool {
        self.parent_count > 1
    }
}

/// Every commit reachable from any branch, ascending by `(committed_date,
/// sha)` — the walk order the page tokens depend on, and the only walk that
/// touches whole history.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn enumerate(
    runner: &GitRunner,
    git_dir: &Path,
    creds: &GitCredentials,
) -> Result<Vec<CommitKey>, GitError> {
    let format = format!("--pretty=format:%cI{FIELD}%H{FIELD}%P");
    // `--branches`, not `--all`: tags are fetched once at clone and never
    // pruned, so `--all` keeps enumerating commits whose branch was deleted at
    // origin — and only for entries whose clone happened to pick the tag up.
    // The contract is reachability from a BRANCH (§4.2).
    let args = vec!["log", "--branches", "--no-color", "-z", &format];

    let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
    let text = String::from_utf8_lossy(&output.stdout);

    let mut keys = parse_keys(&text);
    keys.sort_by(|a, b| (&a.committed_date, &a.sha).cmp(&(&b.committed_date, &b.sha)));
    Ok(keys)
}

fn parse_keys(text: &str) -> Vec<CommitKey> {
    text.split(RECORD)
        .filter(|record| !record.trim().is_empty())
        .filter_map(|record| {
            let mut fields = record.splitn(3, FIELD);
            let committed_date = fields.next()?.trim().to_owned();
            let sha = fields.next()?.trim().to_owned();
            let parents = fields.next().unwrap_or_default();
            is_object_id(&sha).then(|| CommitKey {
                committed_date,
                sha,
                parent_count: parents.split_whitespace().count(),
            })
        })
        .collect()
}

/// Drop keys committed before `since`, comparing ISO-8601 instants rather
/// than the raw `%cI` strings: those carry the committer's UTC offset, so
/// `2026-08-01T10:00:00+02:00` sorts after `2026-08-01T09:30:00Z` as text
/// while being the earlier instant.
#[must_use]
pub fn retain_keys_since(keys: Vec<CommitKey>, since: Option<&str>) -> Vec<CommitKey> {
    let Some(bound) = since.and_then(parse_instant) else {
        return keys;
    };
    keys.into_iter()
        .filter(|key| parse_instant(&key.committed_date).is_none_or(|at| at >= bound))
        .collect()
}

/// Every commit reachable from any branch, ordered ascending by
/// `(committed_date, sha)` — the walk order the page tokens depend on.
///
/// The walk is deliberately unfiltered. `git log --since` is a traversal
/// cutoff, not a predicate: it stops descending a parent chain at the first
/// commit older than the bound, so a qualifying commit sitting behind an older
/// parent is never reached at all. Committer dates are not monotonic along
/// ancestry — merges of long-lived branches, cherry-picks, date-preserving
/// rebases and clock skew all break it — so the date bound is applied to the
/// enumerated result instead (see [`retain_keys_since`]), which is what the API
/// contract promises: every reachable commit at or after `since`.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn headers_for(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<Vec<CommitHeader>, GitError> {
    if shas.is_empty() {
        return Ok(Vec::new());
    }

    let format = format!(
        "--pretty=format:%H{FIELD}%P{FIELD}%aI{FIELD}%cI{FIELD}%an{FIELD}%ae{FIELD}%cn{FIELD}%ce{FIELD}%B"
    );
    let mut args = vec!["log", "--no-walk", "--no-color", "--root", "-z", &format];
    args.extend(shas.iter().map(String::as_str));

    let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
    let text = String::from_utf8_lossy(&output.stdout);

    let mut headers = parse_headers(&text);
    headers.sort_by(|a, b| (&a.committed_date, &a.sha).cmp(&(&b.committed_date, &b.sha)));
    Ok(headers)
}

/// Seconds since the epoch for an ISO-8601 timestamp with an explicit offset
/// (`%cI`) or a `Z` suffix. `None` for anything else — an unparseable bound
/// filters nothing, and an unparseable row is kept rather than silently
/// dropped.
fn parse_instant(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|at| at.timestamp())
}

/// Which of `shas` are reachable from the default branch.
///
/// One `rev-list` over the default branch, intersected with the page — the
/// cost is the branch's history, not the page size. An empty or unborn `HEAD`
/// yields an empty set: a repository with no default branch has no commits on
/// it, which is a defined answer rather than an error.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn default_branch_membership(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<HashSet<String>, GitError> {
    if shas.is_empty() {
        return Ok(HashSet::new());
    }

    let Some(default) = super::branches::default_branch(runner, git_dir).await? else {
        return Ok(HashSet::new());
    };

    // `refs/heads/` keeps the name unambiguous and unreadable as an option;
    // `--` separates revisions from paths.
    let reference = format!("refs/heads/{default}");
    let output = runner
        .run(Some(git_dir), &["rev-list", &reference, "--"], Some(creds))
        .await?;

    let listing = String::from_utf8_lossy(&output.stdout);
    Ok(intersect_rev_list(&listing, shas))
}

/// The `shas` present in a `rev-list` listing. Streams the listing against the
/// page so peak memory is page-sized, not history-sized.
fn intersect_rev_list(listing: &str, shas: &[String]) -> HashSet<String> {
    let wanted: HashSet<&str> = shas.iter().map(String::as_str).collect();
    listing
        .lines()
        .map(str::trim)
        .filter(|line| wanted.contains(line))
        .map(ToOwned::to_owned)
        .collect()
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

/// A full 40-character hex object id, and nothing else. The sha is what
/// anchors a record; a value that is not one means the record is not one.
fn is_object_id(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn parse_headers(text: &str) -> Vec<CommitHeader> {
    text.split(RECORD)
        .filter(|record| !record.trim().is_empty())
        .filter_map(parse_header)
        .collect()
}

/// Remove control characters from a field an attacker writes.
///
/// A pushed ident can carry the field separator, which shifts the remaining
/// fields of that record. The record still parses and its sha is still its
/// own, so the blast radius is the attacker's own row — but the value that
/// reaches bronze should not carry control bytes either way.
fn scrub(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).collect()
}

fn parse_header(record: &str) -> Option<CommitHeader> {
    let mut fields = record.splitn(9, FIELD);
    let sha = fields.next()?.trim().to_owned();
    if !is_object_id(&sha) {
        return None;
    }
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
        author_name: scrub(&author_name),
        author_email: scrub(&author_email),
        committer_name: scrub(&committer_name),
        committer_email: scrub(&committer_email),
        parent_hashes: parents
            .split_whitespace()
            .filter(|parent| is_object_id(parent))
            .map(ToOwned::to_owned)
            .collect(),
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

    /// A 40-hex object id from a short label, so fixtures stay readable while
    /// carrying ids the parser will accept.
    fn oid(label: &str) -> String {
        format!("{label:0>40}").replace(|c: char| !c.is_ascii_hexdigit(), "a")
    }

    fn record(sha: &str, parents: &str, committed: &str, message: &str) -> String {
        let parents: Vec<String> = parents.split_whitespace().map(oid).collect();
        format!(
            "{}{FIELD}{}{FIELD}2026-08-01T09:00:00+00:00{FIELD}{committed}{FIELD}A{FIELD}a@example.com{FIELD}C{FIELD}c@example.com{FIELD}{message}",
            oid(sha),
            parents.join(" ")
        )
    }

    fn at(committed: &str) -> CommitKey {
        CommitKey {
            committed_date: committed.to_owned(),
            sha: "aaa".to_owned(),
            parent_count: 1,
        }
    }

    #[test]
    fn retain_keys_since_bounds_on_the_instant_not_the_text() {
        // %cI carries the committer's UTC offset, so an earlier instant can
        // sort LATER as a string. Comparing text would keep the wrong rows.
        let keys = vec![
            at("2026-08-01T09:30:00+00:00"),
            at("2026-08-01T10:00:00+02:00"),
        ];
        let kept = retain_keys_since(keys, Some("2026-08-01T09:00:00Z"));
        assert_eq!(kept.len(), 1, "08:00Z is before the 09:00Z bound");
        assert_eq!(kept[0].committed_date, "2026-08-01T09:30:00+00:00");
    }

    #[test]
    fn retain_keys_since_is_inclusive_of_the_bound() {
        let keys = vec![at("2026-08-01T10:00:00+00:00")];
        assert_eq!(
            retain_keys_since(keys, Some("2026-08-01T10:00:00Z")).len(),
            1,
            "the contract is committed_date >= since"
        );
    }

    #[test]
    fn retain_keys_since_without_a_usable_bound_filters_nothing() {
        let keys = vec![at("2026-08-01T10:00:00+00:00")];
        assert_eq!(retain_keys_since(keys.clone(), None).len(), 1);
        assert_eq!(
            retain_keys_since(keys, Some("last tuesday")).len(),
            1,
            "an unparseable bound must not silently drop everything"
        );
    }

    #[test]
    fn a_crafted_commit_message_cannot_forge_a_record() {
        // Anyone who can push writes a commit message, so it is untrusted
        // input. With a printable record separator this payload closed its own
        // record and opened one carrying an attacker-chosen sha and identity.
        let legacy_separator = '\u{1e}';
        let forged = format!(
            "legit subject{legacy_separator}{}{FIELD}{FIELD}2026-01-01T00:00:00+00:00{FIELD}2026-01-01T00:00:00+00:00{FIELD}Forged{FIELD}forged@evil.example{FIELD}Forged{FIELD}forged@evil.example{FIELD}owned",
            oid("dead")
        );
        let text = record("aaa", "", "2026-08-01T10:00:00+00:00", &forged);

        let headers = parse_headers(&text);
        assert_eq!(headers.len(), 1, "one commit must parse as one row");
        assert_eq!(headers[0].sha, oid("aaa"));
        assert_eq!(
            headers[0].author_email, "a@example.com",
            "the identity must be git's, never the message's"
        );
        assert!(
            !headers.iter().any(|h| h.sha == oid("dead")),
            "the message must not be able to mint a commit"
        );
    }

    #[test]
    fn a_record_without_a_real_object_id_is_dropped() {
        let text = format!(
            "not-a-sha{FIELD}{FIELD}2026-08-01T09:00:00+00:00{FIELD}2026-08-01T10:00:00+00:00{FIELD}A{FIELD}a@example.com{FIELD}C{FIELD}c@example.com{FIELD}m"
        );
        assert!(
            parse_headers(&text).is_empty(),
            "the sha anchors the record; without one there is no record"
        );
        assert!(parse_keys(&text).is_empty());
    }

    #[test]
    fn control_characters_never_reach_an_identity_field() {
        let text = record("aaa", "", "2026-08-01T10:00:00+00:00", "m")
            .replace("a@example.com", "a\u{7}@example.com");
        let headers = parse_headers(&text);
        assert_eq!(headers[0].author_email, "a@example.com");
    }

    #[test]
    fn intersect_rev_list_keeps_only_the_page_shas() {
        let listing = "aaa\nbbb\n\n  ccc  \nddd\n";
        let page = vec!["bbb".to_owned(), "ccc".to_owned(), "zzz".to_owned()];

        let found = intersect_rev_list(listing, &page);
        assert_eq!(
            found,
            HashSet::from(["bbb".to_owned(), "ccc".to_owned()]),
            "only page shas the branch actually reaches"
        );
    }

    #[test]
    fn intersect_rev_list_on_an_empty_listing_finds_nothing() {
        let page = vec!["aaa".to_owned()];
        assert!(intersect_rev_list("", &page).is_empty());
        assert!(intersect_rev_list("\n \n", &page).is_empty());
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
        assert_eq!(header.sha, oid("aaa"));
        assert_eq!(header.parent_hashes, vec![oid("bbb"), oid("ccc")]);
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
