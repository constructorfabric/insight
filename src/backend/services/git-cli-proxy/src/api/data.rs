use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::engine::page::PageToken;
use crate::engine::read::{self, Page, blobs, branches, commits, numstat, patches};
use crate::engine::store::{Freshness, RepoGuard};

use super::AppState;
use super::error::ApiError;
use super::request::{Paging, RequestContext, clamp_page_size, clamp_patch_bytes};

#[derive(Debug, Deserialize)]
pub struct CommitsQuery {
    repo: String,
    since: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FileChangesQuery {
    repo: String,
    since: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
    include_patch: Option<bool>,
    max_patch_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct BranchesQuery {
    repo: String,
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
) -> Result<Json<Page<commits::CommitRow>>, ApiError> {
    let context = RequestContext::from_parts(&headers, &query.repo)?;
    let paging = Paging::parse(query.page_token.as_deref(), query.page_size)?;
    let guard = open(&state, &context, &paging).await?;

    let runner = state.store.runner();
    let headers_page = commits::headers(runner, guard.git_dir(), query.since.as_deref()).await?;
    let (window, cursor) = read::slice_page(
        headers_page,
        paging.token.as_ref(),
        paging.page_size,
        |header| (header.committed_date.clone(), header.sha.clone()),
    );

    let shas: Vec<String> = window.iter().map(|header| header.sha.clone()).collect();
    blobs::prefetch(runner, guard.git_dir(), &shas, &context.creds).await?;

    let file_stats = numstat::read(runner, guard.git_dir(), &shas).await?;
    let membership = commits::branch_membership(runner, guard.git_dir(), &shas).await?;
    let ids = commits::patch_ids(runner, guard.git_dir(), &shas).await?;

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
                branch_names: membership.get(&header.sha).cloned().unwrap_or_default(),
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

    Ok(Json(Page {
        items,
        next_page_token: encode_cursor(cursor, guard.generation()),
    }))
}

/// # Errors
///
/// [`ApiError`] on malformed input, origin failures, or a snapshot that moved
/// out from under a page cursor.
pub async fn list_file_changes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<FileChangesQuery>,
) -> Result<Json<Page<FileChangeRow>>, ApiError> {
    let context = RequestContext::from_parts(&headers, &query.repo)?;
    let paging = Paging::parse(query.page_token.as_deref(), query.page_size)?;
    let include_patch = query.include_patch.unwrap_or(true);
    let max_patch_bytes = clamp_patch_bytes(query.max_patch_bytes);
    let guard = open(&state, &context, &paging).await?;

    let runner = state.store.runner();
    let all = commits::headers(runner, guard.git_dir(), query.since.as_deref()).await?;
    // Parity with the CDK connectors: merge commits contribute no file rows.
    let non_merge: Vec<commits::CommitHeader> = all
        .into_iter()
        .filter(|header| !header.is_merge())
        .collect();
    let (window, cursor) = read::slice_page(
        non_merge,
        paging.token.as_ref(),
        clamp_page_size(query.page_size),
        |header| (header.committed_date.clone(), header.sha.clone()),
    );

    let shas: Vec<String> = window.iter().map(|header| header.sha.clone()).collect();
    blobs::prefetch(runner, guard.git_dir(), &shas, &context.creds).await?;

    let file_stats = numstat::read(runner, guard.git_dir(), &shas).await?;
    let texts = if include_patch {
        patches::read(runner, guard.git_dir(), &shas, max_patch_bytes).await?
    } else {
        std::collections::HashMap::new()
    };

    let mut items = Vec::new();
    for header in window {
        let files = file_stats.get(&header.sha).map_or(&[][..], Vec::as_slice);
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
    }

    Ok(Json(Page {
        items,
        next_page_token: encode_cursor(cursor, guard.generation()),
    }))
}

/// # Errors
///
/// [`ApiError`] on malformed input or an origin failure.
pub async fn list_branches(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<BranchesQuery>,
) -> Result<Json<Page<branches::BranchRow>>, ApiError> {
    let context = RequestContext::from_parts(&headers, &query.repo)?;
    let paging = Paging::parse(None, None)?;
    let guard = open(&state, &context, &paging).await?;

    let items = branches::read(state.store.runner(), guard.git_dir()).await?;
    Ok(Json(Page {
        items,
        next_page_token: None,
    }))
}

/// Resolve the snapshot: a first page honors fetch-if-stale, a continuation is
/// pinned to the generation its token carries.
async fn open(
    state: &Arc<AppState>,
    context: &RequestContext,
    paging: &Paging,
) -> Result<RepoGuard, ApiError> {
    let freshness = match paging.token.as_ref() {
        Some(token) => Freshness::Pinned {
            generation: token.generation,
        },
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

fn encode_cursor(cursor: Option<(String, String)>, generation: u64) -> Option<String> {
    cursor.map(|(committed_date, sha)| {
        PageToken {
            generation,
            committed_date,
            sha,
        }
        .encode()
    })
}
