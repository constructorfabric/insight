---
status: accepted
date: 2026-04-23
---

# `activitiesPage` cursor pagination (not offset)


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option 1 — `CursorPagination` (afterCursor / hasAfter)](#option-1--cursorpagination-aftercursor--hasafter)
  - [Option 2 — `OffsetIncrement` ($skip / $top)](#option-2--offsetincrement-skip--top)
  - [Option 3 — Custom Python CDK](#option-3--custom-python-cdk)
  - [Option 4 — Issue-level snapshot only](#option-4--issue-level-snapshot-only)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-youtrack-activitiespage-cursor`
## Context and Problem Statement

YouTrack's per-issue activity stream is exposed at `GET /api/issues/{id}/activitiesPage`. This endpoint returns one row per field change, comment lifecycle event, attachment change, link edit, etc. — paginated **backwards in time** (newest first) when `reverse=true` is set.

Unlike most YouTrack REST endpoints, `activitiesPage` does not support stable offset pagination via `$skip` / `$top`. The response shape carries `afterCursor` (opaque token) and `hasAfter` (boolean) instead. Walking the full activity timeline of an issue requires following these cursor tokens.

Every other YouTrack stream in this connector uses offset pagination (`$skip` / `$top`) — directories, issues themselves, comments, worklogs, project custom fields. Mixing pagination strategies in one manifest is supported by the Airbyte declarative CDK but adds documentation burden and a per-stream review surface.

## Decision Drivers

- Correctness — activity timeline must be complete and gap-free for downstream replay (future scope §2.6 backward-replay engine).
- API support — offset against `activitiesPage` is not viable; YouTrack returns inconsistent results across page boundaries.
- Resume on partial failure — cursor tokens are deterministic per partition, allowing safe re-tries.
- Manifest readability — having one pagination shape per stream is clearer than per-page logic.
- Builder-UI compatibility — both `OffsetIncrement` and `CursorPagination` are first-class declarative components.

## Considered Options

1. **`CursorPagination` (afterCursor / hasAfter)** — use the endpoint's native cursor.
2. **`OffsetIncrement` ($skip / $top)** — attempt offset, accept potential duplication / gaps.
3. **Custom Python CDK** — write a custom paginator with retry / dedup logic.
4. **Issue-level snapshot** — skip the activity history entirely and reconstruct from current state.

## Decision Outcome

Chosen option: **`CursorPagination`** (Option 1), because it is the only correct pagination for this endpoint. `cursor_value: "{{ response.afterCursor }}"`, `stop_condition: "{{ response.hasAfter is false }}"`, paired with `categories` query whitelist to trim chatter.

The cursor pagination is declared inline in `connector.yaml` per the Builder-UI rule (no whole-object `$ref`). Page size is templated from `config.get('youtrack_activities_page_size', 200)` so operators can throttle the cursor walk when YouTrack returns 429 for very active projects.

The 23-category whitelist (`CustomFieldCategory`, `SummaryCategory`, `CommentsCategory`, `LinksCategory`, `TimeTrackingCategory`, `IssueResolvedCategory`, etc.) reflects the v1/v2 donor research from `monitor` repo `packages/cli/commands/youTrack/fields/IssueActivities.ts`. The categories the connector requests are precisely those needed for the future `class_task_field_history` replay (event-sourcing reconstruction). Categories like `AttachmentsCategory` and `IssueWatcherCategory` are included but only for future analytics.

### Consequences

**Positive**:

- Correct, gap-free activity timeline for every issue.
- Stable resume on partial sync failure — `afterCursor` is deterministic per partition.
- Same `CursorPagination` declarative component used by other connectors in the repo (consistency for reviewers).

**Negative**:

- Manifest mixes two pagination strategies (offset for most streams, cursor for `youtrack_issue_history`). Mitigated by documentation in the connector README and per-stream component docs.
- Cursor tokens are opaque — debugging mid-walk failures requires capturing `afterCursor` from logs.
- The 23-category whitelist is encoded as a static string in the manifest. If YouTrack adds a new category, the manifest must be updated. Mitigated by Phase 1 research evidence that new categories are rare (no additions in the v1 → v2 donor span of ~3 years).

**Rejected — Option 2 (offset)**: YouTrack returns duplicate rows across `$skip` boundaries on `activitiesPage` for high-throughput issues, and silently drops rows in some cases. Tested in Phase 1; not viable.

**Rejected — Option 3 (custom CDK)**: Adds Python code that has to be maintained. The declarative `CursorPagination` component handles the same semantics with no custom code. Future scope (§2.6 enrich core) needs Rust; not a Python CDK.

**Rejected — Option 4 (current-state only)**: Loses all historical context — `class_task_field_history` becomes impossible. Critical analytics (cycle time, status periods) become impossible.

### Confirmation

Decision is confirmed when:

- `youtrack_issue_history` stream in `connector.yaml` declares `CursorPagination` with `cursor_value: "{{ response.afterCursor }}"` and `stop_condition: "{{ response.hasAfter is false }}"`.
- The 23-category whitelist appears in the request `categories` parameter.
- `source.sh read task-tracking/youtrack youtrack_issue_history <tenant>` walks one or more `afterCursor` pages without errors.
- PR #227's verification table shows non-zero record counts for `youtrack_issue_history` across multiple pages (verified — 55 310 rows in PR #227).

## Pros and Cons of the Options

### Option 1 — `CursorPagination` (afterCursor / hasAfter)

- **Pros**: Native shape, stable resume, no duplicates / gaps, same declarative component used elsewhere in the repo.
- **Cons**: Mixed pagination strategies in one manifest. Mitigated by docs in connector README.

### Option 2 — `OffsetIncrement` ($skip / $top)

- **Pros**: Consistent with the rest of the manifest.
- **Cons**: YouTrack returns duplicates / gaps for high-throughput issues. Not viable.

### Option 3 — Custom Python CDK

- **Pros**: Full control over retry / dedup / batching.
- **Cons**: Adds Python code that must be maintained. The declarative `CursorPagination` covers the same semantics with no custom code.

### Option 4 — Issue-level snapshot only

- **Pros**: Connector simpler.
- **Cons**: Loses historical context — `class_task_field_history` and cycle-time analytics become impossible. Effectively kills the use case.

## More Information

- YouTrack REST API reference for `/api/issues/{id}/activitiesPage`: <https://www.jetbrains.com/help/youtrack/devportal/api-issue-activitiesPage.html>.
- v1/v2 donor enumeration of activity categories: `monitor` repo `packages/cli/commands/youTrack/fields/IssueActivities.ts`.

## Traceability

- Implements PRD `cpt-insightspec-fr-youtrack-stream-activities-cursor`.
- Implements DESIGN `cpt-insightspec-principle-youtrack-cursor-for-activities`, `cpt-insightspec-constraint-youtrack-activitiespage-cursor`.
- Drives future Enrich ADR-001 (activitiesPage event-sourcing with backward replay) — the captured activity stream is the input to the replay engine.
