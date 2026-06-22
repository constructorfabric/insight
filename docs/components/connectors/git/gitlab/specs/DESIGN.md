# DESIGN — GitLab Connector

> Status: draft for review
> Scope: self-hosted GitLab (CE & EE), REST API v4
> Type: CDK (Python), thin extractor per [ADR-0002](../../../../../domain/connector/specs/ADR/0002-connector-responsibility-scope.md)

## 1. Purpose & Scope

A typed Python CDK Airbyte source that extracts code, review, and flow data
from a self-hosted GitLab instance and emits source-native RECORD messages to
stdout. The connector transforms nothing: it pulls, trims oversized text,
attaches the platform envelope, and streams. All schema unification, identity
resolution, and metrics live downstream in dbt (silver/gold).

Design priorities, in order: **robustness → memory safety → throughput**. A
sync must never OOM, never accumulate unbounded state, and must complete or
fail loudly — never silently truncate coverage.

**Universality.** This is an open-source connector targeting no specific
instance. The same artifact must run correctly against any GitLab — CE or EE,
any supported version, any rate-limit configuration, any size, behind any
deployment. No behavior is simplified or tuned for a known first target;
characteristics of one instance are never assumed of another. Capabilities are
detected at runtime (§5.4), limits are honored as reported (§5.6).

### 1.1 Inherited contracts

