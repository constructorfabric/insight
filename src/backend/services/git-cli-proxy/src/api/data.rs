use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::engine::key::CacheKey;
use crate::engine::page::PageToken;
use crate::engine::read::{self, Page, authors, branches, commits, numstat, patches};
use crate::engine::runner::GitError;
use crate::engine::store::{Freshness, RepoGuard, StoreError};

use super::AppState;
use super::error::ApiError;
use super::request::{
    BadRequest, Paging, RequestContext, ShaFilter, ValidatedQuery, clamp_patch_bytes,
    parse_sha_filter, required_param,
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Deserialize)]
pub struct CommitsQuery {
    repo: Option<String>,
    since: Option<String>,
    sha: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FileChangesQuery {
    repo: Option<String>,
    since: Option<String>,
    sha: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
    include_patch: Option<bool>,
    max_patch_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct BranchesQuery {
    repo: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuthorsQuery {
    repo: Option<String>,
    since: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

/// Concrete page wrappers, one per endpoint: `Page<T>` cannot be a schema
/// because the registry keys components on the type's own name, so all three
/// instantiations would collide on one component.
#[derive(Debug, Serialize, ToSchema)]
pub struct CommitsPage {
    pub items: Vec<commits::CommitRow>,
    pub next_page_token: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for CommitsPage {}

#[derive(Debug, Serialize, ToSchema)]
pub struct FileChangesPage {
    pub items: Vec<FileChangeRow>,
    pub next_page_token: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for FileChangesPage {}

#[derive(Debug, Serialize, ToSchema)]
pub struct BranchesPage {
    pub items: Vec<branches::BranchRow>,
    pub next_page_token: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for BranchesPage {}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorsPage {
    pub items: Vec<authors::AuthorRow>,
    pub next_page_token: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for AuthorsPage {}

impl From<Page<commits::CommitRow>> for CommitsPage {
    fn from(page: Page<commits::CommitRow>) -> Self {
        Self {
            items: page.items,
            next_page_token: page.next_page_token,
        }
    }
}

impl From<Page<FileChangeRow>> for FileChangesPage {
    fn from(page: Page<FileChangeRow>) -> Self {
        Self {
            items: page.items,
            next_page_token: page.next_page_token,
        }
    }
}

impl From<Page<branches::BranchRow>> for BranchesPage {
    fn from(page: Page<branches::BranchRow>) -> Self {
        Self {
            items: page.items,
            next_page_token: page.next_page_token,
        }
    }
}

impl From<Page<authors::AuthorRow>> for AuthorsPage {
    fn from(page: Page<authors::AuthorRow>) -> Self {
        Self {
            items: page.items,
            next_page_token: page.next_page_token,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FileChangeRow {
    pub sha: String,
    pub committed_date: String,
    pub filename: String,
    pub previous_filename: Option<String>,
    #[schema(value_type = String)]
    pub status: &'static str,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub changes: Option<u64>,
    pub is_binary: bool,
    pub pre_image_oid: Option<String>,
    pub post_image_oid: Option<String>,
    pub patch: Option<String>,
    pub patch_truncated: bool,
}

/// # Errors
///
/// [`ApiError`] on malformed input, origin failures, or a snapshot that moved
/// out from under a page cursor.
pub async fn list_commits(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedQuery(query): ValidatedQuery<CommitsQuery>,
) -> Result<Response, ApiError> {
    let repo = required_param(query.repo.as_deref(), "repo")?;
    let context = RequestContext::from_parts(&headers, repo, state.clone_url_policy())?;
    let paging = Paging::parse(query.page_token.as_deref(), query.page_size)?;
    let selected = parse_sha_filter(query.sha.as_deref())?;

    let page = read_snapshot(&state, &context, &paging, |guard: RepoGuard| {
        let (state, context, paging) = (&state, &context, &paging);
        let since = query.since.as_deref();
        let selected = selected.as_ref();
        Box::pin(async move {
            let runner = state.store.runner();
            let (keys, cursor, indexed_membership) =
                page_of_keys(state, &guard, context, paging, since, selected, true).await?;

            let shas: Vec<String> = keys.iter().map(|key| key.sha.clone()).collect();
            // Only the page's own commits are read in full: a header carries
            // the whole commit message, so reading them for all of history
            // would dwarf everything else on the request. Independent of the
            // prefetch — headers touch commit objects only — so the two run
            // concurrently, as do the diff readers behind them.
            let (window, _fetched) = tokio::try_join!(
                commits::headers_for(runner, guard.git_dir(), &shas, &context.creds),
                state
                    .store
                    .prefetch_window(&context.key, guard.git_dir(), &shas, &context.creds),
            )?;

            let (file_stats, ids, in_default) = tokio::try_join!(
                numstat::totals(runner, guard.git_dir(), &shas, &context.creds),
                commits::patch_ids(runner, guard.git_dir(), &shas, &context.creds),
                async {
                    match indexed_membership {
                        Some(membership) => Ok(membership),
                        None => {
                            commits::default_branch_membership(
                                runner,
                                guard.git_dir(),
                                &shas,
                                &context.creds,
                            )
                            .await
                        }
                    }
                },
            )?;

            let items = window
                .into_iter()
                .map(|header| {
                    let totals = file_stats.get(&header.sha).copied().unwrap_or_default();
                    commits::CommitRow {
                        is_merge: header.is_merge(),
                        changed_files: totals.changed_files,
                        additions: totals.additions,
                        deletions: totals.deletions,
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
                next_page_token: encode_cursor(cursor, &context.key, &guard),
            })
        })
    })
    .await?;

    json_page(CommitsPage::from(page)).await
}

/// # Errors
///
/// [`ApiError`] on malformed input, origin failures, or a snapshot that moved
/// out from under a page cursor.
pub async fn list_file_changes(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedQuery(query): ValidatedQuery<FileChangesQuery>,
) -> Result<Response, ApiError> {
    let repo = required_param(query.repo.as_deref(), "repo")?;
    let context = RequestContext::from_parts(&headers, repo, state.clone_url_policy())?;
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
            // Parity with the CDK connectors: merge commits contribute no file rows.
            let (window, cursor, _membership) =
                page_of_keys(state, &guard, context, paging, since, selected, false).await?;

            let shas: Vec<String> = window.iter().map(|key| key.sha.clone()).collect();
            state
                .store
                .prefetch_window(&context.key, guard.git_dir(), &shas, &context.creds)
                .await?;

            // Independent after the prefetch: numstat and patch text read the
            // same blobs, neither needs the other.
            let (file_stats, mut texts) = tokio::try_join!(
                numstat::read(
                    runner,
                    guard.git_dir(),
                    &shas,
                    RowCaps::DEFAULT.max_rows,
                    &context.creds,
                ),
                async {
                    if include_patch {
                        patches::read(
                            runner,
                            guard.git_dir(),
                            &shas,
                            max_patch_bytes,
                            RowCaps::DEFAULT.max_patch_bytes,
                            &context.creds,
                        )
                        .await
                    } else {
                        Ok(HashMap::new())
                    }
                },
            )?;

            let (items, early_cursor) =
                emit_file_changes(window, &file_stats, &mut texts, RowCaps::DEFAULT);

            Ok(Page {
                items,
                next_page_token: encode_cursor(early_cursor.or(cursor), &context.key, &guard),
            })
        })
    })
    .await?;

    json_page(FileChangesPage::from(page)).await
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
    window: Vec<commits::CommitKey>,
    file_stats: &HashMap<String, Vec<numstat::FileStat>>,
    texts: &mut HashMap<String, patches::CommitPatches>,
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

        // Consumed, not borrowed: every `(sha, file)` patch is emitted once,
        // and a page can hold the whole patch budget — keeping the map alive
        // alongside the rows doubles peak memory for nothing.
        let mut per_file = texts.remove(&header.sha).unwrap_or_default();
        for file in files {
            let patch = per_file.remove(&file.filename);
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
                pre_image_oid: file.pre_image_oid.clone(),
                post_image_oid: file.post_image_oid.clone(),
                patch_truncated: patch.as_ref().is_some_and(|p| p.truncated),
                patch: patch.map(|p| p.text),
            });
        }

        patch_bytes += bytes;
        // INVARIANT: the cursor is minted in the coordinate system the walk is
        // ORDERED by — the normalised ordinal, never the raw `%cI`. `%cI`
        // carries the committer's own UTC offset, so a raw cursor compares
        // against later pages' ordinals as plain text and silently skips every
        // commit between the two spellings of the same instant.
        last_complete = Some((header.ordinal, header.sha));
    }

    (items, stopped_early.then_some(last_complete).flatten())
}

/// # Errors
///
/// [`ApiError`] on malformed input or an origin failure.
pub async fn list_branches(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedQuery(query): ValidatedQuery<BranchesQuery>,
) -> Result<Response, ApiError> {
    let repo = required_param(query.repo.as_deref(), "repo")?;
    let context = RequestContext::from_parts(&headers, repo, state.clone_url_policy())?;
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
                next_page_token: encode_cursor(cursor, &context.key, &guard),
            })
        })
    })
    .await?;

    json_page(BranchesPage::from(page)).await
}

