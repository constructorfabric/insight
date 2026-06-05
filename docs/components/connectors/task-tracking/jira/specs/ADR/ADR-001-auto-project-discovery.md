---
status: accepted
date: 2026-06-02
---

# Auto-discovery of Jira projects via SubstreamPartitionRouter

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option 1 — Static allowlist in K8s Secret](#option-1--static-allowlist-in-ks-secret)
  - [Option 2 — Auto-discovery via SubstreamPartitionRouter](#option-2--auto-discovery-via-substreampartitionrouter)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-jira-auto-project-discovery`

## Context and Problem Statement

The original Jira connector required a `jira_project_keys` field in the K8s Secret — a comma-separated list of Jira project keys to sync (e.g., `TC,TNG`). The rationale at the time was that "Jira Cloud rejects unbounded JQL queries."

In practice this created operational pain:

- Jira projects are frequently created and archived. Every change required a manual Secret edit and connector re-trigger.
- The "unbounded query" concern was a misconception: Jira Cloud's API restriction applies to queries with *no* bounds at all. The connector already queries in 30-day windows (`step: P30D`), which is a valid temporal bound. Queries bounded by project key *and* time window are equivalent in cost and reliability to queries bounded by time window alone.
- The `jira_project_keys` allowlist diverges from the token's Browse Projects permission boundary. Rotating the token without updating the allowlist silently keeps ingesting a stale project set — or silently drops new projects.

## Decision Drivers

- Operational simplicity: no manual maintenance when projects are added or archived.
- Permission-boundary alignment: the Jira API token already constrains what the connector can read.
- Consistency with YouTrack: YouTrack ADR-003 already chose full-ingestion for the same reasons.
- Correctness: per-project partitioning gives per-project incremental cursor state, which is strictly better than a single global cursor.

## Considered Options

1. **Static allowlist in K8s Secret** (`jira_project_keys`) — keep the current behaviour.
2. **Auto-discovery via SubstreamPartitionRouter** — query `GET /rest/api/3/project/search` at the start of each sync to obtain all accessible project keys; partition `jira_issue` over those keys.

## Decision Outcome

Chosen option: **auto-discovery via SubstreamPartitionRouter** (Option 2).

The connector now issues one JQL request per project partition:

```sql
project = "<KEY>" AND updated >= "<t_start>" AND updated <= "<t_end>"
ORDER BY updated ASC
```

Project scope is delegated to Jira's Browse Projects permission on the API token. To limit ingestion to a specific subset of projects, operators scope the token in Jira — not in Insight config.

The `jira_project_keys` field is removed from `spec.connection_specification.required`, `spec.connection_specification.properties`, `descriptor.yaml:secret.required_fields`, and `jira.yaml.example`.

### Consequences

**Positive**:

- New or renamed projects are ingested automatically on the next scheduled sync without any config change.
- Archived/deleted projects disappear from `/project/search` and are no longer queried — no stale state.
- Per-project incremental cursor state: each project advances independently; a new project backfills from `jira_start_date` without affecting other projects.
- Jira and YouTrack connectors converge on the same architecture (full-ingestion, token-scoped).
- Eliminates the class of drift bugs where `jira_project_keys` is stale relative to the token scope.

**Negative**:

- Operators who want to limit ingestion to a subset of projects must manage token permissions in Jira rather than in Insight config. This is a one-time workflow change.
- Syncing more projects costs more Jira API calls. For instances with hundreds of projects this adds latency to the directory-discovery phase. Jira rate-limits (429/503) are handled by the existing `Retry-After` backoff strategy.
- The parent stream (`jira_project_discovery`) makes an additional `/project/search` call every sync run. This is negligible — project lists are small (typically < 200) and fully paginated in one or two requests.

**State migration**:

Existing connections lose their per-source incremental cursor state on upgrade. The connector will perform a full re-sync from `jira_start_date` on the first run after the change. Acceptable for current deployments (dev stage; data has no production value).

### Confirmation

Decision is confirmed when:

- `connector.yaml`'s `jira_issue.retriever` contains a `SubstreamPartitionRouter` whose parent stream queries `/rest/api/3/project/search`.
- `connector.yaml`'s shared JQL uses `project = "{{ stream_slice.project_key }}"` (not `project IN (...)`).
- `connector.yaml`'s `spec.connection_specification.required` does **not** list `jira_project_keys`.
- `descriptor.yaml:secret.required_fields` does **not** list `jira_project_keys`.
- `jira.yaml.example` does **not** contain a `jira_project_keys` entry.

## Pros and Cons of the Options

### Option 1 — Static allowlist in K8s Secret

- **Pros**: Explicit scope visible in config; operators control exactly which projects are ingested from Insight UI.
- **Cons**: Manual maintenance on every project change; allowlist can drift from token scope; blocks automatic onboarding of new projects; diverges from YouTrack architecture.

### Option 2 — Auto-discovery via SubstreamPartitionRouter

- **Pros**: Zero maintenance; auto-pickup of new projects; per-project cursor state; consistent with YouTrack; permission boundary is the token (single source of truth).
- **Cons**: Scope management moves to Jira token administration; proportional API cost on large instances.

## More Information

- Jira Cloud Project Search API: `GET /rest/api/3/project/search` — supports `startAt`/`maxResults` offset pagination; returns all projects accessible to the authenticated user.
- YouTrack equivalent decision: `docs/components/connectors/task-tracking/youtrack/specs/ADR/ADR-003-no-whitelist-full-ingestion.md`.
- Airbyte CDK `SubstreamPartitionRouter`: combines with `DatetimeBasedCursor` via cartesian product — each `(project_key, time_window)` pair becomes a request.

## Traceability

- Supersedes the `jira_project_keys` design documented in DESIGN §3.3 and PRD §3.1.
- Mirrors YouTrack ADR-003 decision; Jira and YouTrack now converge on full-ingestion scope.
- Implementation PR: `feat/jira-auto-project-discovery`.

## Known Behaviors

### Archived projects

`GET /rest/api/3/project/search` returns **all project types** including archived projects by default (Jira Cloud does not filter by `status=live` unless explicitly requested). In practice archived projects return 0 issues via JQL and contribute negligible API cost. If this becomes a concern a follow-up can add `status=live` to the `jira_project_discovery` parent stream's `request_parameters` — that is a backwards-compatible, non-breaking change.

### Issue pagination (`/rest/api/3/search/jql`)

The connector currently uses an `OffsetIncrement` paginator (`startAt` / `maxResults`) against `/rest/api/3/search/jql`. Atlassian's enhanced-JQL endpoint supports cursor-based pagination via `nextPageToken` (already used by the connector's own `CursorPagination` paginator config). The two mechanisms coexist on this endpoint — offset pagination is functional but will be replaced with `nextPageToken`-based pagination in a follow-up to align with Atlassian's recommended approach for large result sets.