- **Thin extractor** ([ADR-0002](../../../../../domain/connector/specs/ADR/0002-connector-responsibility-scope.md)): stdout only, zero DB access, no transformation.
- **Append-only sync**: `destinationSyncMode=append`. Dedup is a downstream RMT concern, never the connector's.
- **Envelope on every record**: `insight_tenant_id`, `insight_source_id`, `data_source`, `collected_at`, `unique_key`.
- **unique_key** ([ADR-0004](../../../../domain/ingestion-data-flow/specs/ADR/0004-unique-key-formula.md)): `{tenant}:{source}:{natural_key_parts}` joined with colons. Colon delimiter (not the ADR's literal dash) because git ref names forbid `:` but allow `-`/`/`, and the prefix parts already carry dashes (tenant UUID, `source_id`). Collision-free under two invariants: (A) `source_id` contains no colon; (B) only the terminal key component may contain a colon — all non-terminal parts are numeric ids or hex SHAs, making concatenation injective. Deviation from ADR-0004 is the git-connector convention.

## 2. Insight Objectives

Extraction is justified by the insights it must enable downstream. The data
captured must make all of the following derivable in dbt **without
re-extraction**:

| Family | Representative metrics | Primary inputs |
|---|---|---|
| Flow | MR cycle time by phase (code → open → first-review → approve → merge), throughput, WIP age/size | `merge_requests`, `merge_request_commits`, `merge_request_notes`, `merge_request_state_events`, `merge_request_approvals` |
| Review health | time-to-first-review, review load, comment depth, rubber-stamp detection, author↔reviewer graph | `merge_request_notes`, `merge_request_discussions`, `merge_request_approvals` |
| Volume | commits & lines per person/week, code-vs-test-vs-config split, size distributions | `commits` (+stats), `commit_file_changes` |
| Code health | file/module churn, hotspots (churn × authors), ownership | `commit_file_changes`, `commits` |
| Identity | resolve git email ↔ GitLab username ↔ person | `users`, every actor-bearing record |

## 3. Stream Catalog

Streams are GitLab-native and land unprefixed in the `bronze_gitlab` namespace
(e.g. `bronze_gitlab.merge_requests`). Field lists below are the
extraction-relevant projection, not the full API payload.

### 3.1 Structure / dimension streams

Small, bounded, refreshed in full each run (snapshot semantics; downstream RMT
keeps the latest version).

#### `projects`
Cursor: none (full snapshot). Natural key: `id`. Two enumeration modes, selected
by config with no extra flag:
- **Scoped** (`gitlab_groups` set): `GET /groups/:id/projects?include_subgroups=true`
  per configured group (offset pagination).
- **Whole instance** (`gitlab_groups` empty): `GET /projects?pagination=keyset&order_by=id&sort=asc`
  — every project the token can access (keyset). Full coverage requires an admin
  or broad service token; otherwise scope is whatever the token can see.

`gitlab_projects` adds explicit projects on top of either mode.

| Field | Type | Notes |
|---|---|---|
| `id` | Int | project id |
| `path_with_namespace` | String | `group/subgroup/repo` |
| `name`, `path` | String | |
| `namespace_id`, `namespace_full_path` | Int / String | group attribution |
| `default_branch` | String | defines trunk |
| `visibility` | String | private/internal/public |
| `archived` | Bool | |
| `created_at`, `last_activity_at` | DateTime | |
| `statistics_commit_count`, `statistics_repository_size` | Int | requires `statistics=true` (member/owner scope) |

#### `users`
Cursor: none. Natural key: `id`. Whole-instance scope (no groups/projects
configured) uses `/users` (keyset), naturally unique. Scoped runs use
`/groups/:id/members/all` and `/projects/:id/members/all`; a user in multiple
scopes is emitted once per scope and collapsed downstream by RMT on
`unique_key` — no in-connector dedup set, which keeps streams concurrency-safe.

| Field | Type | Notes |
|---|---|---|
| `id` | Int | |
| `username` | String | identity bridge to MR/note actors |
| `name` | String | |
| `public_email` / `email` | String | identity bridge to commit author email (often empty on CE) |
| `state` | String | active/blocked |

#### `branches`
Cursor: none. Natural key: `project_id` + `name`. **Listed only — never
commit-iterated.**

| Field | Type | Notes |
|---|---|---|
| `project_id` | Int | |
| `name` | String | |
| `commit_sha` | String | branch head |
| `default`, `protected`, `merged` | Bool | governance signal |

Label and milestone names are carried inline on the MR record
(`labels`/`milestone_id`); no standalone catalog stream is collected.

### 3.2 Code streams

#### `commits`
All-branch coverage via **SHA-graph deltas using revision ranges**, never
`all=true` (which is empirically dirty — it returns keep-around / hidden refs,
double-counting squashed-away and force-pushed history). The commits API accepts
a git revision range in `ref_name` (`A..B` = commits reachable from `B` but not
`A`), with `with_stats=true` (inline per-commit additions/deletions/total) and
pagination — all verified on a live instance.

Per project: enumerate branches (reusing the `branches` stream's per-project
records), then scan only the graph deltas:

- **default branch**: `old_default_head..new_default_head` (first run: full
  default from `gitlab_start_date`).
- **each non-default branch ahead of default**: `default_head..branch_head` —
  returns only the branch-unique (unmerged feature) commits, **no shared-trunk
  re-paging** (verified: a feature branch returned 8 unique vs 534 full).
- branch head SHA unchanged since last scan → skip.

This captures in-progress feature work **as it is pushed** (daily, correctly
authored and dated, with line totals) — independent of any MR — while fetching
only deltas. Cross-branch / inclusive-boundary overlaps collapse via RMT on
`unique_key`. `since`/`until` are used **only** to window a range under the
offset cap, never for correctness (correctness is graph-based, immune to
old-dated pushes). Natural key: `project_id` + `sha`.

| Field | Type | Notes |
|---|---|---|
| `project_id` | Int | |
| `id` (sha), `short_id` | String | |
| `title`, `message` | String | trimmed (§5.3) |
| `author_name`, `author_email` | String | identity key |
| `authored_date` | DateTime | |
| `committer_name`, `committer_email` | String | |
| `committed_date` | DateTime | |
| `parent_count` | Int | `is_merge` derivable: `> 1` (scalar; full parent SHAs not stored in v1) |
| `stats_additions`, `stats_deletions`, `stats_total` | Int | from `with_stats` |

Downstream reconciliation (dbt): per-person *volume* counts original authored
commits and **excludes** merge commits (`parent_count > 1`) and squash commits
(via MR `squash_commit_sha`), so the squashed-away originals (captured here from
the feature branch) are counted once — not double-counted against the squash
commit on trunk.

#### `commit_file_changes`
Per-file change for **landed (default-branch) commits only**, from `GET
/repository/commits/:sha/diff`. Independent top-level stream (NOT a CDK substream
of `commits` — that would cache the unbounded commits parent and balloon the
requests-cache). It re-derives the small default-branch SHA delta and fetches
one diff per commit; **merge commits (`parent_count > 1`) are skipped** to avoid
double-counting the combined merge diff (squash commits are normal single-parent
landed commits and are diffed).

GitLab returns no per-file counts → the connector parses hunks: count lines
starting `+`/`-`, excluding `+++`/`---`/`@@`/`\ No newline`; `too_large` /
`collapsed` (or beyond GitLab's diff limit) → row with null counts +
`diff_truncated=true`; binary / rename / mode-only → `0/0`; per-file line budget
→ null + truncated on overflow.

Cursor: none of its own; driven by the default-branch SHA delta. Natural key:
`project_id` + `sha` + `new_path`.

| Field | Type | Notes |
|---|---|---|
| `project_id`, `commit_sha` | Int / String | |
| `old_path`, `new_path` | String | |
| `new_file`, `deleted_file`, `renamed_file` | Bool | → change_type downstream |
| `lines_added`, `lines_removed` | Int | parsed from hunks; null if truncated/binary-unknown |
| `diff_truncated` | Bool | true when GitLab elided the diff |

> Feature-branch per-file churn is deferred — per-commit line *totals* (volume)
> already arrive timely from `commits` `with_stats`; per-file granularity on
> unmerged work is a later add if file-level daily churn is needed.

### 3.3 Review / flow streams

#### `merge_requests`
Cursor: `updated_at` (`updated_after` + `order_by=updated_at`, `sort=asc`). Two
enumeration modes (§4): **whole-instance** scope (no groups/projects configured)
uses the global `/merge_requests?scope=all` endpoint — one stream returning
every visible MR (archived projects included by default), each carrying
`project_id`, so dormant projects never appear; **configured** scope resolves to per-project
`/projects/:id/merge_requests` over the `ProjectsStream`-resolved set (groups
expand via `include_subgroups`, deduped by project id). The group-level
`/groups/:id/merge_requests` endpoint is **not** used (its issue counterpart
silently omits project rows). Re-emitting a changed MR re-drives its children.
Natural key: `project_id` + `iid`.

| Field | Type | Notes |
|---|---|---|
| `project_id`, `iid`, `id` | Int | `iid` per-project, `id` global |
| `title`, `description` | String | trimmed (§5.3) |
| `state` | String | opened/closed/merged/locked |
| `draft` (`work_in_progress`) | Bool | |
| `author_id`, `author_username` | Int / String | |
| `assignee_ids`, `reviewer_ids` | Array(Int) | assigned reviewers |
| `merged_by_id`, `merged_by_username` | Int / String | |
| `source_branch`, `target_branch` | String | |
| `created_at`, `updated_at`, `merged_at`, `closed_at` | DateTime | phase anchors |
| `sha`, `merge_commit_sha`, `squash_commit_sha` | String | links MR → landed commit |
| `squash`, `squash_on_merge` | Bool | squash detection |
| `merge_status`, `user_notes_count` | String / Int | |
| `labels`, `milestone_id` | Array(String) / Int | |

#### `merge_request_commits`
**Identity only** — no stats, no diffs. Captures squashed-away commit identity
and commit↔MR membership. Driven by parent MR. Natural key: `project_id` +
`mr_iid` + `sha`.

| Field | Type | Notes |
|---|---|---|
| `project_id`, `mr_iid` | Int | |
| `id` (sha), `short_id` | String | |
| `title`, `message` | String | trimmed |
| `author_name`, `author_email`, `authored_date` | String / DateTime | |
| `committed_date`, `parent_ids` | DateTime / Array | |
| `commit_order` | Int | position within MR |

#### `merge_request_notes`
User comments + system notes. Driven by parent MR. Natural key: `project_id` +
`note_id`.

| Field | Type | Notes |
|---|---|---|
| `project_id`, `mr_iid`, `id` | Int | |
| `body` | String | trimmed (§5.3) |
| `author_id`, `author_username` | Int / String | |
| `created_at`, `updated_at` | DateTime | |
| `system` | Bool | system note vs human comment |
| `resolvable`, `resolved`, `resolved_by_id` | Bool / Int | review-thread resolution |
| `position_new_path`, `position_new_line` | String / Int | diff-anchored comments (null for general) |

#### `merge_request_discussions`
Thread grouping for notes (which notes form one resolvable discussion). Driven
by parent MR. Natural key: `project_id` + `mr_iid` + `discussion_id` +
`note_id`.

| Field | Type | Notes |
|---|---|---|
| `project_id`, `mr_iid` | Int | |
| `discussion_id` | String | thread id |
| `note_id` | Int | member note |
| `individual_note` | Bool | standalone vs thread |

#### `merge_request_approvals`
EE feature — **degrades gracefully on CE** (§5.4). Driven by parent MR. Natural
key: `project_id` + `mr_iid` + `approver_user_id`.

| Field | Type | Notes |
|---|---|---|
| `project_id`, `mr_iid` | Int | |
| `approvals_required`, `approvals_left` | Int | |
| `approved_by_id`, `approved_by_username` | Int / String | one row per approver |

> Approval *timing* (when the click happened) is not on this endpoint. It is
> reconstructed downstream from `merge_request_state_events` /
> `merge_request_notes` system entries.

#### `merge_request_state_events`
Structured lifecycle events (opened/closed/reopened/merged) with actor +
timestamp. Cleaner than parsing system-note bodies. Driven by parent MR.
Natural key: `project_id` + `event_id`.

| Field | Type | Notes |
|---|---|---|
| `project_id`, `mr_iid`, `id` | Int | |
| `user_id`, `user_username` | Int / String | actor |
| `state` | String | merged/closed/reopened/opened |
| `created_at` | DateTime | event time |

### 3.4 Planning streams (recommended)

#### `issues`
Cursor: `updated_at`, same two-mode enumeration as `merge_requests`: whole-instance
`/issues?scope=all`, else per-project `/projects/:id/issues`
over the resolved set. `/groups/:id/issues` is **not** used — it omits
project-level issues. Powers issue↔MR linking, ticket refs, issue cycle time.
Natural key: `project_id` + `iid`.

| Field | Type | Notes |
|---|---|---|
| `project_id`, `iid`, `id` | Int | |
| `title`, `description` | String | trimmed |
| `state` | String | opened/closed |
| `author_id`, `assignee_ids` | Int / Array | |
| `labels`, `milestone_id` | Array / Int | |
| `created_at`, `updated_at`, `closed_at` | DateTime | |

Issue notes / state events: same pattern as MR children, added when issue
analytics is built out.

### 3.5 Deferred (designed-for, separate milestone)

CI/CD → DORA: `pipelines`, `jobs`, `deployments`, `releases`. A distinct domain
(delivery vs code-review). Bronze namespacing leaves room; not in the first
cut.

## 4. State & Incremental Model

| Stream | Cursor | Strategy |
|---|---|---|
| `projects`, `users`, `branches` | — | full snapshot each run |
| `commits` | per-ref head **SHA** | scan graph deltas `old_head..new_head` |
| `commit_file_changes` | per-project default head **SHA** | diff the default-branch SHA delta |
| `merge_requests`, `issues` | per-**scope** `updated_at` (`instance` or `project:<id>`) | `updated_after = max(start_date, watermark − overlap)` (`ScopeUpdatedAtStream`) |
| MR children (`merge_request_*`) | **own** per-scope `updated_at` | each independently re-enumerates changed MRs (same scope modes), then fetches its sub-resource |

**Scope, not per-project fan-out.** Whole-instance scope (no groups/projects)
enumerates `merge_requests`/`issues`/the MR enumerator from the global
`scope=all` list endpoints, state-keyed `instance`. Any configured scope resolves
to one cursor per **distinct project** (groups expand via the `ProjectsStream`
parent's `include_subgroups`, deduped by id), state-keyed `project:<id>`, hitting
`/projects/:id/...`. Both filter on each record's own `updated_at` — the cursor
field — so dormant projects never appear, with no reliance on
`project.last_activity_at` (an unreliable activity watermark — it lags real
activity and does not advance for every event type, so it is never a crawl gate;
emitted only as a `projects` field). The group-level list endpoints are avoided
(`/groups/:id/issues` omits project-level issues).

**MR children do NOT share the top-level `merge_requests` cursor.** A child
reading through an incremental parent would be starved (the parent advances its
own cursor mid-sync). Each child (`MergeRequestChildStream`) owns a private
scope-aware MR enumerator and its **own** per-scope `updated_at` watermark —
decoupled, independently resumable. Cost: each child re-enumerates MRs
(the cursor-filtered list, cheap); the per-MR sub-resource fetches are not
duplicated.

Date cursors (`merge_requests`/`issues`/MR children) floor `updated_after` at
`start_date` on first run (so `start_date` bounds the whole sync, not just
`commits`), then `watermark − overlap` clamped to ≥ `start_date`, all normalised
to UTC `Z` (the `+offset` form must not reach the URL). The cursor advances
**monotonically per record** (max-guarded against the 1 s window-boundary
re-emit) and the parent streams set `state_checkpoint_interval` so an
instance-wide scope checkpoints mid-slice rather than all-or-nothing; RMT
collapses the boundary re-emit on resume. Offset-cap windowing (§5.8) applies to
these cursor streams unchanged (server-sorted `updated_at` ⇒ rolling-continue).
`merge_request_approvals` captures the current approver **set** (no per-approver
timestamp exists on the endpoint); approval **timing** is derived downstream from
`merge_request_notes` system notes.

**Commits use graph (SHA) cursors, not date cursors** — more correct than dates
because a rebase/old-dated push changes the head SHA, so the `old_head..new_head`
range captures it regardless of commit date (the old-dated-push miss dissolves).
State, per project, head updated only after a range completes (crash mid-range →
next run re-fetches the same delta; RMT dedups):

```json
{"projects": {"123": {
  "default_branch": "main",
  "default_head_sha": "abc…",
  "last_project_activity_at": "…",
  "branches": {"feature/x": {"head_sha": "def…", "merged": false,
                             "last_seen_at": "…", "deleted_at": null}},
  "recent_scanned_heads": {"def…": "…"}
}}}
```

Scan logic per run: per branch, unchanged head → skip; new branch →
`default_head..branch_head`; changed
branch → `old_head..new_head`; missing old SHA (force-push) or range error →
fall back to `default_head..branch_head`; default changed →
`old_default..new_default`; first run → full default from `gitlab_start_date`
then `default_head..branch_head` per non-default branch. Deleted branch →
`deleted_at` + prune after retention; rename = delete + new branch (skip if the
new head is already in `recent_scanned_heads`). `since`/`until` window a single
range only when it nears the offset cap (split until a 1-instant window still
exceeds it → fail loud).

As implemented: non-default branches use `default_head..branch_head` (the base
is always the current default head, never stale — a force-pushed branch just
yields its current unique set). The default-branch range `old_default..new_default`
is the only one with a stored (possibly stale) base; if its base was
force-pushed/GC'd the range 404s, and that slice opts **out** of the 404-skip so
it fails loud rather than advancing the head past a gap — recover with a
full-refresh (state reset → full default re-traversal). `gitlab_start_date`
bounds every commit range via `since`. Stored branch heads are pruned each run
to the current branch set (no unbounded state on branch-farms). Deferred
(documented optimization): `old_branch_head..new_branch_head` deltas with
fallback — currently a changed branch re-fetches its full (small) unique set,
which RMT dedups.

Known gap: a branch pushed **and** deleted between syncs with no MR is not
recoverable from the branch list (gone before observation). Events API is
best-effort only (retention + bulk-push omits refs), not a correctness base.

## 5. Memory & Robustness Model

### 5.1 Bounded streaming
Every stream is a generator. Top-level streams page lazily and yield each
record immediately. For each changed MR, its children are fetched, emitted, and
discarded before the next MR. Peak memory is O(one parent's children), never
O(repo). No global accumulation, no seen-hashes set anywhere.

Substream parents must stay bounded. A substream re-reads its parent's records
to build slices, and the CDK response-caches parent HTTP responses — so an
**unbounded** parent (e.g. `commits` as the parent of `file_changes`) would
balloon that cache. `projects` (bounded, ~pages) is a safe parent; for
`file_changes`, per-commit diffs are driven from within the `commits` traversal
(or a bounded per-project commit-sha partition), never by making the full
`commits` stream a cached substream parent. The push-based children (`branches`,
`commits`, `file_changes`) each re-enumerate the bounded `projects` parent; the
MR children each re-enumerate the cursor-filtered scope MR list. That redundant
enumeration is acceptable while bounded and is revisited in the
concurrency/optimization phase (a shared partition source).

### 5.2 Pagination
Offset-based pagination is the universal mechanism and the only one most of our
endpoints support (`commits`, `merge_requests`, `branches`, group projects, MR
children, notes). Keyset pagination is documented only for `/projects`
(`order_by=id`) and `/users` — used there, offset everywhere else. `per_page` is
capped at 100.

Pages are followed via the `Link` header `rel="next"` until it is absent and the
array is empty — never by constructing page URLs, never relying on totals (above
10,000 results GitLab drops `x-total`/`x-total-pages`).

Offset pagination has a hard ceiling: the instance's max-allowed-offset (default
50,000) caps how deep one ordered query can page. Large collections must not be
paged past that wall — `commits` and `merge_requests` are **windowed by time**
(`since`/`until`, `updated_after`/`updated_before`) so each window stays shallow
on both backfill and incremental runs. Windowing doubles as a memory bound
(each request's result set stays small) and is what keeps the connector correct
on instances large enough to exceed the offset cap.

### 5.3 Text trimming (baked invariant, no config)
Oversized text is the primary per-record OOM vector. Hard caps applied at emit:

- `message`, `description`, note `body`: 16384 chars
- `title`: 1024 chars
- On truncation, set `<field>_truncated = true`.

Constants, not knobs — pathological multi-MB bodies (pasted logs, generated
changelogs) are cut before they reach memory or storage.

### 5.4 404 handling & capability degradation
GitLab overloads `404` across five meanings; the connector must skip the
benign ones (or it stalls on the first deleted entity) and fail loud on the
rest (or it masks bugs):

The decisive distinction is **discovered child** vs **configured root**:

| 404 cause | Example | Disposition |
|---|---|---|
| Discovered child deleted (TOCTOU) | MR removed before its notes are fetched | skip child, advance |
| Discovered child feature disabled | repository off on a discovered project → `/repository/branches` | skip child, advance |
| Discovered child auth-mask (404, not 403, to hide existence) | non-admin token, private discovered resource | skip child, advance |
| **Configured root** absent/typo/revoked | `gitlab_groups`/`gitlab_projects` entry that 404s | **fail loud** (preflighted in `check`) |
| Our bug — wrong path / wrong id | typo → every entity 404s | fail loud / detectable |
| Null-parent routing | `/projects/None/...` | fail loud |
| 404 **after pagination started** (page 2+) | entity deleted mid-read | **fail loud** (no partial slice) |

Policy (structural, no config):

- **Substreams skip `404` only on the first page of a slice**
  (`GitlabSubstream.skippable_statuses = {404}`, inherited; the base predicate
  also requires `next_page_token is None`). A discovered child that is gone
  before any data is read is skipped; a `404` *after* pagination has begun
  raises `GitlabApiError` rather than silently keeping a truncated slice — it
  self-heals on the next sync when the entity is fully gone.
- **Configured roots raise on `404`.** `ScopedGitlabStream` (the `projects` and
  `users` streams) does **not** skip — a configured group/project that 404s is
  explicit coverage loss the operator must see. Configured groups *and* projects
  are validated in `check_connection` (preflight), so a bad root fails at setup,
  not silently mid-sync. The whole-instance list endpoints (`/projects`,
  `/users`) also raise — they never 404 on success.
- **Null-parent-routing guard**: a substream raises `GitlabApiError` if the
  parent routing id is null/missing *before* building the path, so the
  `/None/...` bug fails loud instead of masquerading as a skippable 404.
- **Every skip is logged** with stream + url.
- `400` / `409` / `422` / `5xx`-after-retry always raise.

EE-only or license-gated endpoints (notably `merge_request_approvals`) extend
their own skip set (e.g. `{402, 403, 404}`) so CE and EE both work unchanged;
generic substreams keep raising on `403` (usually a token-scope problem worth
surfacing).

Residual: a discovered-child stream where the token can see *none* of the
resource (every first-page request 404s) collects zero rows. The bug shapes
that could cause this all raise (routing, mid-pagination, configured-root); the
remaining case is a genuine permission boundary, surfaced by per-table bronze
freshness monitoring (a stale `_airbyte_extracted_at` for that stream's table),
which is per-stream, not per-connection. An in-connector all-404 abort is not
added because the CDK's per-slice `read_records` has no stream-completion hook
to distinguish it from a single benign skip without a heuristic threshold.

### 5.5 Diff safety
`commit_file_changes` parses hunks with a per-diff line budget; diffs GitLab
marks `too_large`/`collapsed` are recorded as a row with null counts +
`diff_truncated=true` rather than fetched in expanded form.

### 5.6 Rate limits & retry
Rate-limit handling is first-class and universal. The connector makes **no
assumption** about any instance's limits — admins configure them freely (none,
strict per-user/IP, per-endpoint, search-specific, raw-blob, …) and the same
artifact must run correctly everywhere. It complies with whatever the instance
reports:

- On `429`: wait per `Retry-After` (seconds or HTTP-date). If absent, derive
  the wait from `RateLimit-Reset` / `RateLimit-ResetTime`.
- Proactive throttle: when `RateLimit-Remaining` approaches zero, slow down
  before being refused rather than spinning into 429s.
- Exponential backoff with jitter on `429` and `5xx`; bounded retry count;
  a page is never silently dropped — exhaustion raises.
- Auth failures (401, non-rate-limit 403) raise loudly rather than being
  swallowed as empty pages.

Verified header semantics: `RateLimit-Limit`, `RateLimit-Name`,
`RateLimit-Observed`, `RateLimit-Remaining`, `RateLimit-Reset` are returned on
**all** responses — `RateLimit-Reset` is **Unix epoch seconds**, not a relative
value. On `429`, `RateLimit-ResetTime` (RFC2616 HTTP-date) and `Retry-After`
(seconds) are added and the body is **plain text** (`Retry later`), never JSON.

Critically, application-level limits (distinct from the header-reported
`Rack::Attack` throttles) also return `429` but expose **no** `RateLimit-*`
headers — so a `429` must be handled even when `Remaining` looked healthy, and
the handler never assumes a 429 is explained by the headers. Wait precedence:
`Retry-After` → `RateLimit-ResetTime` / `RateLimit-Reset` → exponential backoff
with jitter. Under concurrency the limiter is respected per request, so a 429 on
one worker backs that request off without failing the stream.

### 5.7 Concurrency posture
A shared instance can be degraded by aggressive crawling whether or not it
enforces limits, so politeness is the connector's responsibility in addition to
honoring whatever limits the instance reports (§5.6). Therefore:

- Streams are written concurrency-safe from the start (no shared mutable state,
  clean `stream_slices`/cursor contracts), but run **sequentially until each is
  correct**. ConcurrentSource is enabled only after sequential correctness.
- Concurrency is **bounded (~4–8), not maximised** — it is the politeness
  throttle against the shared instance, not a throughput dial.
- Concurrency must be **≥4**: the CDK self-deadlocks at `default_concurrency=1`
  once a sync exceeds ~10k partitions.
- The primary throughput lever is **request count** (diff granularity), not
  thread count. Concurrency multiplies a reduced N.

### 5.8 Offset-cap windowing

Offset pagination has a hard ceiling (instance max-allowed-offset, default
50,000). A single ordered query cannot page past it, so large collections are
split into time windows. A collection that genuinely cannot be windowed **fails
loud** — never silently truncates.

**Detection** (no dependence on totals, which vanish above 10k results — the
commits endpoint omits `x-total` entirely):
- Proactive **soft page cap** is the primary trigger — `SOFT_PAGE_LIMIT = 490`
  (×100 = 49k, just under the default). Deterministic; needs no instance config.
- Reactive **HTTP 400** (offset/pagination text) as a backstop for instances
  with a lower-than-default cap. The instance's configured cap is never read
  (non-admin tokens can't).

**Where:** inside `read_records`, via an internal window loop — never in
`stream_slices` (window sizes are unknowable upfront, and a slice erroring
mid-pagination can't resume). The outer slice stays the logical unit (ref range
/ project); the loop calls `HttpStream.read_records` per concrete window. Page
count is tracked in `next_page_token`, the cap-400 raised from `parse_response`;
both surface a `WindowTooLarge` the loop catches. Partial records emitted before
a split are harmless (RMT dedups).

**Two adapters under a shared `TimeWindowedReadMixin`:**
- `UpdatedAtWindowing` (`merge_requests`, `issues`, MR enumerator — server-sorted
  `updated_at asc`): **rolling-continue** — on soft cap, the next window's
  `updated_after = last-emitted updated_at − 1s` (the 1 s rewind tolerates an
  inclusive `updated_after`; the boundary record re-emits and RMT dedups). When
  the next start would not advance past the current window start, >cap records
  share one timestamp ⇒ fail loud.
- `CommittedDateWindowing` (`commits`, `commit_file_changes` — a ref range is not
  date-sorted, so cannot roll): **midpoint-bisect** `[since, until]` into
  `[since, mid]` + `[mid, until]`. `since`/`until` are inclusive, so the halves
  already overlap at `mid` — no manual rewind. The over-cap window re-reads its
  range on split (RMT dedups). The **final window stays open-ended** (no
  `until`); bisecting an open window uses `now()` as the working upper bound
  while the right child keeps the open end, so the realistic handful of
  clock-skewed future-dated commits lands in that open tail and is collected.
  Window bounds are serialized at second resolution; when the midpoint collapses
  onto a bound, the window is unsplittable ⇒ fail loud. A genuinely unbounded
  future is not windowable by bisection — a tripped window is incomplete and the
  API returns commits in topological, not chronological, order, so there is no
  reliable max date to bound against; the degenerate case of more-than-the-cap
  commits dated across future time fails loud rather than truncating silently.
  Date-only / tz-naive inputs (`gitlab_start_date` is `YYYY-MM-DD`) normalize to
  UTC before any arithmetic.

**State:** the cursor (per-ref SHA / per-scope `updated_at`) advances only
after **all** windows of the unit complete. No per-window persistence — a crash
re-runs the unit; append-only + RMT make the re-emit harmless.

`fail loud` raises `UnwindowableWindow` carrying the window bounds — i.e. >cap
records share one timestamp / one unsplittable second, unwindowable via the
available API.

Timestamps round-trip through `datetime.fromisoformat` with `Z` normalized to
`+00:00` (the 3.10 floor rejects a bare `Z`); windows are emitted UTC `Z` so the
`+`-offset encoding pitfall (§9) never reaches the query string.

Bronze is source-native. Staging dbt models map each stream to the shared
silver contract via `silver:<class>` tags.

| Stream | Silver target tag |
|---|---|
| `projects` | `silver:class_git_repositories` |
| `branches` | `silver:class_git_repository_branches` |
| `commits` | `silver:class_git_commits` |
| `commit_file_changes` | `silver:class_git_file_changes` |
| `merge_requests` | `silver:class_git_pull_requests` |
| `merge_request_commits` | `silver:class_git_pull_requests_commits` |
| `merge_request_notes` | `silver:class_git_pull_requests_comments` |
| `merge_request_approvals` + distinct note authors | `silver:class_git_pull_requests_reviewers` |

GitLab's review participation maps to the unified reviewer contract as:
approvers → reviewer events with `approved`; distinct non-system note authors →
reviewer events with `commented`.

**Proposed silver centerpiece (silver/gold rebuild):**
`class_git_change_request_event` — one row per lifecycle event
(`opened`, `commit_pushed`, `comment`, `approved`, `merged`, `closed`),
assembled from the MR record + `merge_request_state_events` +
`merge_request_notes` + `merge_request_approvals` + `merge_request_commits`.
Every cycle-time metric becomes a window function over this log; no phase
timestamps are hardcoded in the connector.

**Squash and per-original-commit line stats (v1 scope, honest limitation).**
v1 does **not** fetch per-original-commit line stats for squash-merged work, but
this is a deliberate trade-off, not a no-op:

- Fully served by net data (no per-commit stats needed, often *better*): net
  file churn / hotspots (`commit_file_changes` — a squash commit's own diff is
  the MR's net change; per-commit churn would double-count in-MR rewrites/
  reverts), MR size (landed `commit_file_changes` stats), MR cycle/coding time,
  commit *count* per person (`merge_request_commits` identity).
- **Degraded** without per-commit stats: exact per-person *line* volume and
  work-week line dating (a squash credits all net lines to the squash commit
  author at merge time), file ownership / bus-factor by line churn, and
  original-commit-size distribution. Worst case is **bot/integrator-merged**
  squash MRs, where all lines collapse onto one identity.
- Cheap recovery is mostly unavailable: GitLab's default squash message is only
  `%{title}`, so `Co-authored-by` trailers appear only if the project template
  opted in; squash author = MR creator and committer = merger (not real
  attribution). A downstream proportional split (net lines ÷ original-commit
  authors) is an `estimated` dashboard fallback only, never a compliance signal.

Reversible by design: `merge_request_commits` already carries every original
commit's identity (sha, author, dates), so the refinement is an **additive**
enrichment — a `merge_request_commit_stats` stream keyed
`{tenant}:{source}:{project_id}:{mr_iid}:{sha}`, fetching `/commits/:sha` stats
**only for merged + squashed MRs** (optionally only multi-author), never open
MRs — added later with no re-architecture if a tenant's metrics require it.

## 7. Empirical Verification

Resolved by live-instance spikes:

1. **`all=true` is dirty — confirmed, rejected.** A squash-merged MR with a
   deleted source branch had its squashed-away original commit returned by
   `?all=true` but absent from the default branch → keep-around inclusion →
   double-counting. Commits now use revision-range graph deltas instead.
2. **Revision ranges verified.** `ref_name=<sha>..<sha>&with_stats=true` returns
   full commit fields + inline stats + `Link` pagination; `default..feature`
   returned 8 branch-unique commits vs 534 full (no trunk re-paging);
   `old..new` returns exactly the new commits, empty range returns none.

Still open:

3. **Max-allowed-offset on the target instance** (default 50,000) — confirm the
   value; windowing splits a range only if it nears the cap.
4. **Approvals shape on the target edition** (CE vs EE) — confirm degradation
   path (verified instance is EE).

## 8. Decisions to Ratify (supersede legacy `../gitlab.md`)

1. **Drop `gitlab_files` + integer `file_id` lookup** — surrogate keys need
   global state a stateless extractor can't own; ClickHouse compresses repeated
   paths. → emit `new_path`/`old_path` inline on `commit_file_changes`.
2. **No connector-computed/enrichment fields** (`duration_seconds`,
   `ai_percentage`, `language_breakdown`, scancode) — transformation lives in
   dbt/enrichment per ADR-0002.
3. **All-branch commits via SHA-graph revision-range deltas**, not `all=true`
   (empirically dirty — returns keep-around / hidden refs that double-count
   squashed-away and force-pushed history) and not full per-branch iteration —
   scan only `old..new` graph deltas (§3.2), with no in-memory dedup set.
4. **MR terminology, unprefixed stream names** in `bronze_gitlab` — source-native
   names (`merge_requests`); the namespace supplies the prefix, so table names
   carry no `gitlab_`.
5. **No full raw-payload `metadata` column** — explicit typed fields only;
   raw-blob storage doubles payload, and worse, re-introduces the untrimmed
   text the §5.3 caps just removed. Diverges from the CONNECTORS_REFERENCE
   `metadata` convention. Ratified.
6. **Add lifecycle event streams** (`*_state_events`, `discussions`) absent from
   the legacy spec — required for the event log.
7. **Explicit declared fields; no `additionalProperties` reliance** — the
   ClickHouse destination (v2.x, `enable_json=false`) persists only
   schema-declared fields; undeclared properties do not become bronze columns.
   Each stream therefore **projects** every record to exactly its declared field
   set, hoisting needed nested scalars to top-level (e.g.
   `namespace.full_path` → `namespace_full_path`) — field mapping per ADR-0002,
   not derivation. All-scalar output also sidesteps `enable_json=false`. Bonus:
   smaller in-flight records (memory) and a deterministic bronze contract.
8. **Two-mode MR / issue enumeration**, not per-project substreams nor group
   list endpoints — whole-instance scope uses the global
   `/merge_requests?scope=all` & `/issues?scope=all` endpoints (one stream,
   archived projects included by default); any configured scope resolves to per-project
   `/projects/:id/...` over the `ProjectsStream`-resolved set (groups expanded via
   `include_subgroups`, deduped by project id). Both filter on each record's own
   `updated_at`, so dormant projects never appear. The group-level
   `/groups/:id/{merge_requests,issues}` endpoints are avoided — the issue
   variant omits project-level issues (silent loss).
9. **`gitlab_start_date` floors every incremental cursor**, not only `commits` —
   first-run `updated_after = start_date` for `merge_requests` / `issues` /
   MR children (clamped on the overlap), so the configured window bounds the
   entire backfill.
10. **`last_activity_at` is never a crawl gate** — it lags real activity and does
    not advance for every event type, so it cannot decide project dormancy
    without silent loss. It is emitted only as a `projects` field; dormant-project
    skipping is delivered by the enumeration design (#8), not by activity gating.

## 9. Verified API Conventions (REST v4)

Confirmed against current GitLab docs. The connector codes to these, not to
assumptions.

- **Base / auth** — `{instance}/api/v4`; `PRIVATE-TOKEN: <pat>` header.
- **Encoding** — namespaced project paths and any path/branch/tag containing `/`
  are URL-encoded (`%2F`). A literal `+` in an ISO-8601 query value must be
  `%2B`; the connector emits UTC `Z` timestamps so `+` never arises.
- **`id` vs `iid`** — MRs and issues are addressed within a project by `iid`
  (per-project); `id` is instance-global. Both captured.
- **`null` booleans** — boolean fields may be `null` (GitLab treats as `false`);
  parsing must not assume `true`/`false`.
- **Status codes** — `401` unauthenticated, `403` forbidden, `404`
  not-found-or-unauthorized, `429` rate-limited, `5xx` server/overload. `401` and
  non-rate-limit `403` raise; `404` on an optional sub-resource is skipped+logged
  (§5.4); `429`/`5xx` retry with backoff (§5.6).
- **Pagination headers** — `Link` (`rel=prev/next/first/last`) on offset;
  `Link rel=next` + `X-NEXT-CURSOR`/`X-PREV-CURSOR` on keyset. `x-total`/
  `x-total-pages` are omitted above 10,000 results — never relied upon.
- **Commit list** — `ref_name=A..B` (git revision-range delta, §3.2; `all=true`
  rejected as dirty), `with_stats=true` (inline additions/deletions/total),
  `since`/`until` (ISO-8601, range windowing only). Per-file detail only via
  `/repository/commits/:sha/diff` — no per-file counts, parsed from hunks.
- **MR / issue list** — whole-instance via global `/merge_requests` & `/issues`
  with `scope=all` (archived projects included by default; `non_archived` is not a
  valid global-endpoint param); configured scope via per-project
  `/projects/:id/...`. Both with `updated_after`/`updated_before`,
  `order_by=updated_at`, `sort=asc`. The global record carries `project_id`/`iid`,
  `sha`/`merge_commit_sha`/`squash_commit_sha`, `squash`, `user_notes_count`
  (`changes_count` is detail-only — absent from list endpoints). Group-level
  `/groups/:id/{merge_requests,issues}` are not used (issue variant omits project
  rows).
- **Members** — `/groups/:id/members/all` returns inherited members and is
  rate-limited (200/min, 18.6+); enumerate at group level, not per-project, to
  minimize calls.