/// `GET /v1/authors` — one row per distinct commit author.
///
/// # Errors
///
/// [`ApiError`] on malformed input or an origin failure.
pub async fn list_authors(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedQuery(query): ValidatedQuery<AuthorsQuery>,
) -> Result<Response, ApiError> {
    let repo = required_param(query.repo.as_deref(), "repo")?;
    let context = RequestContext::from_parts(&headers, repo, state.clone_url_policy())?;
    let paging = Paging::parse(query.page_token.as_deref(), query.page_size)?;

    let page = read_snapshot(&state, &context, &paging, |guard: RepoGuard| {
        let (state, context, paging) = (&state, &context, &paging);
        let since = query.since.as_deref();
        Box::pin(async move {
            // Already sorted by e-mail, which is the ascending key the cursor
            // pages on — an author has no date to order by, since the walk
            // collapses every commit they wrote into one row.
            let all =
                authors::read(state.store.runner(), guard.git_dir(), &context.creds, since).await?;
            let (items, cursor) =
                read::slice_page(all, paging.token.as_ref(), paging.page_size, |row| {
                    (row.author_email.clone(), String::new())
                });

            Ok(Page {
                items,
                next_page_token: encode_cursor(cursor, &context.key, &guard),
            })
        })
    })
    .await?;

    json_page(AuthorsPage::from(page)).await
}

