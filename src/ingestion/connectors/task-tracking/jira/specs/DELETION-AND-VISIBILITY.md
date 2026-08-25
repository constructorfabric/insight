# Jira Deletion & Visibility Mechanism

Status: implemented. Part of the data-completeness scope of issue #2419.

## Problem

The ingestion pipeline is incremental and append-only:

- Incremental streams query `updated >= <cursor>`. A deleted Jira entity never
  matches such a query again — it does not "update", it stops existing. Its
  last known version stays in Bronze (and everything downstream) as if alive.
- Jira's changelog records field-value changes only. Entity deletion is not a
  changelog event; deleting an issue destroys its changelog entirely.
- Bronze is `ReplacingMergeTree` keyed by `unique_key`: rows are versioned and
  replaced, never compared against "what the API returned last time". The
  pipeline has no step that observes *absence*.

The same blind spot hides a different, non-destructive cause: the service
account can lose **Browse Projects** on a project (permission change, project
archived or moved to trash). Every entity of that project disappears from the
API identically to deletion. The two must not be conflated.

## Principles

1. **Nothing is ever physically deleted from the warehouse.** Bronze rows,
   snapshots, and history are retained forever. Deletion is recorded as an
   *event in the entity's history*, exactly like any field change.
2. **Absence must be observed, then classified.** An entity disappearing from
   the API is a fact; *why* it disappeared is an inference. When the cause
   cannot be determined, the event gets an honest name (`unobserved`) rather
   than a guessed one.
3. **Access is assumed stable.** Losing Browse permission is the exceptional
   path; the classifier still detects it, but thresholds are tuned for the
   normal world where a disappearance inside a visible project is a deletion.

## Mechanism overview

```text
connector (per sync)                      dbt (per run)
────────────────────                      ─────────────────────────────────────
jira_project_visibility  ──────────────►  jira__project_visibility_state
  full roster of projects per status        is_visible per ever-seen project
  (live / archived / deleted)             ► jira__project_visibility_snapshot   (SCD2, snapshot())
                                          ► jira__project_visibility_history    (events, fields_history())

jira_issue_census        ──────────────►  jira__issue_availability_state
  id-only sweep of ALL issues               availability per ever-seen issue
  in every visible live project           ► jira__issue_availability_snapshot   (SCD2, snapshot())
                                          ► jira__issue_availability_history    (events, fields_history())
                                          ► jira__availability_events           (staging → silver)
                                            └─► silver.class_task_field_history (union_by_tag,
                                                 event_kind='availability')
                                                 └─► gold task models filter on it
```

Both new streams are **full-refresh censuses**: each sync re-observes the
complete set. Bronze RMT promotion (`ReplacingMergeTree(_airbyte_extracted_at)`
keyed by `unique_key`) collapses them to one row per entity whose
`_airbyte_extracted_at` is the **last time the entity was observed**. Absence
detection is then a comparison of each entity's last-seen timestamp against
the census high-water mark — no row diffing, no state files.

## Streams

### `jira_project_visibility`

Roster of every project the service account can currently see, per lifecycle
status.

