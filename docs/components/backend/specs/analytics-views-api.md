---
status: proposed
date: 2026-04-09
authors: ["insight-front team"]
---

# Analytics API — Frontend Data Requirements

<!-- toc -->

- [1. Scope](#1-scope)
- [2. Architecture Alignment with DESIGN.md](#2-architecture-alignment-with-designmd)
  - [Remaining discrepancies with upstream DESIGN.md](#remaining-discrepancies-with-upstream-designmd)
- [3. Data Sources Inventory](#3-data-sources-inventory)
- [4. Required Metric Definitions](#4-required-metric-definitions)
  - [4.1 Executive Summary Metric](#41-executive-summary-metric)
  - [4.2 Team Member Metric](#42-team-member-metric)
  - [4.3 IC (Individual Contributor) Metric](#43-ic-individual-contributor-metric)
  - [4.4 Org Aggregate (FE-Computed)](#44-org-aggregate-fe-computed)
  - [4.5 IC Chart Trend Metrics](#45-ic-chart-trend-metrics)
  - [4.6 Person Profile (Identity Service)](#46-person-profile-identity-service)
  - [4.7 Time Off Notice Metric](#47-time-off-notice-metric)
  - [4.8 Drill Metrics](#48-drill-metrics)
- [5. Screen → Metric Mapping](#5-screen--metric-mapping)
- [6. Frontend-Computed Fields and Backend Threshold Evaluation](#6-frontend-computed-fields-and-backend-threshold-evaluation)
- [7. Open Questions](#7-open-questions)
- [8. Data Availability](#8-data-availability)
- [9. CI Connector Roadmap](#9-ci-connector-roadmap)
- [10. Implementation Checklist](#10-implementation-checklist)

<!-- /toc -->

---

## 1. Scope

This document defines what data the `insight-front` React frontend needs from the Analytics API,
expressed in terms of the `Metric` entity defined in
`docs/components/backend/specs/DESIGN.md`.

It does **not** re-propose a new API shape. It adopts DESIGN.md as the architecture baseline
and specifies:

- Which `Metric` entries the frontend requires (seeded in MariaDB via the metrics catalog)
- Which columns each metric's `query_ref` must expose
- Which fields the frontend will compute locally (removing backend burden)
- Open questions where the generic query model needs clarification to cover screen needs

---

## 2. Architecture Alignment with DESIGN.md

The following decisions from DESIGN.md are accepted in full:

| Topic | Accepted approach |
|---|---|
| Query endpoint | `POST /api/analytics/v1/metrics/{id}/query` — metric UUID in URL path, OData query over seeded metric definitions |
| Query paradigm | OData `$filter`, `$orderby`, `$select`, `$top` (default 25, max 200); cursor via `$skip` — per DNA REST conventions |
| Date params | Date filtering via OData `$filter` (e.g. `metric_date ge '...' and metric_date lt '...'`); FE maps period labels to ranges |
| Auth | `Authorization: Bearer <token>` only; `insight_tenant_id` extracted from JWT |
| Team scoping | `org_unit_id` in OData `$filter`; **backend validates access** via AuthZEN PolicyEnforcer (see security note below) |
| Person filter | Frontend sends Insight `person_ids` (UUIDs); backend resolves to source aliases |
| Response shape | `{ items: Row[], page_info: { has_next, cursor } }` |
| Errors | RFC 9457 Problem Details |
| Metric management | CRUD via `POST/PUT/DELETE /api/analytics/v1/metrics`; thresholds at `/api/analytics/v1/metrics/{id}/thresholds` — Tenant Admin only |
| Threshold evaluation | Backend evaluates each row against metric's `thresholds` table and attaches `_thresholds: { field: level }` to every response row |

**Security note — IDOR on `org_unit_id`:**
Accepting `org_unit_id` from the request body requires a backend authorization check:
the query engine must verify that the authenticated user's AccessScope includes the
requested org unit before executing the query. Trusting the client-supplied UUID alone
while only validating `insight_tenant_id` from JWT would allow any user within a tenant
to query any team's data by guessing or enumerating UUIDs.

**Multi-period queries for delta computation:**
The IC Dashboard requires current and previous period values to display deltas (e.g.
"PR cycle time: 18h, ↓ 3h vs last month"). OData does not support multi-period queries
natively, so the frontend issues **two parallel requests** to the same metric with
different date ranges in `$filter`:

```
Request 1: $filter=person_id eq 'uuid-1' and metric_date ge '2026-03-01' and metric_date lt '2026-04-01'
Request 2: $filter=person_id eq 'uuid-1' and metric_date ge '2026-02-01' and metric_date lt '2026-03-01'
```

This is acceptable for ClickHouse given the lightweight aggregated query shapes
(one row per person per period). The frontend computes deltas client-side from the
two responses.

### Remaining discrepancies with upstream DESIGN.md

All discrepancies from the original draft are resolved per cyberantonz comments (2026-04-14):

| Topic | Resolution |
|---|---|
| Metric `query_ref` semantics | **Confirmed**: `query_ref` holds raw ClickHouse SQL. Query engine wraps it as subquery, appends security filters + OData filters as parameterized WHERE clauses, executes against ClickHouse. |
| Metric UUID in request | **Confirmed**: `POST /api/analytics/v1/metrics/{id}/query` — UUID in path. OData params in body: `$filter`, `$orderby`, `$top`, `$select`, `$skip` (cursor). No `metric_id` in `$filter`. |
| Stats / percentiles | **Resolved**: No separate stats endpoint. Seed a dedicated stats `Metric` per KPI with `quantile(0.05)(...)`, `quantile(0.50)(...)`, `quantile(0.95)(...)` in `query_ref`. Same query path as all other metrics. |
| Threshold config | **Resolved**: Thresholds are nested under metrics. CRUD: `GET/POST /api/analytics/v1/metrics/{id}/thresholds`, `PUT/DELETE /api/analytics/v1/metrics/{id}/thresholds/{tid}`. Each threshold: `field_name`, `operator`, `value`, `level` (good/warning/critical). Evaluation is server-side — each query response row includes `_thresholds: { field_name: level }`. |
| Data availability | **Resolved**: Analytics API does NOT include `data_availability` in query responses. Frontend calls Connector Manager directly: `GET /api/connectors/v1/connections/{id}/status`. See §8. |
| Person profile | **Resolved**: Use Identity Service directly (`GET /api/identity-resolution/v1/persons/{id}`). No analytics metric needed for person header. |

---

**Previously proposed but dropped:**

- ~~`X-Tenant-ID` header~~ — tenant from JWT only (DESIGN.md §4.1)
- ~~`period=week|month|quarter|year` enum~~ — replaced by date ranges in OData `$filter`
- ~~Per-screen hardcoded endpoints~~ (`/views/executive`, `/views/team`) — replaced by metric UUIDs
- ~~Custom `View` entity~~ — mapped to `Metric` entries from DESIGN.md §3.1
- ~~JSON body `{ filters: {...}, order_by, limit }`~~ — replaced by OData `$filter` / `$orderby` / `$top`
- ~~`periods` array for multi-period delta~~ — replaced by two parallel OData requests
- ~~`status: good|warn|bad` inlined in every field~~ — backend evaluates thresholds server-side and attaches `_thresholds: { field_name: 'good'|'warning'|'critical' }` per row (DESIGN.md §3.2)
- ~~`trend_label` string from backend~~ — frontend computes from multi-period delta (see §6)

---

## 3. Data Sources Inventory

Source table annotations appear as `[source]` comments in metric definitions (§4).

| Source | Silver Table | Status | Connector PR |
|---|---|---|---|
| GitHub | `class_git_commits`, `class_git_pull_requests`, `class_git_pull_requests_reviewers` | ✓ available | #57 merged |
| Bitbucket Cloud | same `class_git_*` tables (`data_source` discriminator) | ⚠ in progress | #58 draft |
| Bitbucket Server | same `class_git_*` tables | ⚠ configured | — |
| Claude Team / Code | `class_ai_dev_usage` | ✓ available | #50 merged |
| Cursor | `class_ai_dev_usage` | ✓ available | merged |
| BambooHR | `class_people` | ✓ available | #47 merged |
| Zoom | `class_comms_events` | ✓ available | #61 merged |
| M365 | `class_comms_events` | ✓ available | merged |
| Slack | `class_comms_events` | ✓ available | #48 merged |
| Jira | `class_tasks` (TBD schema) | ⚠ pending | #62 open |
| GitHub Actions (CI) | `class_ci_runs` (proposed) | ❌ no connector | see §9 |

---

## 4. Required Metric Definitions

These are the `Metric` entries that need to be seeded into the Analytics API MariaDB catalog.
Each metric has a stable UUID assigned at seed time. The frontend references metrics by UUID
via `POST /api/analytics/v1/metrics/{id}/query`.

Each definition below specifies:
- **`query_ref`** — conceptual ClickHouse SQL that the query engine executes
- **Row shape** — TypeScript type of each returned `items[]` element
- **OData filters** — example `$filter` / `$orderby` expressions the frontend will send

### 4.1 Executive Summary Metric

**Purpose:** Per-team aggregate row for the Executive screen team table and KPI cards.

**`query_ref`** (conceptual — actual ClickHouse SQL defined by backend):

```sql
SELECT
  org_unit_id,                                              -- [identity] org_units.id
  org_unit_name,                                            -- [identity] org_units.name
  COUNT(DISTINCT person_id) AS headcount,                   -- [hr] class_people WHERE active
  COUNT(task_id) FILTER (WHERE done AND type != 'Bug')
    AS tasks_closed,                                        -- [tasks] class_tasks — null until PR #62
  COUNT(task_id) FILTER (WHERE done AND type  = 'Bug')
    AS bugs_fixed,                                          -- [tasks] class_tasks — null until PR #62
  AVG(CASE WHEN status = 'success' THEN 100 ELSE 0 END)
    AS build_success_pct,                                   -- [ci]    class_ci_runs — null until §9
  AVG(focus_time_pct)       AS focus_time_pct,              -- [comms] class_comms_events
  COUNT(DISTINCT person_id) FILTER (WHERE ai_active)
    * 100.0 / COUNT(DISTINCT person_id) AS ai_adoption_pct, -- [ai-code] class_ai_dev_usage
  SUM(ai_lines_added) * 100.0 / NULLIF(SUM(total_lines),0)
    AS ai_loc_share_pct,                                    -- [ai-code] class_ai_dev_usage
  AVG(pr_cycle_time_h) AS pr_cycle_time_h                   -- [git]   class_git_pull_requests
FROM ...
WHERE org_unit_id = ?   -- required filter: team scoping
  AND date >= ?         -- required filter: date_from
  AND date <  ?         -- required filter: date_to
GROUP BY org_unit_id, org_unit_name
```

**Row shape** (one row per team):

```ts
type ExecSummaryRow = {
  org_unit_id:        string;
  org_unit_name:      string;
  headcount:          number;
  tasks_closed:       number | null;   // null until PR #62
  bugs_fixed:         number | null;   // null until PR #62
  build_success_pct:  number | null;   // null until CI connector
  focus_time_pct:     number;
  ai_adoption_pct:    number;
  ai_loc_share_pct:   number;
  pr_cycle_time_h:    number;
};
```

**OData query from frontend:**

```
POST /api/analytics/v1/metrics/{uuid}/query

$filter=metric_date ge '2026-03-01' and metric_date lt '2026-04-01'
$orderby=org_unit_name asc
$top=100
```

> **Note on team scoping (reviewer comment):** The frontend adds
> `org_unit_id eq 'uuid'` to `$filter` when showing a single team, or omits it when
> the user has org-wide visibility (Executive screen). The AuthZEN PolicyEnforcer
> adds `org_unit_id IN (...)` constraints from the user's AccessScope automatically.

---

### 4.2 Team Member Metric

**Purpose:** Per-person rows for the Team screen member table and bullet benchmark sections.
The frontend also derives three team KPI cards (`at_risk_count`, `focus_gte_60`,
`not_using_ai`) from these rows — no separate aggregate metric needed.

**`query_ref`** (conceptual):

```sql
SELECT
  person_id,                                                -- [identity]
  display_name,                                             -- [hr]   class_people
  -- seniority excluded: see §7, open question #4
  dev_time_h,                                               -- [comms] work_h - meeting_h
  prs_merged,                                               -- [git]   COUNT(class_git_pull_requests WHERE merged)
  build_success_pct,                                        -- [ci]    null until §9
  focus_time_pct,                                           -- [comms]
  ai_tools,                                                 -- [ai-code] ARRAY_AGG(DISTINCT source)
  ai_loc_share_pct,                                         -- [ai-code]
  tasks_closed,                                             -- [tasks] 0 until PR #62
  bugs_fixed                                                -- [tasks] 0 until PR #62
FROM ...
WHERE org_unit_id = ?   -- required: team scoping (passed by FE — not inferred from JWT)
  AND date >= ?
  AND date <  ?
```

**Row shape:**

```ts
type TeamMemberRow = {
  person_id:         string;
  display_name:      string;
  dev_time_h:        number;
  prs_merged:        number;
  build_success_pct: number | null;
  focus_time_pct:    number;
  ai_tools:          string[];
  ai_loc_share_pct:  number;
  tasks_closed:      number;
  bugs_fixed:        number;
};
```

**Frontend-derived team KPI strip** (computed from member rows, no extra query):

| KPI | Computation |
|---|---|
| `at_risk_count` | `COUNT` where any alert threshold triggered |
| `focus_gte_60` | `COUNT` where `focus_time_pct >= 60` |
| `not_using_ai` | `COUNT` where `ai_tools.length === 0` |

**OData query from frontend:**

```
POST /api/analytics/v1/metrics/{uuid}/query

$filter=metric_date ge '2026-03-01' and metric_date lt '2026-04-01' and org_unit_id eq 'uuid-of-team'
$orderby=display_name
$top=200
```

---

### 4.3 IC (Individual Contributor) Metric

**Purpose:** Aggregated KPI values for the IC Dashboard screen for a single person.

**`query_ref`** (conceptual):

```sql
SELECT
  person_id,
  SUM(clean_loc)          AS loc,                -- [git]     class_git_commits
  SUM(ai_lines_added) * 100.0 / NULLIF(SUM(clean_loc + ai_lines_added), 0)
                          AS ai_loc_share_pct,   -- [ai-code]
  COUNT(pr_id)            AS prs_merged,         -- [git]
  AVG(pr_cycle_time_h)    AS pr_cycle_time_h,    -- [git]
  AVG(focus_time_pct)     AS focus_time_pct,     -- [comms]
  COUNT(task_id) FILTER (WHERE done AND type != 'Bug') AS tasks_closed, -- [tasks]
  COUNT(task_id) FILTER (WHERE done AND type  = 'Bug') AS bugs_fixed,   -- [tasks]
  AVG(CASE WHEN status = 'success' THEN 100 ELSE 0 END)
                          AS build_success_pct,  -- [ci]
  SUM(session_count)      AS ai_sessions         -- [ai-code] class_ai_dev_usage
FROM ...
WHERE person_id = ?   -- required: person scoping
  AND date >= ?
  AND date <  ?
```

**Row shape** (one row per person per period):

```ts
type IcSummaryRow = {
  person_id:         string;
  loc:               number;
  ai_loc_share_pct:  number;
  prs_merged:        number;
  pr_cycle_time_h:   number;
  focus_time_pct:    number;
  tasks_closed:      number;
  bugs_fixed:        number;
  build_success_pct: number | null;
  ai_sessions:       number;
};
```

For **chart trend data** see §4.5 — separate time-series metrics are required.

For **drill tables**, see §4.8.

> **Note on `date_from`/`date_to` and drill (CodeRabbit comment, line 275):**
> All drill queries include `date_from`/`date_to` as required filters.
> Previously the IC drill was missing this — now explicit.

---

### 4.4 Org Aggregate (FE-Computed)

**Purpose:** Org-wide KPI cards on the Executive screen (`avgBuildSuccess`, `avgAiAdoption`,
`avgFocus`, `bugResolutionScore`, `prCycleScore`).

These are aggregates of team aggregates and can be computed on the frontend from the
rows returned by §4.1 (Executive Summary Metric queried without an `org_unit_id` filter).
No separate backend metric is needed.

**Frontend computation from `ExecSummaryRow[]`:**

| KPI | Computation |
|---|---|
| `avgBuildSuccess` | `AVG(build_success_pct)` — `null` if all rows have `null` |
| `avgAiAdoption` | `AVG(ai_adoption_pct)` |
| `avgFocus` | `AVG(focus_time_pct)` |
| `bugResolutionScore` | `null` until Jira connector ships (tasks unavailable) |
| `prCycleScore` | `AVG(pr_cycle_time_h)` displayed directly — score formula TBD |

---

### 4.5 IC Chart Trend Metrics

**Purpose:** Time-series chart data for the IC Dashboard screen.
The IC screen shows two charts: LOC trend and Delivery trend.
These require rows grouped by date bucket (week or month), **not** a single aggregate —
so they need separate seeded metrics with a `GROUP BY date_bucket` in `query_ref`.

#### LOC Trend Metric (per person, per time bucket)

```sql
SELECT
  toStartOfWeek(commit_date) AS date_bucket,         -- granularity per UI period
  SUM(ai_lines_added)        AS ai_loc,              -- [ai-code] class_ai_dev_usage
  SUM(clean_lines)           AS code_loc,            -- [git]     class_git_commits
  SUM(spec_lines)            AS spec_lines            -- [git]     class_git_commits (if tracked)
FROM ...
WHERE person_id = ?
  AND commit_date >= ?
  AND commit_date <  ?
GROUP BY date_bucket
ORDER BY date_bucket
```

**Row shape:**

```ts
type LocTrendRow = {
  date_bucket: string;   // ISO date — FE formats as chart label
  ai_loc:      number;
  code_loc:    number;
  spec_lines:  number;
};
```

#### Delivery Trend Metric (per person, per time bucket)

```sql
SELECT
  toStartOfWeek(activity_date) AS date_bucket,
  COUNT(DISTINCT commit_sha)   AS commits,     -- [git]   class_git_commits
  COUNT(DISTINCT pr_id)        AS prs_merged,  -- [git]   class_git_pull_requests WHERE merged
  COUNT(DISTINCT task_id)      AS tasks_done   -- [tasks] class_tasks WHERE done
FROM ...
WHERE person_id = ?
  AND activity_date >= ?
  AND activity_date <  ?
GROUP BY date_bucket
ORDER BY date_bucket
```

**Row shape:**

```ts
type DeliveryTrendRow = {
  date_bucket: string;
  commits:     number;
  prs_merged:  number;
  tasks_done:  number;
};
```

> **Note on SQL dialect:** The `query_ref` in Metric definitions is **ClickHouse SQL**,
> not MariaDB SQL. MariaDB stores only the metric metadata (name, description, UUID,
> `query_ref` as text). The query engine executes the stored SQL against ClickHouse.
> Functions like `toStartOfWeek`, `quantile`, `ARRAY_AGG` are ClickHouse-specific and
> must not be validated or parsed by MariaDB. Metric authors must write ClickHouse-compatible
> SQL in the `query_ref` field.

> **Note on granularity:** The FE selects date ranges via OData `$filter` based on the UI
> period (week → last 7 days, month → last 4 weeks, quarter → last 13 weeks, year → last 12
> months). The metric's `toStartOfWeek` / `toStartOfMonth` grouping in `query_ref`
> determines point density. If the query engine supports a configurable `GROUP BY`
> expression per metric, the same metric UUID can serve different granularities.
> Otherwise two metric variants (weekly/monthly) may be needed.

---

### 4.6 Person Profile (Identity Service)

**Purpose:** Person header on the IC Dashboard (name, role, seniority).

> **Preferred source:** The Identity Service exposes
> `GET /api/v1/identity/persons/{id}` (DESIGN.md §3.3) which serves this data directly.
> The frontend should call Identity Service for person profile — no seeded analytics
> metric needed. The `query_ref` below is kept only as a fallback reference for the
> data shape.

```sql
SELECT
  person_id,
  display_name,           -- [hr] class_people.display_name
  role,                   -- [hr] class_people.job_title or role mapping
  seniority               -- [hr] class_people.custom_str_attrs['seniority'] — see §7 Q4
FROM class_people
WHERE person_id = ?
LIMIT 1
```

**Row shape:**

```ts
type PersonProfileRow = {
  person_id:  string;
  display_name: string;
  role:       string;
  seniority:  string | null;   // null if not configured — FE renders nothing
};
```

---

### 4.7 Time Off Notice Metric

**Purpose:** Upcoming time-off banner on the IC Dashboard.
Sourced from BambooHR leave records in `class_people`.

```sql
SELECT
  person_id,
  leave_start_date,
  leave_end_date,
  leave_days,
  bamboohr_leave_url
FROM class_people_leave   -- or equivalent leave table
WHERE person_id = ?
  AND leave_start_date >= {date_from:Date}  -- passed by FE as today's UTC date
ORDER BY leave_start_date
LIMIT 1
```

> **Note:** Do not use ClickHouse `today()` — it returns the server's local date which
> may differ from the client's UTC date. The frontend passes `date_from` as today's
> UTC date (`new Date().toISOString().slice(0, 10)`).

**Row shape** (`null` response means no upcoming leave):

```ts
type TimeOffRow = {
  person_id:          string;
  leave_start_date:   string;
  leave_end_date:     string;
  leave_days:         number;
  bamboohr_leave_url: string;
} | null;
```

> **Note:** If `class_people` does not carry leave records in V1, this metric can return
> empty results and the IC screen will simply not render the notice banner.

---

### 4.8 Drill Metrics

A drill is a paginated row-level breakdown of a single KPI for a specific person or team.
The frontend triggers a drill when a user clicks a metric value. Each drill maps to a
pre-defined `Metric` entry in the catalog.

Fields per drill are fixed and determined by the Silver table being queried.
The frontend renders them generically (column list + row array — no field-specific code).

**Response shape** (same envelope as all metric queries):

```json
{
  "items": [
    { "pr_title": "Fix auth middleware", "merged_at": "2026-03-14T11:00:00Z", "cycle_time_h": 18.5 }
  ],
  "page_info": { "has_next": true, "cursor": "eyJ..." }
}
```

All drill queries require `metric_date` range in OData `$filter`.

---

#### IC Drills (scoped to `person_id`)

| Drill ID | Title | Source | Silver Table | Columns |
|---|---|---|---|---|
| `commits` | Commits | GitHub / Bitbucket | `class_git_commits` | Commit SHA, Repository, +LOC, -LOC, Date |
| `pull-requests` | Pull Requests | GitHub / Bitbucket | `class_git_pull_requests` | PR title, Repository, Status, Cycle Time |
| `reviews` | Code Reviews Given | GitHub / Bitbucket | `class_git_pull_requests_reviewers` | PR title, Author, Outcome, Time to Review |
| `builds` | Build Results | CI | `class_ci_runs` *(pending §9)* | Build ID, Branch, Status, Duration |
| `tasks-completed` | Tasks Completed | Jira | `class_tasks` *(pending PR #62)* | Task key, Story Points, Dev Time, Closed Date |
| `cycle-time` | Task Development Time | Jira | `class_tasks` | Task key, Story Points, Dev Time, Status |
| `task-reopen` | Reopened Tasks | Jira | `class_tasks` | Task key, Reopen Reason, Reopened At, Resolved At |
| `bugs-fixed` | Bugs Fixed | Jira | `class_tasks` | Bug key, Priority, Fix Time, Closed Date |

---

#### Team Drills (scoped to `org_unit_id`)

| Drill ID | Title | Source | Silver Table | Columns |
|---|---|---|---|---|
| `team-members` | Team Members Overview | Jira + GitHub/Bitbucket | `class_git_*`, `class_tasks`, `class_people` | Name, Tasks, Dev Time, PRs, Build %, Focus %, AI LOC % |
| `team-tasks` | Tasks Closed per Developer | Jira | `class_tasks` | Name, Tasks Closed, Story Points, Avg SP/Task, vs Team |
| `team-dev-time` | Task Development Time per Member | Jira | `class_tasks` | Name, Dev Time, vs Team avg, vs Org avg, Tasks Sampled |
| `team-prs` | Pull Requests Merged per Developer | GitHub / Bitbucket | `class_git_pull_requests` | Name, PRs Merged, Avg Cycle Time, Avg Size (LOC), Reviews Given |
| `team-pr-cycle` | Pull Request Cycle Time per Member | GitHub / Bitbucket | `class_git_pull_requests` | Name, Cycle Time, Pickup Time, Review Time, vs Org avg |
| `team-bugs` | Bugs Fixed per Member | Jira | `class_tasks` | Name, Bugs Fixed, Avg Fix Time, Reopened, Reopen Rate |
| `team-build` | Build Success Rate per Member | CI | `class_ci_runs` *(pending §9)* | Name, Build %, Passed, Failed |
| `team-focus` | Focus Time per Member | Calendar / M365 | `class_comms_events` | Name, Focus %, Focus Hours, Meeting Hours |
| `team-ai-active` | AI Tool Adoption by Member | Cursor + Claude Code | `class_ai_dev_usage` | Name, Tools Active, AI LOC Share, Sessions per tool |
| `team-ai-loc` | AI LOC Share per Member | Cursor + Claude Code | `class_ai_dev_usage` | Name, AI LOC %, AI Lines, Clean LOC, Lines per tool |
| `team-reopen` | Task Reopen Rate per Member | Jira | `class_tasks` | Name, Reopen Rate, Reopened, Total Closed, Top Reason |

---

> **Note on drill discovery:** The frontend knows which drill IDs exist per KPI via the
> `drill_id` field returned in bullet metric objects. An empty `drill_id` means no drill
> is available for that KPI. The backend does not need to expose a drill catalog endpoint —
> the mapping is part of the metric seed data.

> **Note on Team Drill JOIN complexity:** Team drills like `team-prs` and `team-tasks`
> require filtering pull requests or tasks by org unit membership. A runtime JOIN between
> `class_git_pull_requests` and `class_people` in ClickHouse is expensive at scale.
> The Silver dbt models for `class_git_*` and `class_tasks` **must pre-populate
> `org_unit_id`** from `class_people` at transformation time, so drill queries become
> simple `WHERE org_unit_id = ?` without any JOIN. Confirm this is the case before
> implementing Team Drill metric definitions.

> **Note on cursor pagination with aggregated sort:** Drill metrics support sorting by
> aggregated fields (e.g. sort `team-prs` by `PRs Merged` via `$orderby`). Cursor-based
> pagination must be stable under this sort — confirm the query engine supports keyset
> pagination on computed/aggregated columns, not just on raw row keys.

---

## 5. Screen → Metric Mapping

The frontend will maintain a mapping of screen name → metric UUID:

| Screen | Metrics / services used | Query count |
|---|---|---|
| Executive | §4.1 Exec Summary Metric (all teams) | 1 |
| Executive KPI cards | Derived from §4.1 rows on FE (§4.4) | 0 extra |
| Team members table | §4.2 Team Member Metric | 1 |
| Team KPI strip | Derived from §4.2 rows on FE | 0 extra |
| IC Dashboard KPIs | §4.3 IC Summary Metric × 2 periods (current + previous) | 2 |
| IC LOC chart | §4.5 LOC Trend Metric | 1 |
| IC Delivery chart | §4.5 Delivery Trend Metric | 1 |
| IC person header | Identity Service: `GET /api/v1/identity/persons/{id}` | 1 |
| IC time-off notice | §4.7 Time Off Notice Metric | 1 |
| Any drill-down | §4.8 Drill Metric (paginated) | 1 per open drill |

**Date range mapping on the frontend** (OData `$filter` date boundaries):

| UI label | `$filter` expression |
|---|---|
| Week | `metric_date ge '2026-04-07' and metric_date lt '2026-04-14'` |
| Month | `metric_date ge '2026-04-01' and metric_date lt '2026-05-01'` |
| Quarter | `metric_date ge '2026-01-01' and metric_date lt '2026-04-01'` |
| Year | `metric_date ge '2026-01-01' and metric_date lt '2027-01-01'` |

**Timezone contract:** All date values in `$filter` are **UTC dates** in `YYYY-MM-DD`
format. The backend treats `2026-04-07` as `2026-04-07T00:00:00Z` — `2026-04-07T23:59:59Z`.
The frontend computes date boundaries in UTC regardless of the user's local timezone.
This prevents data loss at day boundaries when team members are in different timezones
from the server.

---

## 6. Frontend-Computed Fields and Backend Threshold Evaluation

### Backend — Threshold evaluation (server-side)

Per DESIGN.md §3.2 and cyberantonz confirmation, the Query Engine loads the `thresholds`
table for each metric and evaluates every result row server-side. Each row in `items[]`
includes a `_thresholds` field with the highest matched threshold level per field:

```json
{
  "focus_time_pct": 0.58,
  "pr_cycle_time_h": 26.4,
  "_thresholds": {
    "focus_time_pct": "warning",
    "pr_cycle_time_h": "critical"
  }
}
```

The frontend reads `row._thresholds[fieldName]` to apply cell coloring.
It does NOT compute status itself — this is fully managed by the backend.

Threshold CRUD for Tenant Admin: `GET/POST /api/analytics/v1/metrics/{id}/thresholds`,
`PUT/DELETE /api/analytics/v1/metrics/{id}/thresholds/{tid}`.

### Frontend — fields still computed client-side

| Field | Frontend computation |
|---|---|
| `trend_label` | FE computes delta between two periods (current vs previous query) |
| `delta` / `delta_type` on `IcKpi` | FE queries two date ranges in parallel and computes diff |
| `at_risk_count` on Team KPI strip | `COUNT` of members where `_thresholds` has any `critical` level |
| `focus_gte_60` on Team KPI strip | `COUNT` where `focus_time_pct >= 60` |
| `not_using_ai` on Team KPI strip | `COUNT` where `ai_tools.length === 0` |
| Bullet chart `bar_left_pct`, `median_left_pct`, `bar_width_pct` | Computed from stats metric percentiles (§7 Q1 — resolved) |
| `prCycleScore` in `OrgKpis` | FE displays raw `avg_pr_cycle_time_h`; score formula TBD |

---

## 7. Open Questions

### ~~Open Question 1~~ — Bullet chart distribution (P5/P50/P95) — RESOLVED

**Resolution (cyberantonz, 2026-04-14):** No separate stats endpoint needed.
Seed a dedicated stats `Metric` for each KPI distribution using ClickHouse `quantile()` functions in `query_ref`:

```sql
SELECT
  quantile(0.05)(focus_time_pct) AS p5,
  quantile(0.50)(focus_time_pct) AS p50,
  quantile(0.95)(focus_time_pct) AS p95
FROM ...
WHERE org_unit_id = ?
  AND date >= ?
  AND date < ?
```

The frontend queries this stats metric once per screen load (same endpoint, same OData path),
and uses the returned P5/P50/P95 values to position bullet chart markers.

**Seeded stats metrics needed:** one per KPI column used in bullet charts
(e.g. `focus_time_pct`, `pr_cycle_time_h`, `ai_loc_share_pct` for Team screen).

---

### ~~Open Question 2~~ — Threshold and "Attention Needed" config — RESOLVED

**Resolution (cyberantonz, 2026-04-14):** Thresholds are nested under each metric.
CRUD endpoints:

```
GET    /api/analytics/v1/metrics/{id}/thresholds
POST   /api/analytics/v1/metrics/{id}/thresholds
PUT    /api/analytics/v1/metrics/{id}/thresholds/{tid}
DELETE /api/analytics/v1/metrics/{id}/thresholds/{tid}
```

Each threshold: `field_name`, `operator` (gt/ge/lt/le/eq), `value` (decimal), `level` (good/warning/critical).

Evaluation is **server-side** — the Query Engine attaches `_thresholds` to every result row (see §6).
Frontend reads `_thresholds` for cell coloring and for `at_risk_count` ("Attention Needed" block).

Both column thresholds (cell coloring) and alert thresholds are the same entity differentiated by
`level`. The frontend uses `critical` level for "Attention Needed" block membership.

---

### Open Question 4 — `seniority` field

**Decision: use `job_title` as-is in V1.** Do not attempt to map or normalize seniority
levels in the backend. Every company has different grade nomenclature (L1/L2/Senior/G3)
and building a normalization layer is a separate data transformation task out of scope for
V1. The `PersonProfileRow.seniority` field (§4.6) should be populated directly from
`class_people.job_title` with no transformation. The frontend renders it as a plain label
and handles a null value gracefully (renders nothing).

---

## 8. Data Availability

**Resolution:** The Analytics API does NOT include `data_availability` in query responses.
This is the Connector Manager's responsibility. The frontend calls Connector Manager directly:

```
GET /api/connectors/v1/connections/{id}/status
```

The frontend fetches connector status in parallel with analytics queries and merges
the result client-side to show "Not configured" states instead of misleading zeros.

**Desired shape from Connector Manager** (one call per connector type needed on screen):

```json
{
  "status": "available",
  "last_synced_at": "2026-04-09T06:00:00Z"
}
```

**Status values:**

| Value | Meaning | Effect on UI |
|---|---|---|
| `available` | Connector configured and synced | Fields show real values |
| `partial` | Connector configured, not all sources synced | Fields show real values with coverage warning |
| `no-connector` | Connector not configured | Affected fields show "—" or "Not configured" |
| `syncing` | Initial full sync in progress | Affected fields show "—" |

`partial` prevents misleading `available` when, for example, one GitHub repository
is still onboarding. The frontend surfaces a banner: "Data may be incomplete —
not all sources have synced."

**Affected fields per connector:**

- `tasks` (Jira): `tasks_closed`, `bugs_fixed` → show "—" when `no-connector` or `syncing`
- `ci` (GitHub Actions / Bitbucket Pipelines): `build_success_pct` → show "—" when `no-connector` or `syncing`

**Scope: per-org-unit, not per-tenant.**
If Jira is configured for only 2 of 5 teams, connector status must be fetched with org unit context.
An org unit with no Jira project linked returns `no-connector`; one with Jira configured returns `available`.

---

## 9. CI Connector Roadmap

`build_success_pct` returns `null` in V1 — no CI connector exists yet.
This section proposes the implementation path for a future release.

### Proposed `class_ci_runs` Silver table

```sql
class_ci_runs (
  tenant_id        UUID,
  source_id        String,        -- 'insight_github', 'insight_bitbucket_cloud'
  unique_key       String,
  run_id           String,
  pipeline_name    String,
  branch           String,
  commit_sha       String,        -- joins class_git_commits
  repo_name        String,
  triggered_by     String,        -- person email or 'scheduler'
  status           LowCardinality(String),  -- 'success' | 'failure' | 'cancelled'
  started_at       DateTime,
  finished_at      DateTime,
  duration_seconds UInt32,
  person_id        UUID NULL      -- resolved via Identity Resolution
)
ENGINE = ReplacingMergeTree
PARTITION BY toYYYYMM(started_at)
ORDER BY (tenant_id, source_id, run_id);
```

### Step 1 — GitHub Actions stream

Add `workflow_runs` stream to the GitHub connector (follow-up to PR #57):

- Endpoint: `GET /repos/{owner}/{repo}/actions/runs`
- Incremental by `updated_at`
- Bronze table: `bronze_github.workflow_runs`
- Fields: `id`, `name`, `head_branch`, `head_sha`, `status`, `conclusion`,
  `created_at`, `run_started_at`, `triggering_actor.login`

### Step 2 — Bitbucket Pipelines stream

Add `pipelines` stream to the Bitbucket Cloud connector (before PR #58 merge):

- Endpoint: `GET /repositories/{workspace}/{repo}/pipelines/`
- Incremental by `created_on`
- Bronze table: `bronze_bitbucket_cloud.pipelines`
- Fields: `uuid`, `state.name`, `state.result.name`, `target.ref_name`,
  `target.commit.hash`, `trigger.name`, `created_on`, `completed_on`, `duration_in_seconds`

### Step 3 — dbt Silver model

Create `class_ci_runs` unifying both sources via `union_by_tag`:

- Map `conclusion='success'` / `state.result.name='SUCCESSFUL'` → `status='success'`
- Map `head_sha` / `target.commit.hash` → `commit_sha`

### Step 4 — Identity resolution

Register `triggering_actor.login` (GitHub) and `trigger.name` (Bitbucket)
as `value_type = 'username'` in `identity_inputs`.

### Impact

Once `class_ci_runs` is available:

- `build_success_pct` changes from `null` to real values in all metrics
- `data_availability.ci` changes from `no-connector` to `available`
- No frontend type changes needed — `number | null` already handles both states

---

## 10. Implementation Checklist

### Resolved — backend confirmations (cyberantonz, 2026-04-14)

- [x] `Metric.query_ref` holds raw ClickHouse SQL executed by the query engine
- [x] Metric UUID in URL path: `POST /api/analytics/v1/metrics/{id}/query`
- [x] OData params in body: `$filter`, `$orderby`, `$top`, `$select`, `$skip` (cursor)
- [x] `org_unit_id` in `$filter` validated against AccessScope (IDOR prevention)
- [x] Threshold CRUD at `GET/POST /api/analytics/v1/metrics/{id}/thresholds`
- [x] Threshold evaluation is server-side — `_thresholds` field in every response row
- [x] P5/P50/P95 via regular seeded stats metrics with `quantile()` in `query_ref`
- [x] `data_availability` from Connector Manager, not Analytics API
- [x] Person profile from Identity Service: `GET /api/identity-resolution/v1/persons/{id}`

### Backend — Metric Catalog Seeding

- [ ] `POST /api/analytics/v1/metrics` — seed all metric definitions listed below
- [ ] `POST /api/analytics/v1/metrics/{id}/query` — execute with OData `$filter` / `$orderby` / `$top`

**Metrics to seed:**

| Metric | Section | Notes |
|---|---|---|
| Exec Summary | §4.1 | Group by org_unit; all teams when no org_unit_id filter |
| Team Member | §4.2 | Per-person aggregate |
| IC Summary | §4.3 | Single-person aggregate; queried twice for delta |
| IC LOC Trend | §4.5 | Time-series, GROUP BY date bucket |
| IC Delivery Trend | §4.5 | Time-series, GROUP BY date bucket |
| Time Off Notice | §4.7 | Upcoming leave; empty if not in class_people |
| IC Drills (×8) | §4.8 | commits, pull-requests, reviews, builds, tasks-completed, cycle-time, task-reopen, bugs-fixed |
| Team Drills (×11) | §4.8 | team-members, team-tasks, team-dev-time, team-prs, team-pr-cycle, team-bugs, team-build, team-focus, team-ai-active, team-ai-loc, team-reopen |

> **Note:** Person Profile (§4.6) is NOT seeded as a metric — use Identity Service directly.

### ~~Backend — Stats / Percentiles (Open Question 1)~~ — RESOLVED

- [x] Seed stats metrics per KPI with `quantile(0.05)`, `quantile(0.50)`, `quantile(0.95)` in `query_ref`
- No separate endpoint needed — same `POST /api/analytics/v1/metrics/{id}/query` path

### ~~Backend — Threshold Config (Open Question 2)~~ — RESOLVED

- [x] Threshold CRUD at `/api/analytics/v1/metrics/{id}/thresholds` (GET/POST/PUT/DELETE)
- [x] Threshold evaluation server-side; `_thresholds` field in every response row
- [ ] Seed initial thresholds for V1 metrics (good/warning/critical per field)

### Backend — Jira connector (unblocks `tasks_closed`, `bugs_fixed`) — **P1 priority**

Without task data the Executive Summary (§4.1) and Team Member metric (§4.2) surface
only Git metrics, which are misleading for management without business delivery context.

- [ ] Confirm `class_tasks` Silver schema with Jira connector author (PR #62)
- [ ] Switch null → real values once connector ships; set `data_availability.tasks = 'available'`

### Backend — CI connector (unblocks `build_success_pct`)

- [ ] GitHub Actions stream (§9 Step 1)
- [ ] Bitbucket Pipelines stream (§9 Step 2)
- [ ] `class_ci_runs` Silver dbt model (§9 Step 3)
- [ ] Identity resolution for CI actors (§9 Step 4)
- [ ] Switch `build_success_pct` null → real; set `data_availability.ci = 'available'`

### Frontend — already implemented (insight-front)

- [x] `DataAvailability` type and nullable field types (`number | null`)
- [x] Null-safe rendering in all tables (`null` → "—")
- [x] `OrgKpiCards` shows "Not configured" when `avgBuildSuccess` is null
- [x] `METRIC_KEYS` catalog in `types/index.ts`
- [ ] Build OData query builder (compose `$filter`, `$orderby`, `$top` from UI state)
- [ ] Switch from period enum to `metric_date` OData `$filter` ranges
- [ ] Read `_thresholds` field from response rows for cell coloring (replaces client-side threshold computation)
- [ ] Move `delta`/`delta_type` computation to frontend (two parallel requests, diff)
- [ ] Add Identity Service call for IC person header (`GET /api/identity-resolution/v1/persons/{id}`)
- [ ] Fetch connector status from Connector Manager in parallel with analytics queries