/// One page of commit keys, its cursor, and — when the index answered — the
/// page's default-branch membership.
///
/// The index is a per-generation cache of the two whole-history walks; the
/// walks stay as the fallback so an entry cloned before indexes existed, or
/// one whose build failed, serves correctly at the old cost. Both paths MUST
/// apply the same filters in the same order — the parity test in
/// `engine::read` is what holds them together.
async fn page_of_keys(
    state: &Arc<AppState>,
    guard: &RepoGuard,
    context: &RequestContext,
    paging: &Paging,
    since: Option<&str>,
    selected: Option<&ShaFilter>,
    merges: bool,
) -> Result<
    (
        Vec<commits::CommitKey>,
        Option<(String, String)>,
        Option<HashSet<String>>,
    ),
    ApiError,
> {
    let query = crate::engine::index::PageQuery {
        since_epoch: since.and_then(commits::parse_instant),
        after: paging.token.clone(),
        page_size: paging.page_size,
        merges,
        sha_prefixes: selected.map(ShaFilter::prefixes),
    };
    let git_dir = guard.git_dir().to_path_buf();
    let generation = guard.generation();
    let indexed = tokio::task::spawn_blocking(move || {
        crate::engine::index::read_page(&git_dir, generation, &query)
    })
    .await
    .map_err(|e| ApiError::Store(StoreError::Io(std::io::Error::other(e))))?;

    match indexed {
        Ok(Some((rows, cursor))) => {
            let membership = crate::engine::index::membership_of(&rows);
            let keys = rows.into_iter().map(|row| row.key).collect();
            return Ok((keys, cursor, Some(membership)));
        }
        Ok(None) => {}
        Err(e) => {
            // A corrupt index must not take the endpoint down with it; the
            // walk still knows the truth.
            tracing::warn!(error = %e, "page index unreadable; falling back to the live walk");
        }
    }

    let all = commits::enumerate(state.store.runner(), guard.git_dir(), &context.creds).await?;
    let all = commits::retain_keys_since(all, since);
    let all = retain_selected(all, selected, |key| &key.sha);
    let all: Vec<commits::CommitKey> = if merges {
        all
    } else {
        all.into_iter().filter(|key| !key.is_merge()).collect()
    };
    let (keys, cursor) = read::slice_page(all, paging.token.as_ref(), paging.page_size, |key| {
        (key.ordinal.clone(), key.sha.clone())
    });
    Ok((keys, cursor, None))
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
    let outcome = read(guard).await;
    check_for_drift(state, context);
    match outcome {
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
    let outcome = read(guard).await;
    check_for_drift(state, context);
    outcome
}

/// Hand the entry to the store's post-serve purge, detached.
///
/// INVARIANT: the reader's guard is already dropped when this is called. The
/// purge probes the write side without waiting, so calling it any earlier
/// would simply find the entry busy and do nothing.
fn check_for_drift(state: &Arc<AppState>, context: &RequestContext) {
    let store = Arc::clone(&state.store);
    let key = context.key.clone();
    tokio::spawn(async move { store.purge_if_drifted(&key).await });
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
                incarnation: token.incarnation.clone(),
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
async fn json_page<T>(page: T) -> Result<Response, ApiError>
where
    T: Serialize + Send + 'static,
{
    let body = tokio::task::spawn_blocking(move || serde_json::to_vec(&page))
        .await
        .map_err(|e| ApiError::Serialization(e.to_string()))?
        .map_err(|e| ApiError::Serialization(e.to_string()))?;

    // Set explicitly, not left to the wire layer: the metrics middleware reads
    // it off the response, long before hyper would compute one.
    let length = body.len();
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_owned()),
            (header::CONTENT_LENGTH, length.to_string()),
        ],
        body,
    )
        .into_response())
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
    guard: &RepoGuard,
) -> Option<String> {
    cursor.map(|(primary, secondary)| {
        PageToken {
            entry: PageToken::binding_for(key),
            generation: guard.generation(),
            incarnation: guard.incarnation().to_owned(),
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

    #[tokio::test]
    async fn a_page_declares_its_own_length() {
        // The response-size histogram reads this header. Left to the wire
        // layer it is absent here, and every page is recorded as zero bytes.
        let page = BranchesPage {
            items: vec![branches::BranchRow {
                name: "main".to_owned(),
                head_sha: "0".repeat(40),
                head_committed_date: "2026-08-01T10:00:00+00:00".to_owned(),
                is_default: true,
            }],
            next_page_token: None,
        };
        let Ok(response) = json_page(page).await else {
            panic!("a page must serialize")
        };

        let declared: usize = match response.headers().get(header::CONTENT_LENGTH) {
            Some(value) => match value.to_str().ok().and_then(|v| v.parse().ok()) {
                Some(parsed) => parsed,
                None => panic!("content-length must be a number: {value:?}"),
            },
            None => panic!("a page must declare its length"),
        };
        let Ok(body) = axum::body::to_bytes(response.into_body(), usize::MAX).await else {
            panic!("body must be readable")
        };
        assert_eq!(declared, body.len(), "the declared length must be the body");
    }

    fn header(sha: &str, date: &str) -> commits::CommitKey {
        commits::CommitKey {
            sha: sha.to_owned(),
            ordinal: commits::ordinal_of(date),
            committed_date: date.to_owned(),
            parent_count: 1,
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
            pre_image_oid: None,
            post_image_oid: None,
        }
    }

    type Scenario = (
        Vec<commits::CommitKey>,
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
    fn an_early_cursor_does_not_skip_the_commit_that_follows_it() {
        // The cursor commit is written `10:00+02:00` — 08:00Z — and the next
        // commit is 09:00Z, so it genuinely comes later. Minting the cursor
        // from the raw `%cI` makes the next page compare `09:00…Z` against the
        // text `10:00+02:00`, decide it does not follow, and drop it: silent,
        // permanent loss for any committer on a positive offset.
        let early = header("sha0000", "2026-08-01T10:00:00+02:00");
        let next = header("sha0001", "2026-08-01T09:00:00+00:00");
        let mut stats = HashMap::new();
        let mut texts: HashMap<String, patches::CommitPatches> = HashMap::new();
        for key in [&early, &next] {
            let file = stat("f0");
            let per_file: patches::CommitPatches = std::iter::once((
                file.filename.clone(),
                Patch {
                    text: "x".repeat(100),
                    truncated: false,
                },
            ))
            .collect();
            stats.insert(key.sha.clone(), vec![file]);
            texts.insert(key.sha.clone(), per_file);
        }

        let caps = RowCaps {
            max_rows: usize::MAX,
            max_patch_bytes: 150,
        };
        let (rows, cursor) =
            emit_file_changes(vec![early, next.clone()], &stats, &mut texts.clone(), caps);
        assert_eq!(rows.len(), 1, "only the first commit fits");
        let Some((primary, secondary)) = cursor else {
            panic!("a page stopped early must carry a cursor")
        };

        let token = PageToken {
            entry: "e".to_owned(),
            generation: 1,
            incarnation: "i".to_owned(),
            primary,
            secondary,
        };
        assert!(
            token.precedes(&next.ordinal, &next.sha),
            "the very next commit must still be reachable from the cursor"
        );
    }

    #[test]
    fn an_unbounded_page_keeps_every_row_and_no_early_cursor() {
        let (window, stats, texts) = scenario(3, 2, 4);
        let (rows, cursor) =
            emit_file_changes(window, &stats, &mut texts.clone(), RowCaps::DEFAULT);

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
        let (rows, cursor) = emit_file_changes(window, &stats, &mut texts.clone(), caps);

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
        let (rows, cursor) = emit_file_changes(window, &stats, &mut texts.clone(), caps);

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
        let (rows, _) = emit_file_changes(window, &stats, &mut texts.clone(), caps);

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
        let (rows, cursor) = emit_file_changes(window, &stats, &mut texts.clone(), caps);

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
            &mut HashMap::new(),
            RowCaps::DEFAULT,
        );
        assert!(rows.is_empty());
        assert_eq!(cursor, None);
    }
}