- Endpoint: `GET /rest/api/3/project/search?status=<s>` partitioned over
  `s ∈ {live, archived, deleted}` (`deleted` = in Jira's project trash).
  Presence in the response *is* the Browse permission check: `project/search`
  returns only projects the caller can browse, so no separate
  `permissions/project` call is needed.
- Sync mode: full refresh, every sync. Cost: one page per ~50 projects per
  status — negligible.
- Record: `project_id`, `project_key`, `project_status` (stamped from the
  partition), `is_private`, plus the standard identity stamp.
  `unique_key = {tenant}-{source}-{project_id}` — keyed by immutable numeric
  id, not by key (project keys can be renamed).

### `jira_issue_census`

Id-only sweep of **all** issues in every visible live project.

- Endpoint: `GET /rest/api/3/search/jql` with `fields=id` and
  `maxResults=5000`. Jira Cloud serves multi-thousand-id pages for id-only
  queries, so a census of even hundreds of thousands of issues is a few
  hundred requests.
- Partitioned per project (Jira Cloud rejects unbounded JQL) via a dedicated
  inline parent (`jira_census_projects`) that lists live projects with **no
  incremental gate** — unlike `jira_project_discovery`, which skips idle
  projects and therefore must never drive the census: a deletion does not
  reliably bump the project's `insight.lastIssueUpdateTime` aggregate.
- Sync mode: full refresh, every sync.
- Record: `jira_id` (numeric issue id), `project_key`, identity stamp.
  `unique_key = {tenant}-{source}-{jira_id}` — keyed by **numeric id, never by
  issue key**: moving an issue between projects changes its key but not its
  id; a key-based census would report every moved issue as deleted.

## Classification

`jira__issue_availability_state` recomputes, on every dbt run, one row per
issue ever seen (union of the census and `bronze_jira.jira_issue`):

| # | Observed | Project state (from visibility roster) | `availability` |
|---|----------|----------------------------------------|-----------------|
| 1 | in latest census generation | — | `present` |
| 2 | absent | live, absence below mass threshold | `deleted` |
| 3 | absent | live, absence ≥ mass threshold | `unobserved` |
| 4 | absent | archived | `archived` |
| 5 | absent | in project trash | `trashed` |
| 6 | absent | project gone from the roster entirely | `access_lost` |

- "Latest census generation": `max(_airbyte_extracted_at)` over the census
  table minus a tolerance window (`jira_census_tolerance_hours`, default 12)
  that absorbs the duration of a single sync. An issue whose last observation
  is older than that is *absent*.
- **Mass threshold** (rule 3, `jira_availability_mass_threshold`, default
  0.5): if half or more of a live project's known issues vanish in one
  generation, that is almost never a real mass deletion — it is a partial
  sync, an issue-security change, or a permission edge the roster did not
  catch. Such issues are honestly labelled `unobserved`, not `deleted`.
  They reclassify automatically on a later run: back to `present` if
  re-observed, or to `deleted` once the project's census looks sane again.
- `access_lost` vs `archived`/`trashed`: the roster queries all three
  lifecycle statuses, so a project that is merely archived or trashed is still
  *in* the roster with that status. Only a project absent from all three
  partitions has actually become invisible to the account (permission loss,
  or permanent purge after trash retention).

Why classification lives in dbt and not in the connector: the connector only
*observes* (present-in-response); inference needs the warehouse's memory of
what used to exist, which only Bronze has.

### Known residual ambiguity

- Issue-level security can hide a single issue inside a browsable project
  without any deletion. Indistinguishable from deletion via the REST API
  (Jira intentionally returns identical 404s). Mitigated by the mass
  threshold only when it changes in bulk; a single issue hidden this way will
  be classified `deleted`. Accepted: a misclassification is reversible — the
  raw data stays in Bronze, and the issue reclassifies to `present` as soon
  as it becomes observable again.
- Deletion cannot be distinguished from "hidden" retroactively: permission
  APIs (`mypermissions`, `permissions/check`) cannot be asked about an entity
  that no longer appears. Hence the roster is captured *every* sync — the
  classifier always compares against the visibility surface of the same
  generation, never post-hoc.

## History: deletion is an event

Availability state feeds the same SCD2 machinery as user profiles
(`snapshot()` + `fields_history()` macros):

- `jira__issue_availability_snapshot` — appends a version whenever an issue's
  `availability` flips. Never rewritten, retained forever.
- `jira__issue_availability_history` — one row per transition:
  `field_name='availability', old_value='present', new_value='deleted',
  updated_at=<detection time>`. This is the durable "the entity was deleted"
  event. Transitions back (`unobserved → present`) are recorded the same way.
- `jira__project_visibility_snapshot` / `jira__project_visibility_history` —
  the same pair for project visibility (`is_visible`, `project_status`), so
  "the account lost access to project X on date D" is itself a permanent,
  queryable event.

`updated_at` on these events is the **detection** time (census generation),
not the moment the user clicked delete in Jira — the REST API does not expose
the latter for hard-deleted entities.

### Sub-entity lifecycle events

Comments and worklogs are the two issue sub-entities the changelog does not
cover, so their lifecycle enters the same journal as synthetic events
(`event_kind='lifecycle'`, `field_id='comment'|'worklog'`, `delta_action`
add/set/remove). The event carries the **entity id** (`value_ids[1]`) — the
lookup key into `class_task_comments` / `class_task_worklogs` — not the
payload: the class tables stay the materialized current state (typed columns,
cheap aggregates), the journal holds the history. Sources with a native event
log (e.g. YouTrack activities) can emit these events directly; for Jira they
are derived from the entity snapshots:

- **add** — first observation; dated by the entity's own `updated` timestamp
  (real time).
- **set** — the entity's `updated` (worklogs: also seconds/started) changed
  between syncs; dated by the new `updated`. Multiple edits between two syncs
  collapse into one event — the API exposes no intermediate states.
- **remove** — the deletion signals of this spec flip `is_deleted`; worklog
  removals are dated by the `/worklog/deleted` tombstone (real deletion
  time), comment removals by detection.

## Downstream contract

Availability transitions enter silver as **synthetic field-history events** in
`class_task_field_history` — the same table, shape and machinery as every
other field change. `jira__availability_events` maps the transitions from the
availability SCD2 snapshot into the contract and unions in via
`union_by_tag('silver:class_task_field_history')`:

| field-history column | value for availability events |
|----------------------|--------------------------------|
| `field_id` / `field_name` | `availability` / `Availability` |
| `event_kind` | `availability` (new enum value alongside `changelog`, `synthetic_initial`) |
| `event_id` | `availability:<detection epoch ms>` |
| `event_at` | detection time (census generation) |
| `value_ids` / `value_displays` | the new state — enum from the classification table above |
| `value_id_type` | `string_literal` |

This keeps the contract **source-agnostic**: the census streams and the
classifier are Jira's way of *detecting* absence, but the silver surface only
says "this issue's availability changed at T" — a YouTrack, GitHub or any
other task-tracker connector emits identical events from whatever deletion
signal its API exposes (soft-delete flags, activity logs, webhooks), and every
consumer reads one table for the entire issue lifecycle, deletion included.

Consumption policy (implemented):

- `gold/task_issue_state.sql` — pivots `field_id = 'availability'` alongside
  the other fields and filters out issues whose latest availability is
  `deleted` or `trashed`. Deleted issues therefore leave every task metric on
  the next build (`task_status_spans`, `task_worklog_flow`,
  `task_metric_evidence` all derive from `task_issue_state`, so the filter
  propagates through the whole gold layer). `archived`, `access_lost` and
  `unobserved` issues stay **in**: the entity still exists at the source;
  its data is merely stale — excluding it would silently shrink history.
- `jira__task_comments.sql` — the class contract's `is_deleted` column
  (previously a constant 0) is now real. A comment is deleted when its issue
  was re-fetched but the comment was not re-emitted (deleting a comment bumps
  the issue's `updated`, which re-syncs the issue's full comment list), or
  when its parent issue is `deleted`/`trashed`.
- `class_task_worklogs.is_deleted` — real, from three OR-ed signals in
  `jira__task_worklogs`: an authoritative tombstone from the
  `jira_worklog_deleted` stream (`GET /rest/api/3/worklog/deleted` — the one
  Jira surface where deletions are first-class; the whole bounded tombstone
  list is re-read every sync, census-style), the same re-fetch generation
  diff as comments (editing or deleting a worklog bumps the issue's
  `updated`, re-syncing its full worklog list), and a deleted/trashed parent
  issue. *Updated* worklogs need no extra stream: the per-issue re-fetch
  already re-emits them — the same `updated`-bump assumption the project
  discovery gate has relied on all along.
- `gold/task_worklog_flow.sql` — the in-progress side inherits the filter
  through its `task_issue_state` join; the worklog side filters
  `ifNull(is_deleted, 0) = 0` from the class contract.

Changing the policy (e.g. excluding deleted issues from historical throughput
vs only from open-issue counts) is a one-line change in the gold filter; the
availability data supports either choice.

## Operational constraints

- The census adds one id-only scan of every live project per sync. Requests
  scale with `ceil(issues / 5000)` per project plus one project-roster page
  per ~50 projects per status.
- A failed sync does not corrupt classification: dbt runs only after a
  successful sync in the Argo DAG, and the mass threshold catches
  partially-committed censuses if dbt is run manually against one.
- First census run: issues present in `bronze_jira.jira_issue` but absent
  from the very first census were deleted before the mechanism landed; they
  classify as `deleted` (or `unobserved` where they exceed the mass
  threshold) on that first run — this is the intended backfill of
  historical deletions.
- Webhook-based deletion events (`jira:issue_deleted`, `comment_deleted`)
  would add immediacy but require an inbound endpoint the batch-pull
  architecture does not have; the census is the guaranteed path. If webhooks
  are added later they emit into the same availability model.
- Known limitation — total access loss is not classified: if the account
  loses Browse on *every* project, the roster and the census return zero
  rows, no new generation appears, and every issue stays frozen at its last
  state (`present`). This fails safe — nothing is misclassified as deleted —
  and the same event empties every stream of the connector, so the existing
  `bronze_jira` source-freshness gates (warn 36h / error 72h) raise the alarm
  operationally. A per-generation completion marker could classify this case
  explicitly; deliberately not built here.
- The silver contract changes this mechanism ships (`event_kind` gains
  `availability`, `class_task_worklogs` gains `is_deleted`) are deployed via
  the **major descriptor bump**: per ADR-0015 reconcile dispatches a one-shot
  sync with `dbt --full-refresh` on the connector's selector, which rebuilds
  the affected staging/silver tables from bronze. No ALTER migrations are
  shipped for staging/silver.
