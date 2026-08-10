use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::engine::key::CacheKey;
use crate::engine::page::PageToken;
use crate::engine::read::{self, Page, blobs, branches, commits, numstat, patches};
use crate::engine::runner::GitError;
use crate::engine::store::{Freshness, RepoGuard, StoreError};

use super::AppState;
use super::error::ApiError;
use super::request::{
    BadRequest, Paging, RequestContext, ShaFilter, clamp_patch_bytes, parse_sha_filter,
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Deserialize)]
pub struct CommitsQuery {
    repo: String,
    since: Option<String>,
    sha: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FileChangesQuery {
    repo: String,
    since: Option<String>,
    sha: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
    include_patch: Option<bool>,
    max_patch_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct BranchesQuery {
    repo: String,
    page_size: Option<u32>,
    page_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FileChangeRow {
    pub sha: String,
    pub committed_date: String,
    pub filename: String,
    pub previous_filename: Option<String>,
    pub status: &'static str,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub changes: Option<u64>,
    pub is_binary: bool,
    pub patch: Option<String>,
    pub patch_truncated: bool,
}

/// # Errors
///
/// [`ApiError`] on malformed input, origin failures, or a snapshot that moved
/// out from under a page cursor.
pub async fn list_commits(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<CommitsQuery>,
) -> Result<Response, ApiError> {
    let context = RequestContext::from_parts(&headers, &query.repo, state.clone_url_policy())?;
    let paging = Paging::parse(query.page_token.as_deref(), query.page_size)?;
    let selected = parse_sha_filter(query.sha.as_deref())?;

    let page = read_snapshot(&state, &context, &paging, |guard: RepoGuard| {
        let (state, context, paging) = (&state, &context, &paging);
        let since = query.since.as_deref();
        let selected = selected.as_ref();
        Box::pin(async move {
            let runner = state.store.runner();
            let all = commits::headers(runner, guard.git_dir(), since, &context.creds).await?;
            let (window, cursor) = read::slice_page(
                retain_selected(all, selected, |header| &header.sha),
                paging.token.as_ref(),
                paging.page_size,
                |header| (header.committed_date.clone(), header.sha.clone()),
            );

            let shas: Vec<String> = window.iter().map(|header| header.sha.clone()).collect();
            blobs::prefetch(runner, guard.git_dir(), &shas, &context.creds).await?;

            let file_stats = numstat::read(runner, guard.git_dir(), &shas, &context.creds).await?;
            let in_default =
                commits::default_branch_membership(runner, guard.git_dir(), &shas, &context.creds)
                    .await?;
            let ids = commits::patch_ids(runner, guard.git_dir(), &shas, &context.creds).await?;

            let items = window
                .into_iter()
                .map(|header| {
                    let files = file_stats.get(&header.sha).map_or(&[][..], Vec::as_slice);
                    let additions = files.iter().filter_map(|f| f.additions).sum();
                    let deletions = files.iter().filter_map(|f| f.deletions).sum();
                    commits::CommitRow {
                        is_merge: header.is_merge(),
                        changed_files: files.len() as u64,
                        additions,
                        deletions,
                        is_in_default_branch: in_default.contains(&header.sha),
                        patch_id: ids.get(&header.sha).cloned(),
                        sha: header.sha,
                        message: header.message,
                        authored_date: header.authored_date,
                        committed_date: header.committed_date,
                        author_name: header.author_name,
                        author_email: header.author_email,
                        committer_name: header.committer_name,
                        committer_email: header.committer_email,
                        parent_hashes: header.parent_hashes,
                    }
                })
                .collect();

            Ok(Page {
                items,
                next_page_token: encode_cursor(cursor, &context.key, guard.generation()),
            })
        })
    })
    .await?;

    json_page(page).await
}

/// # Errors
///
/// [`ApiError`] on malformed input, origin failures, or a snapshot that moved
/// out from under a page cursor.
pub async fn list_file_changes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<FileChangesQuery>,
) -> Result<Response, ApiError> {
    let context = RequestContext::from_parts(&headers, &query.repo, state.clone_url_policy())?;
    let paging = Paging::parse(query.page_token.as_deref(), query.page_size)?;
    let selected = parse_sha_filter(query.sha.as_deref())?;
    let include_patch = query.include_patch.unwrap_or(true);
    let max_patch_bytes = clamp_patch_bytes(query.max_patch_bytes);

    let page = read_snapshot(&state, &context, &paging, |guard: RepoGuard| {
        let (state, context, paging) = (&state, &context, &paging);
        let since = query.since.as_deref();
        let selected = selected.as_ref();
        Box::pin(async move {
            let runner = state.store.runner();
            let all = commits::headers(runner, guard.git_dir(), since, &context.creds).await?;
            // Parity with the CDK connectors: merge commits contribute no file rows.
            let non_merge: Vec<commits::CommitHeader> =
                retain_selected(all, selected, |header| &header.sha)
                    .into_iter()
                    .filter(|header| !header.is_merge())
                    .collect();
            let (window, cursor) = read::slice_page(
                non_merge,
                paging.token.as_ref(),
                paging.page_size,
                |header| (header.committed_date.clone(), header.sha.clone()),
            );

            let shas: Vec<String> = window.iter().map(|header| header.sha.clone()).collect();
            blobs::prefetch(runner, guard.git_dir(), &shas, &context.creds).await?;

            let file_stats = numstat::read(runner, guard.git_dir(), &shas, &context.creds).await?;
            let texts = if include_patch {
                patches::read(
                    runner,
                    guard.git_dir(),
                    &shas,
                    max_patch_bytes,
                    &context.creds,
                )
                .await?
            } else {
                HashMap::new()
            };

            let (items, early_cursor) =
                emit_file_changes(window, &file_stats, &texts, RowCaps::DEFAULT);

            Ok(Page {
                items,
                next_page_token: encode_cursor(
                    early_cursor.or(cursor),
                    &context.key,
                    guard.generation(),
                ),
            })
        })
    })
    .await?;

    json_page(page).await
}

/// Response-size bounds for `/v1/file-changes`. The page size bounds commits;
/// a commit fans out to one row per changed file, each carrying patch text, so
/// without these a single page can be arbitrarily large.
#[derive(Debug, Clone, Copy)]
struct RowCaps {
    max_rows: usize,
    max_patch_bytes: usize,
}

impl RowCaps {
    const DEFAULT: Self = Self {
        max_rows: 20_000,
        max_patch_bytes: 64 * 1024 * 1024,
    };
}

/// Fan commits out into file rows, stopping at a COMMIT boundary once a cap is
/// reached, and reporting the position of the last fully emitted commit.
///
/// INVARIANT: a commit is emitted whole or not at all. A commit that alone
/// exceeds a cap is emitted over budget rather than refused — otherwise the
/// caller could never advance past it and the repository would be permanently
/// unsyncable.
fn emit_file_changes(
    window: Vec<commits::CommitHeader>,
    file_stats: &HashMap<String, Vec<numstat::FileStat>>,
    texts: &HashMap<String, patches::CommitPatches>,
    caps: RowCaps,
) -> (Vec<FileChangeRow>, Option<(String, String)>) {
    let mut items: Vec<FileChangeRow> = Vec::new();
    let mut patch_bytes = 0usize;
    let mut last_complete: Option<(String, String)> = None;
    let mut stopped_early = false;

    for header in window {
        let files = file_stats.get(&header.sha).map_or(&[][..], Vec::as_slice);
        let rows = files.len();
        let bytes: usize = files
            .iter()
            .filter_map(|file| {
                texts
                    .get(&header.sha)
                    .and_then(|per_file| per_file.get(&file.filename))
                    .map(|patch| patch.text.len())
            })
            .sum();

        if !items.is_empty()
            && (items.len() + rows > caps.max_rows || patch_bytes + bytes > caps.max_patch_bytes)
        {
            stopped_early = true;
            break;
        }

        for file in files {
            let patch = texts
                .get(&header.sha)
                .and_then(|per_file| per_file.get(&file.filename));
            items.push(FileChangeRow {
                sha: header.sha.clone(),
                committed_date: header.committed_date.clone(),
                filename: file.filename.clone(),
                previous_filename: file.previous_filename.clone(),
                status: file.status.as_str(),
                additions: file.additions,
                deletions: file.deletions,
                changes: file.additions.and_then(|a| file.deletions.map(|d| a + d)),
                is_binary: file.is_binary,
                patch: patch.map(|p| p.text.clone()),
                patch_truncated: patch.is_some_and(|p| p.truncated),
            });
        }

        patch_bytes += bytes;
        last_complete = Some((header.committed_date, header.sha));
    }

    (items, stopped_early.then_some(last_complete).flatten())
}

/// # Errors
///
/// [`ApiError`] on malformed input or an origin failure.
pub async fn list_branches(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<BranchesQuery>,
) -> Result<Response, ApiError> {
    let context = RequestContext::from_parts(&headers, &query.repo, state.clone_url_policy())?;
    let paging = Paging::parse(query.page_token.as_deref(), query.page_size)?;

    let page = read_snapshot(&state, &context, &paging, |guard: RepoGuard| {
        let (state, context, paging) = (&state, &context, &paging);
        Box::pin(async move {
            let mut all =
                branches::read(state.store.runner(), guard.git_dir(), &context.creds).await?;
            // Branches have no date cursor, so the walk orders by name — the same
            // ascending-key contract the other endpoints use, which is what makes
            // the page token meaningful here too.
            all.sort_by(|a, b| a.name.cmp(&b.name));
            let (items, cursor) =
                read::slice_page(all, paging.token.as_ref(), paging.page_size, |row| {
                    (row.name.clone(), String::new())
                });

            Ok(Page {
                items,
                next_page_token: encode_cursor(cursor, &context.key, guard.generation()),
            })
        })
    })
    .await?;

    json_page(page).await
}

/// Read one page from a snapshot, healing an entry whose origin refuses to
/// serve explicitly requested objects.
///
/// Such an origin (a GitLab fork-network object pool, for instance) serves the
/// clone but not the blob prefetch that follows, and it does so on every
/// retry — so the entry is rebuilt once as a full clone and the read is tried
/// again. A continuation cannot span that rebuild: the promotion bumps the
/// generation its cursor is pinned to, which is the documented `409`.
async fn read_snapshot<'a, T, F>(
    state: &'a Arc<AppState>,
    context: &'a RequestContext,
    paging: &'a Paging,
    read: F,
) -> Result<Page<T>, ApiError>
where
    // INVARIANT: the reader takes the guard BY VALUE, so it is released before
    // promotion is requested. Promotion takes the entry's write lock, and
    // awaiting the write side while holding the read side deadlocks.
    F: Fn(RepoGuard) -> BoxFuture<'a, Result<Page<T>, ApiError>>,
{
    let guard = open(state, context, paging).await?;
    match read(guard).await {
        Ok(page) => return Ok(page),
        Err(e) if !refuses_promisor_wants(&e) => return Err(e),
        Err(_) => {}
    }

    let generation = state
        .store
        .promote_to_full_clone(&context.key, &context.creds)
        .await?;
    if paging.token.is_some() {
        return Err(StoreError::SnapshotChanged {
            current: generation,
        }
        .into());
    }

    let guard = open(state, context, paging).await?;
    read(guard).await
}

fn refuses_promisor_wants(error: &ApiError) -> bool {
    matches!(
        error,
        ApiError::Git(GitError::PromisorRefused) | ApiError::Store(StoreError::PromisorRefused)
    )
}

/// Resolve the snapshot: a first page honors fetch-if-stale, a continuation is
/// pinned to the generation its token carries.
///
/// INVARIANT: every paginated endpoint resolves its snapshot here, so this is
/// the one place a continuation token is checked against the repository it was
/// minted for.
async fn open(
    state: &Arc<AppState>,
    context: &RequestContext,
    paging: &Paging,
) -> Result<RepoGuard, ApiError> {
    let freshness = match paging.token.as_ref() {
        Some(token) => {
            if !token.binds_to(&context.key) {
                // A cursor from another repository is indistinguishable, from
                // the caller's side, from a corrupted one. Not 409: the
                // connector answers that by resuming from its stored cursor,
                // which would loop forever on a permanently foreign token.
                return Err(BadRequest::MalformedToken.into());
            }
            Freshness::Pinned {
                generation: token.generation,
            }
        }
        None => Freshness::Refresh {
            max_staleness: context.max_staleness.unwrap_or(Duration::from_secs(
                state.config.default_max_staleness_seconds,
            )),
        },
    };
    let guard = state
        .store
        .open(&context.key, &context.creds, freshness)
        .await?;
    Ok(guard)
}

/// Serialize off the reactor: a page carries up to ten thousand commit
/// messages, or the patch text of every file they touched.
async fn json_page<T>(page: Page<T>) -> Result<Response, ApiError>
where
    T: Serialize + Send + 'static,
{
    let body = tokio::task::spawn_blocking(move || serde_json::to_vec(&page))
        .await
        .map_err(|e| ApiError::Serialization(e.to_string()))?
        .map_err(|e| ApiError::Serialization(e.to_string()))?;

    Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
}

fn retain_selected<T, K>(rows: Vec<T>, selected: Option<&ShaFilter>, key: K) -> Vec<T>
where
    K: Fn(&T) -> &str,
{
    match selected {
        Some(filter) => rows
            .into_iter()
            .filter(|row| filter.matches(key(row)))
            .collect(),
        None => rows,
    }
}

fn encode_cursor(
    cursor: Option<(String, String)>,
    key: &CacheKey,
    generation: u64,
) -> Option<String> {
    cursor.map(|(primary, secondary)| {
        PageToken {
            entry: PageToken::binding_for(key),
            generation,
            primary,
            secondary,
        }
        .encode()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::read::numstat::{FileStat, FileStatus};
    use crate::engine::read::patches::Patch;

    fn header(sha: &str, date: &str) -> commits::CommitHeader {
        commits::CommitHeader {
            sha: sha.to_owned(),
            committed_date: date.to_owned(),
            authored_date: date.to_owned(),
            author_name: "a".to_owned(),
            author_email: "a@example.com".to_owned(),
            committer_name: "c".to_owned(),
            committer_email: "c@example.com".to_owned(),
            parent_hashes: Vec::new(),
            message: "m".to_owned(),
        }
    }

    fn stat(name: &str) -> FileStat {
        FileStat {
            filename: name.to_owned(),
            previous_filename: None,
            status: FileStatus::Modified,
            additions: Some(1),
            deletions: Some(1),
            is_binary: false,
        }
    }

    type Scenario = (
        Vec<commits::CommitHeader>,
        HashMap<String, Vec<FileStat>>,
        HashMap<String, patches::CommitPatches>,
    );

    /// `count` commits, each touching `files_each` files whose patch text is
    /// `patch_bytes` long.
    fn scenario(count: usize, files_each: usize, patch_bytes: usize) -> Scenario {
        let mut window = Vec::new();
        let mut stats = HashMap::new();
        let mut texts: HashMap<String, patches::CommitPatches> = HashMap::new();

        for c in 0..count {
            let sha = format!("sha{c:04}");
            window.push(header(&sha, &format!("2026-08-{:02}T00:00:00Z", c + 1)));

            let files: Vec<FileStat> = (0..files_each).map(|f| stat(&format!("f{f}"))).collect();
            let per_file: patches::CommitPatches = files
                .iter()
                .map(|file| {
                    (
                        file.filename.clone(),
                        Patch {
                            text: "x".repeat(patch_bytes),
                            truncated: false,
                        },
                    )
                })
                .collect();

            stats.insert(sha.clone(), files);
            texts.insert(sha, per_file);
        }
        (window, stats, texts)
    }

    #[test]
    fn an_unbounded_page_keeps_every_row_and_no_early_cursor() {
        let (window, stats, texts) = scenario(3, 2, 4);
        let (rows, cursor) = emit_file_changes(window, &stats, &texts, RowCaps::DEFAULT);

        assert_eq!(rows.len(), 6);
        assert_eq!(cursor, None, "nothing was withheld, so nothing to resume");
    }

    #[test]
    fn stops_at_a_commit_boundary_when_the_row_cap_is_hit() {
        let (window, stats, texts) = scenario(10, 3, 1);
        let caps = RowCaps {
            max_rows: 7,
            max_patch_bytes: usize::MAX,
        };
        let (rows, cursor) = emit_file_changes(window, &stats, &texts, caps);

        assert_eq!(rows.len(), 6, "two whole commits fit, the third would not");
        assert_eq!(
            cursor.map(|(_, sha)| sha),
            Some("sha0001".to_owned()),
            "the cursor names the last commit emitted in full"
        );
    }

    #[test]
    fn stops_at_a_commit_boundary_when_the_patch_byte_cap_is_hit() {
        let (window, stats, texts) = scenario(10, 1, 100);
        let caps = RowCaps {
            max_rows: usize::MAX,
            max_patch_bytes: 250,
        };
        let (rows, cursor) = emit_file_changes(window, &stats, &texts, caps);

        assert_eq!(
            rows.len(),
            2,
            "two commits of 100 bytes fit, a third does not"
        );
        assert_eq!(cursor.map(|(_, sha)| sha), Some("sha0001".to_owned()));
    }

    #[test]
    fn no_commit_is_ever_half_emitted() {
        let (window, stats, texts) = scenario(10, 3, 10);
        let caps = RowCaps {
            max_rows: 8,
            max_patch_bytes: 95,
        };
        let (rows, _) = emit_file_changes(window, &stats, &texts, caps);

        let mut per_sha: HashMap<&str, usize> = HashMap::new();
        for row in &rows {
            *per_sha.entry(row.sha.as_str()).or_default() += 1;
        }
        for (sha, count) in per_sha {
            assert_eq!(count, 3, "commit {sha} was emitted partially");
        }
    }

    #[test]
    fn a_single_oversized_commit_is_emitted_whole_so_the_walk_can_advance() {
        let (window, stats, texts) = scenario(2, 50, 1000);
        let caps = RowCaps {
            max_rows: 1,
            max_patch_bytes: 1,
        };
        let (rows, cursor) = emit_file_changes(window, &stats, &texts, caps);

        assert_eq!(
            rows.len(),
            50,
            "the first commit must be served over budget, or it can never be passed"
        );
        assert_eq!(
            cursor.map(|(_, sha)| sha),
            Some("sha0000".to_owned()),
            "and the caller must be told where to resume"
        );
    }

    #[test]
    fn an_empty_window_emits_nothing() {
        let (rows, cursor) = emit_file_changes(
            Vec::new(),
            &HashMap::new(),
            &HashMap::new(),
            RowCaps::DEFAULT,
        );
        assert!(rows.is_empty());
        assert_eq!(cursor, None);
    }
}
