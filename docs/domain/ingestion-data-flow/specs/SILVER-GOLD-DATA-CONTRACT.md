# Silver → Gold Data Contract

**Status:** draft v0.2 (2026-07-16)
**Scope:** the fixed interface between the ingestion database (bronze + staging + silver) and the analytics database (gold).

Schemas in this document are based on the **current silver tables** (real column names and types). The contract adds exactly two normative requirements on top of what exists today: `person_id` on every dataset ([§2.2](#22-person_id)) and `patch_id` on commits ([§3](#3-commits)). Everything else describes data as it is produced now. Known deltas between the contract and the implementation are listed in [Appendix A](#appendix-a-gaps-between-this-contract-and-the-current-implementation).

<!-- toc -->

- [1. Purpose](#1-purpose)
- [2. Common conventions](#2-common-conventions)
  - [2.1 Envelope fields](#21-envelope-fields)
  - [2.2 person_id](#22-person_id)
  - [2.3 Deduplication](#23-deduplication)
- [3. Commits](#3-commits)
- [4. Pull / Merge Requests](#4-pull--merge-requests)
- [5. PR / MR Comments](#5-pr--mr-comments)
- [6. Task Field-Change History](#6-task-field-change-history)
- [7. Messenger Chat Activity](#7-messenger-chat-activity)
- [8. Extending this contract](#8-extending-this-contract)
- [Appendix A. Gaps between this contract and the current implementation](#appendix-a-gaps-between-this-contract-and-the-current-implementation)

<!-- /toc -->

## 1. Purpose

The warehouse is being split into two databases:

- **Ingestion DB** — bronze, staging, and silver layers. Owned by the ingestion pipeline (Airbyte connectors, dbt, Rust enrich binaries). Knows about vendor APIs, sync mechanics, and identity resolution.
- **Gold DB** — metric and reporting models. Owned by analytics. Knows nothing about vendors, sync mechanics, or identity resolution.

This document is the contract between them. It defines the datasets the ingestion side publishes and the gold side consumes:

| Dataset | Section | Silver table |
|---|---|---|
| `commits` | [§3](#3-commits) | `silver.class_git_commits` |
| `repository_branches` (aux dimension) | [§3.5](#35-auxiliary-dimension-repository_branches) | `silver.class_git_repository_branches` |
| `pull_requests` | [§4](#4-pull--merge-requests) | `silver.class_git_pull_requests` |
| `pull_request_comments` | [§5](#5-pr--mr-comments) | `silver.class_git_pull_requests_comments` |
| `task_field_history` | [§6](#6-task-field-change-history) | `silver.class_task_field_history` |
| `task_statuses` (aux dimension) | [§6.5](#65-auxiliary-dimension-task_statuses) | `silver.class_task_statuses` |
| `chat_activity` | [§7](#7-messenger-chat-activity) | `silver.class_collab_chat_activity` |

Rules of the boundary:

1. **Gold reads only contract datasets.** No gold model may reach into bronze, staging, or non-contract silver tables.
2. **Records arrive identity-resolved.** Every record carries a `person_id` produced by Identity Resolution *before* it crosses the boundary. Gold never resolves emails, logins, or account ids itself.
3. **The contract is versioned.** Adding a field or a case is a non-breaking change. Renaming/removing a field, changing a type, or changing the meaning of an existing field requires a new contract version and a migration window during which both shapes are published.
4. **Each dataset section ends with Cases** — the agreed interpretation rules for that data. The case list will grow; when a new question comes up ("how do I count X?"), the answer is added here as a case, not decided ad hoc inside a gold model.

## 2. Common conventions

### 2.1 Envelope fields

Every record carries a common envelope. Column names vary by dataset today (historical naming); the concept mapping:

| Concept | git datasets (§3–§5) | task_field_history (§6) | chat_activity (§7) | Description |
|---|---|---|---|---|
| Tenant | `tenant_id` | — *(gap, see Appendix A)* | `tenant_id` | Multi-tenant isolation key. Every gold query MUST scope by it. |
| Source instance | `source_id` | `insight_source_id` | `insight_source_id` | Connector instance id; distinguishes two instances of the same source type. |
| Source type | `data_source` | `data_source` | `data_source` | Discriminator: `insight_github`, `insight_gitlab`, `insight_bitbucket_cloud`, `insight_jira`, `insight_youtrack`, `insight_slack`, `insight_m365`, `insight_zulip_proxy`. |
| Record identity | `unique_key` | `unique_key` | `unique_key` | Stable logical record identity, `{tenant}-{source}-{natural_key_parts}` (per-dataset natural keys noted in each section). Exactly one logical record per `unique_key`. |
| Person | `person_id` | `person_id` | `person_id` | The resolved acting person — [§2.2](#22-person_id). |
| Version | `_version` | `_version` | `_version` | UInt64, ms epoch. When two physical rows share a `unique_key`, the greater `_version` wins. |

### 2.2 person_id

- `person_id` is the stable person UUID minted by Identity Resolution (UUIDv7 from the persons registry; minted on first observation, never re-derived).
- Semantics per dataset: the **actor** of the record — commit author, PR author, comment author, field-change author, chat-activity user.
- Resolution happens on the ingestion side (account id / email / login → person) before publishing. Gold treats `person_id` as opaque.
- `person_id` is `NULL` when the actor could not be resolved (e.g. a bot, a departed contractor with no directory record, or a source that exposes no identity fields). The default gold policy: **exclude NULL-person rows from per-person metrics; include them in team/repo totals.** A dataset case may override this.
- Raw identity fields (`author_email`, `author_name`, source account ids) are kept **for traceability and debugging only**. Gold MUST NOT join or group by them.

### 2.3 Deduplication

Contract datasets are stored as `ReplacingMergeTree(_version)` with `ORDER BY (unique_key)`; physical duplicates can exist until a background merge. Every gold read MUST dedup at read time using one of:

```sql
SELECT ... FROM <dataset> FINAL
-- or
SELECT ... FROM <dataset>
QUALIFY ROW_NUMBER() OVER (PARTITION BY unique_key ORDER BY _version DESC) = 1
```

Updates to a logical record (an edited comment, a re-synced commit) reuse the same `unique_key` with a higher `_version`; they never create a second logical record.

## 3. Commits

One record per commit per branch observation, as unioned from GitHub / GitLab / Bitbucket staging into `silver.class_git_commits`. The same change may appear as several records: on several branches, or re-created by rebase / cherry-pick with a different `commit_hash`. The `patch_id` field groups those copies — see the cases below.

### 3.1 JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "insight/contracts/silver-gold/commit.schema.json",
  "title": "Commit",
  "type": "object",
  "properties": {
    "tenant_id":       { "type": "string", "minLength": 1 },
    "source_id":       { "type": "string", "minLength": 1 },
    "data_source":     { "type": "string", "pattern": "^insight_[a-z0-9_]+$" },
    "unique_key":      { "type": "string", "minLength": 1 },
    "person_id":       { "type": ["string", "null"], "format": "uuid" },
    "_version":        { "type": "integer", "minimum": 0 },
    "project_key":     { "type": "string" },
    "repo_slug":       { "type": "string" },
    "commit_hash":     { "type": "string", "pattern": "^[0-9a-f]{7,64}$" },
    "patch_id":        { "type": ["string", "null"], "pattern": "^[0-9a-f]{40}$" },
    "branch":          { "type": "string" },
    "author_name":     { "type": "string" },
    "author_email":    { "type": "string" },
    "committer_name":  { "type": "string" },
    "committer_email": { "type": "string" },
    "message":         { "type": "string" },
    "date":            { "type": ["string", "null"], "format": "date-time" },
    "files_changed":   { "type": "integer", "minimum": 0 },
    "lines_added":     { "type": "integer", "minimum": 0 },
    "lines_removed":   { "type": "integer", "minimum": 0 },
    "is_merge_commit": { "type": "integer", "enum": [0, 1] }
  },
  "required": [
    "tenant_id", "source_id", "data_source", "unique_key", "person_id", "_version",
    "project_key", "repo_slug", "commit_hash", "patch_id", "branch",
    "author_name", "author_email", "date",
    "files_changed", "lines_added", "lines_removed", "is_merge_commit"
  ]
}
```

### 3.2 Fields

| Field | Description |
|---|---|
| `person_id` | Resolved commit **author** (not committer). |
| `project_key` | Grouping above repo: GitHub org / GitLab namespace / Bitbucket workspace. |
| `repo_slug` | Repository name within the project. |
| `commit_hash` | VCS SHA. Unique per repo, but NOT stable across rebase / cherry-pick. |
| `patch_id` | **New in this contract.** `git patch-id --stable` of the commit diff. Identical for all copies of the same change (across branches, rebases, cherry-picks). `NULL` for merge commits and empty-diff commits. |
| `branch` | The branch on which this record was observed (single branch name; a commit reachable from N branches yields N records). |
| `author_name` / `author_email` | Raw identity, traceability only. `author_email` is `""` for Bitbucket (no email in the API). |
| `committer_name` / `committer_email` | Raw identity; `""` for Bitbucket and GitLab. |
| `date` | Committer timestamp, UTC. Nullable (unparseable source timestamps become `NULL`). |
| `files_changed`, `lines_added`, `lines_removed` | Diff size stats (0 default when the source did not provide them). |
| `is_merge_commit` | `1` when the commit has more than one parent, else `0`. |

### 3.3 Example

Two records — the same change on a feature branch and, after squash-merge, on the default branch. `commit_hash` differs, `patch_id` is identical:

```json
{
  "tenant_id": "acme", "source_id": "gh-main", "data_source": "insight_github",
  "unique_key": "acme-gh-main-platform-api-8c1f2ab9",
  "person_id": "0197c2a4-7b1e-7f3a-9c2d-4a5b6c7d8e9f",
  "_version": 1752571200000,
  "project_key": "platform", "repo_slug": "api",
  "commit_hash": "8c1f2ab94e7d3c5f6a0b1d2e3f4a5b6c7d8e9f01",
  "patch_id": "3d0f5a9b8c7e6d5f4a3b2c1d0e9f8a7b6c5d4e3f",
  "branch": "feature/retry-backoff",
  "author_name": "Jane Doe", "author_email": "jane.doe@acme.com",
  "committer_name": "Jane Doe", "committer_email": "jane.doe@acme.com",
  "message": "Add retry with exponential backoff to webhook sender",
  "date": "2026-07-10T09:14:32Z",
  "files_changed": 3, "lines_added": 120, "lines_removed": 14,
  "is_merge_commit": 0
}
```

```json
{
  "tenant_id": "acme", "source_id": "gh-main", "data_source": "insight_github",
  "unique_key": "acme-gh-main-platform-api-f04d77c2",
  "person_id": "0197c2a4-7b1e-7f3a-9c2d-4a5b6c7d8e9f",
  "_version": 1752614400000,
  "project_key": "platform", "repo_slug": "api",
  "commit_hash": "f04d77c21b3a4c5d6e7f8a9b0c1d2e3f4a5b6c7d",
  "patch_id": "3d0f5a9b8c7e6d5f4a3b2c1d0e9f8a7b6c5d4e3f",
  "branch": "main",
  "author_name": "Jane Doe", "author_email": "jane.doe@acme.com",
  "committer_name": "GitHub", "committer_email": "noreply@github.com",
  "message": "Add retry with exponential backoff to webhook sender (#412)",
  "date": "2026-07-10T15:02:11Z",
  "files_changed": 3, "lines_added": 120, "lines_removed": 14,
  "is_merge_commit": 0
}
```

### 3.4 Cases

#### Case: using patch_id — statistics across all branches

**Question:** how do I count commits / lines of code without double-counting the same change that exists on several branches?

**Rule:** group commits by `patch_id` and keep only the **first** commit of each group (earliest `date`; tie-break by `commit_hash` ascending). The first commit is the original authorship event; later copies are mechanical (merges of the branch, cherry-picks, rebases). Records with `patch_id = NULL` form singleton groups — fall back to `commit_hash`.

```sql
SELECT * FROM commits FINAL
QUALIFY ROW_NUMBER() OVER (
    PARTITION BY tenant_id, coalesce(patch_id, commit_hash)
    ORDER BY date ASC, commit_hash ASC
) = 1
```

#### Case: using patch_id — statistics on the default branch only

**Question:** how do I count only work that actually landed on the default branch?

**Rule:** restrict to records observed on the repository's default branch (via the `repository_branches` dimension, [§3.5](#35-auxiliary-dimension-repository_branches)), then within each `patch_id` group keep only the **last** commit (latest `date`; tie-break by `commit_hash` ascending). The last copy is the one that finally landed (e.g. the squash or rebase result); earlier copies are superseded.

```sql
SELECT c.* FROM commits AS c FINAL
INNER JOIN repository_branches AS b FINAL
    ON  b.tenant_id  = c.tenant_id
    AND b.source_id  = c.source_id
    AND b.repo_slug  = c.repo_slug
    AND b.branch_name = c.branch
    AND b.is_default = 1
QUALIFY ROW_NUMBER() OVER (
    PARTITION BY c.tenant_id, coalesce(c.patch_id, c.commit_hash)
    ORDER BY c.date DESC, c.commit_hash ASC
) = 1
```

#### Case: merge commits

**Question:** do merge commits count as authored work?

**Rule:** no. Exclude `is_merge_commit = 1` from authored-work metrics (commit counts, LOC). Merge commits carry `patch_id = NULL` and their diff stats double the merged work. They may still be used to detect *integration events* (something landed on a branch).

### 3.5 Auxiliary dimension: repository_branches

One record per (repo × branch), from `silver.class_git_repository_branches`. Used to identify the default branch.

| Field | Type | Description |
|---|---|---|
| `tenant_id`, `source_id`, `data_source`, `unique_key`, `_version` | envelope | See [§2.1](#21-envelope-fields). |
| `project_key`, `repo_slug` | String | Repository. |
| `branch_name` | String | Branch name. |
| `is_default` | UInt8 (0/1) | `1` when this is the repository's default branch. |
| `last_commit_hash` | String | Head commit SHA of the branch. |
| `last_commit_date` | Nullable(DateTime) | Head commit timestamp. |

## 4. Pull / Merge Requests

One record per pull request (GitHub, Bitbucket) or merge request (GitLab), from `silver.class_git_pull_requests`. GitLab MRs are mapped to this shape by the producer.

### 4.1 JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "insight/contracts/silver-gold/pull_request.schema.json",
  "title": "Pull / Merge Request",
  "type": "object",
  "properties": {
    "tenant_id":          { "type": "string", "minLength": 1 },
    "source_id":          { "type": "string", "minLength": 1 },
    "data_source":        { "type": "string", "pattern": "^insight_[a-z0-9_]+$" },
    "unique_key":         { "type": "string", "minLength": 1 },
    "person_id":          { "type": ["string", "null"], "format": "uuid" },
    "_version":           { "type": "integer", "minimum": 0 },
    "project_key":        { "type": "string" },
    "repo_slug":          { "type": "string" },
    "pr_id":              { "type": "integer" },
    "pr_number":          { "type": "integer" },
    "title":              { "type": "string" },
    "description":        { "type": "string" },
    "state":              { "type": "string", "enum": ["OPEN", "MERGED", "CLOSED", "DECLINED"] },
    "author_name":        { "type": "string" },
    "author_email":       { "type": "string" },
    "source_branch":      { "type": "string" },
    "destination_branch": { "type": "string" },
    "created_on":         { "type": ["string", "null"], "format": "date-time" },
    "updated_on":         { "type": ["string", "null"], "format": "date-time" },
    "closed_on":          { "type": ["string", "null"], "format": "date-time" },
    "merge_commit_hash":  { "type": "string" },
    "files_changed":      { "type": "integer", "minimum": 0 },
    "lines_added":        { "type": "integer", "minimum": 0 },
    "lines_removed":      { "type": "integer", "minimum": 0 }
  },
  "required": [
    "tenant_id", "source_id", "data_source", "unique_key", "person_id", "_version",
    "project_key", "repo_slug", "pr_id", "pr_number", "title", "state",
    "author_name", "author_email", "source_branch", "destination_branch",
    "created_on", "closed_on", "merge_commit_hash"
  ]
}
```

### 4.2 Fields

| Field | Description |
|---|---|
| `person_id` | Resolved PR **author**. |
| `pr_id` | Source-native id. GitHub: global `database_id`; GitLab: per-project `iid`; Bitbucket: per-repo `id`. Unique only within (`source_id`, `repo_slug`) — always join with repo context. |
| `pr_number` | Human-facing number, unique within a repo. |
| `state` | Lifecycle state, uppercase. `MERGED` = accepted; `CLOSED` / `DECLINED` = rejected without merge (which one depends on the source: Bitbucket emits `DECLINED`, GitHub/GitLab emit `CLOSED`); `OPEN` = in progress. Compare case-insensitively to be safe. |
| `author_name` | Raw identity: GitHub login / Bitbucket display name / GitLab username. |
| `author_email` | Raw identity; `""` for Bitbucket (no email in the API). |
| `created_on`, `updated_on` | Lifecycle timestamps, UTC. |
| `closed_on` | Terminal timestamp. For `MERGED` PRs this **is** the merge time — there is no separate `merged_at` (GitLab: `coalesce(closed_at, merged_at)`; Bitbucket: heuristic from the closer's participation timestamp). `NULL` while `OPEN`. |
| `merge_commit_hash` | SHA of the merge/squash commit on the destination branch; `""` unless merged. Joins to `commits.commit_hash`. |
| `files_changed`, `lines_added`, `lines_removed` | Diff size. **GitLab: always `0`** (not collected) — treat `0` as "unknown", not "empty diff". |

### 4.3 Example

```json
{
  "tenant_id": "acme", "source_id": "gh-main", "data_source": "insight_github",
  "unique_key": "acme-gh-main-platform-api-pr-412",
  "person_id": "0197c2a4-7b1e-7f3a-9c2d-4a5b6c7d8e9f",
  "_version": 1752614400000,
  "project_key": "platform", "repo_slug": "api",
  "pr_id": 2210443, "pr_number": 412,
  "title": "Add retry with exponential backoff to webhook sender",
  "description": "Fixes #388. Retries 5xx with jittered backoff.",
  "state": "MERGED",
  "author_name": "jane-doe", "author_email": "jane.doe@acme.com",
  "source_branch": "feature/retry-backoff", "destination_branch": "main",
  "created_on": "2026-07-10T09:20:05Z",
  "updated_on": "2026-07-10T15:02:11Z",
  "closed_on": "2026-07-10T15:02:11Z",
  "merge_commit_hash": "f04d77c21b3a4c5d6e7f8a9b0c1d2e3f4a5b6c7d",
  "files_changed": 3, "lines_added": 120, "lines_removed": 14
}
```

### 4.4 Cases

#### Case: determining the PR outcome

**Question:** how do I tell merged, rejected, and in-progress PRs apart?

**Rule:** use `state` only. `MERGED` = accepted. `CLOSED` and `DECLINED` are both "rejected without merge" and MUST be treated identically. `OPEN` = in progress. Never infer the outcome from timestamps or `merge_commit_hash` alone.

#### Case: measuring cycle time

**Question:** how long did a PR take?

**Rule:** cycle time = `closed_on - created_on`, computed **only** for `state = 'MERGED'`. Rejected PRs are not cycle time — they are a separate rejection metric. Guard against clock skew: if `closed_on < created_on`, emit `NULL`, not a negative value.

#### Case: linking a PR to its landed commit

**Question:** how do I connect a merged PR to the commit that landed on the destination branch?

**Rule:** join `pull_requests.merge_commit_hash = commits.commit_hash` within the same `tenant_id` + `source_id`. For squash merges this is the single squashed commit; for merge-commit merges it is the merge node (`is_merge_commit = 1`).

## 5. PR / MR Comments

One record per human comment on a pull/merge request, from `silver.class_git_pull_requests_comments` — both general discussion comments and inline code-review comments. System/bot notes (GitLab "system notes") are excluded by the producer.

### 5.1 JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "insight/contracts/silver-gold/pull_request_comment.schema.json",
  "title": "Pull / Merge Request Comment",
  "type": "object",
  "properties": {
    "tenant_id":   { "type": "string", "minLength": 1 },
    "source_id":   { "type": "string", "minLength": 1 },
    "data_source": { "type": "string", "pattern": "^insight_[a-z0-9_]+$" },
    "unique_key":  { "type": "string", "minLength": 1 },
    "person_id":   { "type": ["string", "null"], "format": "uuid" },
    "_version":    { "type": "integer", "minimum": 0 },
    "project_key": { "type": "string" },
    "repo_slug":   { "type": "string" },
    "pr_id":       { "type": "integer" },
    "comment_id":  { "type": "integer" },
    "content":     { "type": "string" },
    "author_name": { "type": "string" },
    "author_uuid": { "type": "string" },
    "created_at":  { "type": ["string", "null"], "format": "date-time" },
    "updated_at":  { "type": ["string", "null"], "format": "date-time" },
    "is_inline":   { "type": "integer", "enum": [0, 1] },
    "file_path":   { "type": "string" },
    "line_number": { "type": "integer", "minimum": 0 }
  },
  "required": [
    "tenant_id", "source_id", "data_source", "unique_key", "person_id", "_version",
    "project_key", "repo_slug", "pr_id", "comment_id", "content",
    "author_name", "created_at", "is_inline", "file_path", "line_number"
  ]
}
```

### 5.2 Fields

| Field | Description |
|---|---|
| `person_id` | Resolved comment **author**. |
| `pr_id` | Joins to `pull_requests.pr_id` within the same (`source_id`, `repo_slug`). |
| `content` | Comment body (markdown as delivered by the source). |
| `author_name`, `author_uuid` | Raw source identity (login/display name + source account id), traceability only. |
| `is_inline` | `1` = attached to a diff line (code review); `0` = general discussion. |
| `file_path`, `line_number` | Meaningful only when `is_inline = 1`; `""` / `0` otherwise. |
| `updated_at` | Last edit time; edits bump `_version` on the same `unique_key`, they do not create a new record. |

### 5.3 Example

```json
{
  "tenant_id": "acme", "source_id": "gh-main", "data_source": "insight_github",
  "unique_key": "acme-gh-main-platform-api-pr-412-c-99801",
  "person_id": "0197c8d1-2e4f-7a6b-8c9d-0e1f2a3b4c5d",
  "_version": 1752582600000,
  "project_key": "platform", "repo_slug": "api",
  "pr_id": 2210443, "comment_id": 99801,
  "content": "Backoff cap of 30s seems low for webhook consumers — make it configurable?",
  "author_name": "bob-reviewer", "author_uuid": "5521907",
  "created_at": "2026-07-10T11:30:00Z", "updated_at": null,
  "is_inline": 1,
  "file_path": "src/webhooks/sender.py", "line_number": 84
}
```

### 5.4 Cases

#### Case: code-review comments vs discussion

**Question:** how do I measure code-review depth as opposed to general PR chatter?

**Rule:** `is_inline = 1` is a code-review comment (anchored to a file and line); `is_inline = 0` is discussion. Review-depth metrics count inline comments; communication metrics may count both.

#### Case: counting review participation

**Question:** who reviewed a PR?

**Rule:** a person *participated in reviewing* a PR when they authored at least one comment on it with `person_id != pull_requests.person_id` (self-comments are not review). Formal approvals (approve / request-changes verdicts) live in `silver.class_git_pull_requests_reviewers` — a candidate for a future contract dataset; do not infer approval from comments.

## 6. Task Field-Change History

The event-sourced history of task/issue fields, produced by the **enrich** step (Rust `jira-enrich`), from `silver.class_task_field_history`. One record per **(issue × field × event)**. This is the only task dataset gold needs for lifecycle metrics: current state, state at any time T, transitions, and durations are all derivable from it.

`unique_key` formula: `{insight_source_id}-{data_source}-{id_readable}-{field_id}-{event_id}`.

### 6.1 JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "insight/contracts/silver-gold/task_field_history.schema.json",
  "title": "Task Field-Change Event",
  "type": "object",
  "properties": {
    "insight_source_id":   { "type": "string", "minLength": 1 },
    "data_source":         { "type": "string", "pattern": "^insight_[a-z0-9_]+$" },
    "unique_key":          { "type": "string", "minLength": 1 },
    "person_id":           { "type": ["string", "null"], "format": "uuid" },
    "_version":            { "type": "integer", "minimum": 0 },
    "issue_id":            { "type": "string" },
    "id_readable":         { "type": "string" },
    "event_id":            { "type": "string" },
    "event_at":            { "type": "string", "format": "date-time" },
    "event_kind":          { "type": "string", "enum": ["changelog", "synthetic_initial"] },
    "_seq":                { "type": "integer", "minimum": 0 },
    "author_id":           { "type": ["string", "null"] },
    "author_display":      { "type": ["string", "null"] },
    "field_id":            { "type": "string" },
    "field_name":          { "type": "string" },
    "field_cardinality":   { "type": "string", "enum": ["single", "multi"] },
    "delta_action":        { "type": "string", "enum": ["set", "add", "remove"] },
    "delta_value_id":      { "type": ["string", "null"] },
    "delta_value_display": { "type": ["string", "null"] },
    "value_ids":           { "type": "array", "items": { "type": "string" } },
    "value_displays":      { "type": "array", "items": { "type": "string" } },
    "value_id_type":       { "type": "string", "enum": ["opaque_id", "account_id", "string_literal", "path", "none"] },
    "collected_at":        { "type": "string", "format": "date-time" }
  },
  "required": [
    "insight_source_id", "data_source", "unique_key", "person_id", "_version",
    "issue_id", "id_readable", "event_id", "event_at", "event_kind", "_seq",
    "field_id", "field_name", "field_cardinality", "delta_action",
    "value_ids", "value_displays", "value_id_type", "collected_at"
  ]
}
```

### 6.2 Fields

| Field | Description |
|---|---|
| `person_id` | Resolved **change author**: for `changelog` events, who made the change; for `synthetic_initial` events, the issue **reporter** (creation-time values are attributed to the reporter). |
| `issue_id` / `id_readable` | Source-native id and human-readable key (`PROJ-123`). |
| `event_id` | Source changelog entry id, or a synthetic id for `synthetic_initial` rows. |
| `event_at` | When the change happened (for synthetic rows: issue creation time). |
| `event_kind` | `changelog` = a real recorded change. `synthetic_initial` = a reconstructed creation-time baseline row — see the first case below. |
| `_seq` | Deterministic tie-break within one `event_at`: `changelog` rows have `_seq = 0`; `synthetic_initial` rows are numbered `1..N` (field-id order). Order events by `(event_at, _seq, _version)`. |
| `author_id`, `author_display` | Raw source identity (e.g. Atlassian accountId), traceability only. |
| `field_id` / `field_name` | Which field changed (`status`, `assignee`, `labels`, …). The sentinel `field_id = "created"` marks the issue-creation event. |
| `field_cardinality` | `single` (status, assignee) vs `multi` (labels, components). |
| `delta_action` | What the change did: `set` (replace, single-value fields), `add` / `remove` (multi-value fields). |
| `delta_value_id` / `delta_value_display` | The changed value itself (id and display name). |
| `value_ids` / `value_displays` | **Full field value after this event** — the enriched running state. Empty arrays mean the field became empty. For `field_id = 'status'`, `value_ids[1]` joins to `task_statuses.status_id` ([§6.5](#65-auxiliary-dimension-task_statuses)). |
| `value_id_type` | What kind of identifier `value_ids` holds: `opaque_id` (status/priority ids), `account_id` (assignee/reporter), `string_literal` (labels), `path`, `none`. |
| `collected_at` | When the enrich run produced this row (`_version` = its ms epoch). |

### 6.3 Example

Three events for one issue: the creation marker, the initial status baseline, and a real status transition:

```json
{
  "insight_source_id": "jira-main", "data_source": "insight_jira",
  "unique_key": "jira-main-insight_jira-PROJ-123-created-synth-0",
  "person_id": "0197c2a4-7b1e-7f3a-9c2d-4a5b6c7d8e9f",
  "_version": 1752571200000,
  "issue_id": "10500", "id_readable": "PROJ-123",
  "event_id": "synth-0", "event_at": "2026-07-01T08:00:00Z",
  "event_kind": "synthetic_initial", "_seq": 0,
  "author_id": "557058:aa11bb22", "author_display": "Jane Doe",
  "field_id": "created", "field_name": "created", "field_cardinality": "single",
  "delta_action": "set", "delta_value_id": null, "delta_value_display": null,
  "value_ids": [], "value_displays": [], "value_id_type": "none",
  "collected_at": "2026-07-16T02:00:00Z"
}
```

```json
{
  "insight_source_id": "jira-main", "data_source": "insight_jira",
  "unique_key": "jira-main-insight_jira-PROJ-123-status-synth-1",
  "person_id": "0197c2a4-7b1e-7f3a-9c2d-4a5b6c7d8e9f",
  "_version": 1752571200000,
  "issue_id": "10500", "id_readable": "PROJ-123",
  "event_id": "synth-1", "event_at": "2026-07-01T08:00:00Z",
  "event_kind": "synthetic_initial", "_seq": 1,
  "author_id": "557058:aa11bb22", "author_display": "Jane Doe",
  "field_id": "status", "field_name": "Status", "field_cardinality": "single",
  "delta_action": "set", "delta_value_id": "10000", "delta_value_display": "To Do",
  "value_ids": ["10000"], "value_displays": ["To Do"], "value_id_type": "opaque_id",
  "collected_at": "2026-07-16T02:00:00Z"
}
```

```json
{
  "insight_source_id": "jira-main", "data_source": "insight_jira",
  "unique_key": "jira-main-insight_jira-PROJ-123-status-45021",
  "person_id": "0197c8d1-2e4f-7a6b-8c9d-0e1f2a3b4c5d",
  "_version": 1752671100000,
  "issue_id": "10500", "id_readable": "PROJ-123",
  "event_id": "45021", "event_at": "2026-07-12T16:45:00Z",
  "event_kind": "changelog", "_seq": 0,
  "author_id": "557058:cc33dd44", "author_display": "Bob Smith",
  "field_id": "status", "field_name": "Status", "field_cardinality": "single",
  "delta_action": "set", "delta_value_id": "10002", "delta_value_display": "Готово",
  "value_ids": ["10002"], "value_displays": ["Готово"], "value_id_type": "opaque_id",
  "collected_at": "2026-07-16T02:00:00Z"
}
```

### 6.4 Cases

#### Case: enriched events vs raw field-change deltas

**Question:** how do the records in this dataset differ from the raw changelog deltas the source APIs return, and which part do I use?

**Rule:** a raw delta only says *what changed* (from → to), exists only for fields somebody touched, and carries no state. The enrich step turns deltas into **events** that add three things:

1. **Running state.** Every event carries the *complete field value after the event* in `value_ids` / `value_displays` — for multi-value fields the whole resulting set, not just the added/removed element. Gold never folds deltas to reconstruct state; enrich already did it.
2. **Creation baseline.** `synthetic_initial` events reconstruct the value of *every* field at issue-creation time — including fields never touched by any changelog — plus the `field_id = "created"` marker event. So every field has a complete timeline from creation, and "state at time T" is always answerable.
3. **Deterministic order.** `(event_at, _seq, _version)` totally orders the history.

Use `value_ids` / `value_displays` when you need state (current status, assignee at time T). Use `delta_action` / `delta_value_*` when you need the change itself (what was added to labels, who flipped the status). Use `event_kind` to separate real activity (`changelog`) from reconstructed baseline (`synthetic_initial`) — e.g. "number of status changes" counts only `changelog` events.

#### Case: determining that a task is closed

**Question:** how do I tell a task is closed, and when it was closed?

**Rule:** a task is closed iff the **category of its current status is `done`**. Never match status display names (`Closed`, `Done`, `Готово`, …) — names are workflow- and locale-specific. Join the status id (`value_ids[1]`) to the `task_statuses` dimension ([§6.5](#65-auxiliary-dimension-task_statuses)) to get the source-neutral `status_category` (Jira `statusCategory.key`, YouTrack `isResolved`). The close timestamp is the `event_at` of the latest event that put the task into a done-category status.

```sql
WITH status_events AS (
    SELECT fh.insight_source_id, fh.id_readable,
           fh.event_at, fh._seq, fh._version,
           st.status_category
    FROM task_field_history AS fh FINAL
    LEFT JOIN task_statuses AS st FINAL
        ON  st.insight_source_id = fh.insight_source_id
        AND st.status_id = fh.value_ids[1]
    WHERE fh.field_id = 'status'
)
SELECT id_readable,
       argMax(status_category, (event_at, _seq, _version)) AS current_category,
       maxIf(event_at, status_category = 'done')           AS closed_at
FROM status_events
GROUP BY insight_source_id, id_readable
HAVING current_category = 'done'
```

Note the two-part semantics: a task that passed through a done status but was later reopened is **not closed** (fails `HAVING`); a task that was reopened and closed again is closed at the *latest* transition into done.

#### Case: detecting reopens

**Question:** how do I count reopened tasks?

**Rule:** order the `field_id = 'status'` events of an issue by `(event_at, _seq, _version)`, resolve each event's `status_category` via the `task_statuses` join, and compare adjacent events: a **reopen** is a transition from `done` to any non-`done` category; a **close** is a transition from non-`done` to `done`. Use window `lagInFrame(status_category)` per issue.

#### Case: field value at an arbitrary time T

**Question:** what was the assignee (or any field) of a task on a given date?

**Rule:** take the latest event for that `(issue, field)` with `event_at <= T`, ordered by `(event_at, _seq, _version)`; its `value_ids` / `value_displays` are the answer. Thanks to `synthetic_initial` baselines this always yields a value for any `T >=` issue creation; if no event qualifies, the issue did not exist yet at `T`.

#### Case: task creation time

**Question:** when was a task created?

**Rule:** the `event_at` of its `field_id = 'created'` event (`event_kind = 'synthetic_initial'`, `_seq = 0`). Do not use `min(event_at)` across all fields — it is equal today but is not the contract.

### 6.5 Auxiliary dimension: task_statuses

One record per (source × status id), from `silver.class_task_statuses`. Maps workflow statuses to the locale-independent category used for close detection.

| Field | Type | Description |
|---|---|---|
| `insight_source_id`, `data_source`, `unique_key`, `_version` | envelope | See [§2.1](#21-envelope-fields). |
| `status_id` | String | Status id; joins `task_field_history.value_ids[1]` for `field_id = 'status'` events. |
| `status_name` | String | Display name (locale/workflow-specific — never used for logic). |
| `category_id`, `category_key` | Nullable | Raw vendor category (Jira `statusCategory.id` / `.key`). |
| `status_category` | String | Unified category: `new`, `in_progress`, `done`, `undefined`. Jira: from `statusCategory.key`; YouTrack: from the State bundle's `isResolved` flag. |
| `collected_at` | DateTime | Sync time. |

Example:

```json
{
  "insight_source_id": "jira-main", "data_source": "insight_jira",
  "unique_key": "jira-main-insight_jira-status-10002",
  "_version": 1752671100000,
  "status_id": "10002", "status_name": "Готово",
  "category_id": "3", "category_key": "done",
  "status_category": "done",
  "collected_at": "2026-07-16T02:00:00Z"
}
```

## 7. Messenger Chat Activity

What exists today for messengers is **not per-message data**. Vendor *analytics/report* endpoints provide only **daily per-user activity counters**; that is what `silver.class_collab_chat_activity` holds and what this contract fixes: one record per (user × date × source), covering Slack, MS Teams (M365), and Zulip. There is no channel/thread/message-level structure, no message content, and no edit/delete signal.

Several columns are structurally `NULL` depending on the vendor (each vendor reports a different subset of counters) — see the honest-NULL case below. A per-message dataset is a **future extension** ([§7.5](#75-future-per-message-dataset)).

### 7.1 JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "insight/contracts/silver-gold/chat_activity.schema.json",
  "title": "Daily Chat Activity",
  "type": "object",
  "properties": {
    "tenant_id":                 { "type": "string", "minLength": 1 },
    "insight_source_id":         { "type": "string", "minLength": 1 },
    "data_source":               { "type": "string", "enum": ["insight_slack", "insight_m365", "insight_zulip_proxy"] },
    "unique_key":                { "type": "string", "minLength": 1 },
    "person_id":                 { "type": ["string", "null"], "format": "uuid" },
    "_version":                  { "type": "integer", "minimum": 0 },
    "user_id":                   { "type": "string" },
    "user_name":                 { "type": "string" },
    "email":                     { "type": "string" },
    "person_key":                { "type": "string" },
    "date":                      { "type": "string", "format": "date" },
    "direct_messages":           { "type": ["integer", "null"], "minimum": 0 },
    "group_chat_messages":       { "type": ["integer", "null"], "minimum": 0 },
    "direct_and_group_messages": { "type": ["integer", "null"], "minimum": 0 },
    "total_chat_messages":       { "type": "integer", "minimum": 0 },
    "channel_posts":             { "type": ["integer", "null"], "minimum": 0 },
    "channel_replies":           { "type": ["integer", "null"], "minimum": 0 },
    "urgent_messages":           { "type": ["integer", "null"], "minimum": 0 },
    "report_period":             { "type": ["string", "null"] },
    "collected_at":              { "type": "string", "format": "date-time" }
  },
  "required": [
    "tenant_id", "insight_source_id", "data_source", "unique_key", "person_id",
    "_version", "user_id", "person_key", "date", "total_chat_messages"
  ]
}
```

### 7.2 Fields

| Field | Description |
|---|---|
| `person_id` | Resolved person for this activity row. |
| `user_id` | Raw source user id: Slack user id; M365 `userPrincipalName`; Zulip lowercased email. Traceability only. |
| `user_name`, `email`, `person_key` | Raw identity. `person_key = lower(email)` (fallback `lower(user_id)`) — the legacy cross-source join key; superseded by `person_id`, kept for traceability. |
| `date` | Activity date, UTC. |
| `total_chat_messages` | Headline count: all messages the user sent that day, all surfaces combined. The only counter guaranteed non-NULL across vendors. |
| `direct_messages` | 1:1 chat messages. M365 only; `NULL` for Slack/Zulip. |
| `group_chat_messages` | Group-chat messages. Currently `NULL` for all vendors. |
| `direct_and_group_messages` | DMs + group chats combined. Slack and M365; `NULL` for Zulip. |
| `channel_posts` | New top-level channel posts. Slack and M365; `NULL` for Zulip. |
| `channel_replies` | Channel thread replies. M365 only. |
| `urgent_messages` | Messages marked urgent. M365 only. |
| `report_period` | Vendor report window marker (M365 only). |
| `collected_at` | Sync time. |

### 7.3 Example

```json
{
  "tenant_id": "acme", "insight_source_id": "m365-main", "data_source": "insight_m365",
  "unique_key": "9c3f2e1d0b8a7654fedcba9876543210",
  "person_id": "0197c8d1-2e4f-7a6b-8c9d-0e1f2a3b4c5d",
  "_version": 1752658201000,
  "user_id": "bob.smith@acme.com", "user_name": "bob.smith@acme.com",
  "email": "bob.smith@acme.com", "person_key": "bob.smith@acme.com",
  "date": "2026-07-15",
  "direct_messages": 12, "group_chat_messages": null,
  "direct_and_group_messages": 12, "total_chat_messages": 19,
  "channel_posts": 4, "channel_replies": 3, "urgent_messages": 0,
  "report_period": "7", "collected_at": "2026-07-16T02:00:00Z"
}
```

### 7.4 Cases

#### Case: counting messaging activity

**Question:** how many messages did a person send in a period?

**Rule:** sum `total_chat_messages` per `person_id` per `date`. It is the only counter comparable across vendors. Do not add `direct_* + channel_*` yourself — the vendor-specific counters overlap differently per vendor (`total ≈ dm + channel` holds only approximately).

#### Case: honest NULLs in vendor counters

**Question:** a counter is `NULL` — is that zero activity?

**Rule:** no. `NULL` means **the vendor does not report this counter**, not "zero". When aggregating a nullable counter, keep the distinction: emit `NULL` when *all* contributing rows are `NULL`, and a sum otherwise (`if(countIf(x IS NOT NULL) > 0, sumIf(x, x IS NOT NULL), NULL)`). Never `coalesce(x, 0)` a vendor counter — it silently converts "not measured" into "inactive".

#### Case: direct vs channel communication split

**Question:** how do I split DM load from channel communication?

**Rule:** the split is only available where the vendor reports it: use `direct_and_group_messages` (Slack, M365) vs `channel_posts + channel_replies` (with the honest-NULL rule). Zulip rows support only the total. Report the split per-vendor or restrict to sources that provide it — do not fabricate a split from the total.

### 7.5 Future: per-message dataset

When source permissions allow message-level ingestion (Slack `*:history` scopes, MS Graph `Chat.Read.All`), a `messages` dataset will be added: one record per message with `message_id`, `channel_id`/`channel_type` (`channel` / `dm` / `group_dm`), `thread_id` + `is_thread_reply`, `sent_at` / `edited_at` / `is_deleted` (tombstone), sender `person_id` — **without message content**. Until then, all messenger metrics must be expressible over the daily aggregates above.

## 8. Extending this contract

- **New case** (interpretation rule): append a `#### Case:` block to the relevant dataset section — format: *Question / Rule / optional SQL sketch*. This is a non-breaking, documentation-only change, but it is normative once merged: gold models must follow it.
- **New field:** add to the JSON Schema (optional first), the field table, and the example. Non-breaking.
- **New dataset:** new top-level section with the same structure (schema, fields, example, cases). Next candidates: PR reviewers/approvals (`class_git_pull_requests_reviewers`), per-message chat data ([§7.5](#75-future-per-message-dataset)).
- **Breaking change** (rename, retype, semantics change): requires a contract version bump and a dual-publish migration window.

## Appendix A. Gaps between this contract and the current implementation

The schemas above match the current silver tables; the items below are where the contract demands more than silver delivers today (2026-07-16):

| Gap | Detail |
|---|---|
| `person_id` not attached anywhere | No dataset carries a resolved `person_id` today. Git models expose interim `person_key` (lowercased email/login) on `fct_git_*` only; task gold joins `class_task_users` by email at read time; chat rows carry `person_key`. The Identity Resolution machinery (persons registry, UUIDv7, `identity_inputs`) exists but git connectors emit no `identity_inputs` at all; comment authors are never resolved. |
| `patch_id` missing | No git connector computes `git patch-id`. Commit identity today is `commit_hash` only, so cross-branch dedup falls back to hash equality and misses rebases/cherry-picks. |
| `task_field_history` has no tenant column | The table (and its `unique_key` formula) carries `insight_source_id` but no `insight_tenant_id`. Single-tenant-safe today; the DB split makes a tenant column mandatory. |
| No `merged_by` / `closed_by` on PRs | Only the author identity is captured; Bitbucket's `closed_by` is used internally for the `closed_at` heuristic but not surfaced. |
| GitLab PR diff stats | `files_changed` / `lines_added` / `lines_removed` are constant `0` for GitLab MRs (not collected). |
| Per-message data absent | Only daily per-user aggregates exist (vendor analytics endpoints). Message-level ingestion requires new source scopes — [§7.5](#75-future-per-message-dataset). |
