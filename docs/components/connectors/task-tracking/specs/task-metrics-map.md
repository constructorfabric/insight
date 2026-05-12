# Task metrics — silver-to-dashboard map

## Scope and goal

This document maps every **Jira-fed task metric** from the FE dashboard spec
(`frontend-metrics-map-1776414906885.md`) to concrete SQL recipes over the
existing `silver.class_task_*` tables produced by the Jira connector pipeline,
and enumerates the data gaps that must be filled before each metric becomes
computable.

The target layering mirrors the upstream `silver/git/` convention:

```
silver/task_tracking/
  class_tasks.sql                (class union — 1 row per task, source-neutral)
  class_task_changelog.sql       (1 row per status/sprint/assignee change)
  class_task_comments.sql        (already exists as class_task_comments)
  class_task_worklogs.sql        (already exists as class_task_worklogs)

  fct_task.sql                   (row-level enrichment: person_key, week, cycle_time_h, state_norm)
  fct_task_status_span.sql       (1 row per (task × status span) — needed for dev_time)
  fct_task_sprint_membership.sql (1 row per (task × sprint × in/out event) — for scope metrics)

  mtr_task_person_daily.sql      (aggregate: per (tenant_id, person_key, day))
  mtr_task_person_totals.sql     (aggregate: per (tenant_id, person_key))
```

All aggregates land on the `(tenant_id, person_key, day)` grain so FE bullet
charts can consume them directly (matching `insight.jira_closed_tasks` shape in
the existing upstream workaround views).

## Identity resolution

`silver.class_task_users.user_id` = Atlassian `accountId`.
`silver.class_task_users.email` is present for most users but may be null when
Jira Cloud privacy settings hide it.

For every task metric the join is:

```sql
LEFT JOIN silver.class_task_users FINAL u
       ON u.insight_source_id = t.insight_source_id
      AND u.user_id = t.<author_field>
-- person_key with explicit fallback so privacy-hidden users (no email
-- exposed) still roll up consistently under their stable user_id:
--   person_key = COALESCE(lower(nullIf(u.email, '')), t.<author_field>)
```

Resolution to canonical `person_id` (across all connectors via `class_people`)
is out of scope for this document; it will be wired through a separate identity
service.

## Status → lifecycle mapping (hardcoded v1)

Jira Cloud stores status in two related concepts:

- `status.name` — per-project schema-defined human label
  (e.g. `"In Progress"`, `"Done"`, `"Verified"`, `"Closed"`)
- `status.statusCategory.key` — canonical three-state machine:
  - `new` — not yet touched
  - `indeterminate` — work in flight
  - `done` — terminal / archived

FE dashboards want the canonical axis (`tasks_closed`, `task_dev_time`), so the
pipeline **maps `status_id → category` via the lookup `silver.class_task_statuses`**
(which carries `category_id` / `category_key` / `category_name` fields extracted
from Jira's `/rest/api/3/status` endpoint).

Metric SQL below joins on `category_key` — the stable enum (`new` /
`indeterminate` / `done`) — not `category_name`, which is the localized human
label and varies per Jira language setting.

> **Hardcoded mapping for v1**: the category keys `new`, `indeterminate`, `done`
> come directly from the Jira API response. When a second task-tracker
> connector is added (YouTrack), that connector must produce the same three
> values in its `class_task_statuses.category_key` export.

## Field inventory — what the existing silver exposes

### `silver.class_task_field_history` (event-sourced)

Every task field change is a row. The schema below enumerates what
`field_id` values are populated by the Jira enrich binary and how they map to
concrete metric inputs.

| `field_id` | Source | Populated via | Used by metrics |
|---|---|---|---|
| `status` | `jira_issue.status_id` + changelog | `synthetic_initial` + `changelog` | tasks_closed, task_dev_time, task_reopen_rate, bug_reopen_rate |
| `priority` | `jira_issue.priority_id` | both | — |
| `issuetype` | `jira_issue.issuetype_id` | both | bugs_fixed, bug_reopen_rate |
| `resolution` | `jira_issue.resolution_id` | both | due_date_compliance, on_time_delivery (indirectly) |
| `assignee` | `jira_issue.assignee_id` | both | all per-person metrics |
| `reporter` | `jira_issue.reporter_id` | both | — |
| `parent` | `jira_issue.parent_id` | both | — |
| `project` | `jira_issue.project_key` | synthetic_initial only | — |
| `labels` | `jira_issue.labels_csv` | both | — |
| `story_points` | `jira_issue.story_points` | both | *(replacement for time estimates where not used)* |
| `due_date` | `jira_issue.due_date` | synthetic_initial only | due_date_compliance, avg_slip, on_time_delivery |
| **`timeoriginalestimate`** | **⚠ missing** — extract from `bronze.jira_issue.custom_fields_json` or add AddFields | — | estimation_accuracy, overrun_ratio |
| **`timespent`** | **⚠ missing** — same | — | estimation_accuracy, overrun_ratio |
| **`resolutiondate`** | **⚠ missing** — same | — | on_time_delivery, avg_slip, task_reopen_rate (precise timing) |
| **`Sprint`** | **⚠ partial** — Jira logs sprint changes under a custom field (`customfield_<id>`); not currently extracted as a tracked field | — | scope_completion, scope_creep |

### Lookup tables

- `silver.class_task_statuses` — `status_id → name, category_id, category_name`
- `silver.class_task_users` — `user_id (accountId) → email, display_name, is_active`
- `silver.class_task_sprints` — `sprint_id → board_id, state, start_date, end_date, complete_date`

### Supporting silver tables

- `silver.class_task_comments`, `silver.class_task_worklogs`,
  `silver.class_task_projects` — already populated by dbt silver models.

## Metric-by-metric spec

All metrics below assume a daily aggregate grain:
`(tenant_id, person_key, metric_date)` — `metric_date` is the day in UTC on
which the **event that defines the metric** happened (e.g. for `tasks_closed`,
the day the status transitioned to `done`).

### 1. `tasks_closed` / `tasks_completed` / `tasks_done`

All three metric_key's in the FE map correspond to the same measurement.

**Inputs**: `silver.class_task_field_history`, `silver.class_task_statuses`.

**Logic**: count status-transition events where the NEW status belongs to
category `done`.

```sql
WITH done_events AS (
  SELECT
    fh.insight_source_id,
    fh.data_source,
    fh.issue_id,
    fh.id_readable,
    fh.author_id,
    fh.event_at,
    toDate(fh.event_at)   AS metric_date
  FROM silver.class_task_field_history FINAL fh
  INNER JOIN silver.class_task_statuses FINAL s
          ON s.insight_source_id = fh.insight_source_id
         AND s.status_id         = fh.delta_value_id
  WHERE fh.data_source     = 'jira'
    AND fh.field_id        = 'status'
    AND fh.delta_action    = 'set'
    AND fh.event_kind      = 'changelog'   -- synthetic_initial excluded: rows
                                           -- started in Done are not "closures"
    AND s.category_name    = 'Done'
),
assignee_at_close AS (
  -- Resolve the assignee as of the closing event by looking up the latest
  -- `assignee` change at or before event_at per (issue, close_ts).
  SELECT
    de.*,
    argMaxIf(fh2.delta_value_id, fh2.event_at, fh2.event_at <= de.event_at) AS assignee_id
  FROM done_events de
  LEFT JOIN silver.class_task_field_history FINAL fh2
         ON fh2.insight_source_id = de.insight_source_id
        AND fh2.issue_id          = de.issue_id
        AND fh2.field_id          = 'assignee'
  GROUP BY de.insight_source_id, de.data_source, de.issue_id, de.id_readable,
           de.author_id, de.event_at, de.metric_date
)
SELECT
  insight_source_id,
  lower(u.email)                AS person_key,
  metric_date,
  count()                       AS tasks_closed
FROM assignee_at_close ac
LEFT JOIN silver.class_task_users FINAL u
       ON u.insight_source_id = ac.insight_source_id
      AND u.user_id           = ac.assignee_id
GROUP BY insight_source_id, person_key, metric_date
```

**Notes**:
- `event_kind = 'changelog'` excluded because a task that appears *already* in
  Done at its first sync (legacy data) should not be counted as closed that day.
- `assignee_at_close` resolves the reporter / assignee at the moment of the
  status transition, not the current state.

**Gaps**: none — computable today.

### 2. `bugs_fixed`

Same as `tasks_closed` but filtered by issuetype.

```sql
-- prepend to `done_events` CTE:
INNER JOIN silver.class_task_field_history FINAL fh_type
        ON fh_type.insight_source_id = fh.insight_source_id
       AND fh_type.issue_id          = fh.issue_id
       AND fh_type.field_id          = 'issuetype'
       -- take the issuetype active at close time:
       AND fh_type.event_at         <= fh.event_at
-- plus:  JOIN issuetype lookup (if available) or match by display name
WHERE ... AND fh_type.delta_value_display = 'Bug'
```

**Gaps**:
- The issuetype lookup `silver.class_task_issuetypes` doesn't currently
  exist as a silver model (it's only in bronze). Either add a silver model, or
  filter by `delta_value_display = 'Bug'` as a v1 hack and plan to swap in the
  lookup table later.

### 3. `task_dev_time` (hours in In-Progress-category statuses)

Duration from `indeterminate` category entry to exit, per issue per assignee.

**Logic**: build *status spans* from consecutive status events on the same
issue; include spans whose status category is `indeterminate` (i.e.,
actively worked on); sum per assignee per day.

```sql
-- fct_task_status_span.sql (materialized)
WITH status_events AS (
  SELECT
    fh.insight_source_id,
    fh.issue_id,
    fh.event_at,
    fh.delta_value_id AS status_id,
    s.category_name
  FROM silver.class_task_field_history FINAL fh
  LEFT JOIN silver.class_task_statuses FINAL s
         ON s.insight_source_id = fh.insight_source_id
        AND s.status_id         = fh.delta_value_id
  WHERE fh.data_source  = 'jira'
    AND fh.field_id     = 'status'
    AND fh.delta_action = 'set'
),
spans AS (
  SELECT
    insight_source_id,
    issue_id,
    event_at            AS span_start,
    lead(event_at, 1, now64(3)) OVER (
      PARTITION BY insight_source_id, issue_id
      ORDER BY event_at
    )                   AS span_end,
    status_id,
    category_name
  FROM status_events
)
-- aggregate:
SELECT
  insight_source_id,
  ac.person_key,
  toDate(span_start)                                AS metric_date,
  sum(dateDiff('second', span_start, span_end)) / 3600.0 AS dev_time_h
FROM spans s
JOIN assignee_at_time ac USING (insight_source_id, issue_id, span_start)
WHERE s.category_name = 'In Progress'
GROUP BY insight_source_id, person_key, metric_date
```

**Notes**:
- Cross-day spans should be split by day boundary if exactness matters. V1 can
  attribute the whole span to the `span_start` day.
- "In Progress" here refers to the `statusCategory.name = "In Progress"`
  (Jira's canonical label for `indeterminate`), not the literal status name.

**Gaps**: none; `fct_task_status_span` is a materialized output built from the
existing fh table.

### 4. `task_reopen_rate`

> "Reopened within 14d" / "total closed" * 100 per person.

**Logic**: for each issue, find pairs of status-events
(`done at t1`) followed by (`non-done at t2`) where `t2 - t1 <= 14 days`. Count
the `t1` events as "closed" and as "reopened if paired with such a t2".

```sql
WITH close_reopen_pairs AS (
  SELECT
    t1.insight_source_id,
    t1.issue_id,
    t1.event_at AS close_at,
    t1.assignee_at_close,
    any(t2.event_at) AS reopen_at   -- first reopen after close
  FROM (
    -- every done-transition for every issue
    SELECT fh.*, toDate(event_at) AS close_date
    FROM silver.class_task_field_history FINAL fh
    JOIN silver.class_task_statuses FINAL s
      ON s.insight_source_id = fh.insight_source_id
     AND s.status_id         = fh.delta_value_id
    WHERE field_id='status' AND delta_action='set'
      AND s.category_key='done' AND fh.event_kind='changelog'
  ) t1
  LEFT JOIN silver.class_task_field_history FINAL t2
         ON t2.insight_source_id=t1.insight_source_id
        AND t2.issue_id=t1.issue_id
        AND t2.field_id='status' AND t2.delta_action='set'
        AND t2.event_at > t1.event_at
        AND t2.event_at <= t1.event_at + INTERVAL 14 DAY
  LEFT JOIN silver.class_task_statuses FINAL s2
         ON s2.insight_source_id = t2.insight_source_id
        AND s2.status_id         = t2.delta_value_id
  WHERE s2.category_key != 'done' OR s2.category_key IS NULL
  GROUP BY t1.insight_source_id, t1.issue_id, t1.event_at, t1.assignee_at_close
)
SELECT
  person_key,
  toDate(close_at) AS metric_date,
  countIf(reopen_at IS NOT NULL) / greatest(count(), 1) * 100 AS task_reopen_rate
FROM close_reopen_pairs
GROUP BY person_key, metric_date
```

**Gaps**: none.

### 5. `due_date_compliance`

> Of tasks that have a due_date, what % was closed on or before it?

**Inputs**: `tasks_closed` (above) joined with synthetic_initial `due_date`
(or latest `due_date` changelog value).

```sql
WITH due_date_at_close AS (
  SELECT
    de.*,
    argMaxIf(
      parseDateTime64BestEffortOrNull(fh.delta_value_display, 3),
      fh.event_at,
      fh.event_at <= de.event_at
    ) AS due_date
  FROM done_events de
  LEFT JOIN silver.class_task_field_history FINAL fh
         ON fh.insight_source_id = de.insight_source_id
        AND fh.issue_id          = de.issue_id
        AND fh.field_id          = 'due_date'
  GROUP BY de.*
)
SELECT
  person_key,
  metric_date,
  countIf(due_date IS NOT NULL AND event_at <= due_date)
    / greatest(countIf(due_date IS NOT NULL), 1)
    * 100                                     AS due_date_compliance
FROM due_date_at_close
LEFT JOIN class_task_users FINAL u ...
GROUP BY person_key, metric_date
```

**Gaps**: none. `due_date` already flows through fh as a tracked field.

### 6. `on_time_delivery`

Same input as `due_date_compliance` but denominator is `COUNT(*)`, not just
tasks with a due_date.

```sql
SELECT
  person_key,
  metric_date,
  countIf(due_date IS NOT NULL AND event_at <= due_date) / greatest(count(), 1) * 100
FROM due_date_at_close
GROUP BY person_key, metric_date
```

### 7. `avg_slip`

> Average by how many days the task was late.

```sql
SELECT
  person_key,
  metric_date,
  avgIf(dateDiff('day', due_date, event_at),
        due_date IS NOT NULL AND event_at > due_date)  AS avg_slip_days
FROM due_date_at_close
GROUP BY person_key, metric_date
```

### 8. `estimation_accuracy` & `overrun_ratio`

> `estimation_accuracy` = % of tasks where actual within ±20% of estimate.
> `overrun_ratio` = median(actual / estimate).

**Gaps**:
- `timeoriginalestimate` and `timespent` are **not currently extracted** into
  `bronze.jira_issue` (no AddFields → no column in bronze → not in silver fh).
- Need to extract these fields (they live inside `custom_fields_json` today).

**Required connector.yaml change**:
```yaml
- path: [time_original_estimate_sec]
  value: "{{ record.get('fields', {}).get('timeoriginalestimate') }}"
- path: [time_spent_sec]
  value: "{{ record.get('fields', {}).get('timespent') }}"
```
plus include these as tracked fields in the Rust enrich's snapshot+changelog
pipeline, so fh carries the history of their values. As a simpler v1, leave
them as plain bronze columns and query bronze directly.

**SQL (once fields are present in bronze)**:
```sql
WITH closed_with_times AS (
  SELECT
    de.*,
    i.time_original_estimate_sec,
    i.time_spent_sec
  FROM done_events de
  JOIN bronze_jira.jira_issue FINAL i USING (id_readable)
  WHERE i.time_original_estimate_sec > 0
)
SELECT
  person_key, metric_date,
  countIf(abs(time_spent_sec - time_original_estimate_sec)
          <= 0.2 * time_original_estimate_sec)
    / greatest(count(), 1) * 100                     AS estimation_accuracy,
  quantile(0.5)(time_spent_sec / time_original_estimate_sec) AS overrun_ratio
FROM closed_with_times
GROUP BY person_key, metric_date
```

### 9. `scope_completion` & `scope_creep`

> `scope_completion` = tasks done / tasks committed at sprint start
> `scope_creep` = tasks added after sprint start / tasks committed at sprint start

**Gaps**:
- Jira tracks sprint membership via a custom field (`customfield_<id>`). It's
  not currently treated as a first-class tracked field in the Rust enrich
  (our extracted fields focus on status, priority, etc.).
- Per-issue sprint membership history **is** present in the raw bronze
  changelog (`items[]` with `fieldId` containing the sprint customfield id),
  so it's derivable without any connector-level change **provided** the
  pipeline can identify which custom field id is the Sprint field.

**Required additions**:
1. Config: surface `jira_sprint_field_id` (like we already did for
   `jira_story_points_field_id`). Default to `customfield_10020` (most common).
2. Rust enrich: classify events with `field_id == jira_sprint_field_id` under
   `field_id='sprint'` in silver fh, with `value_ids = sprint_ids[]` (multi).
3. Metric SQL: for each (issue × sprint) pair, find when the issue joined the
   sprint (first `add`) vs sprint `start_date`.

```sql
WITH issue_sprint_events AS (
  -- Each sprint add/remove event targets a single sprint id in `delta_value_id`.
  -- No ARRAY JOIN on `value_ids` here — that array is the FULL post-change
  -- sprint membership (multi-value snapshot), and exploding it would produce
  -- N rows per event and mis-attribute scope_creep.
  SELECT
    fh.insight_source_id,
    fh.issue_id,
    sp.sprint_id,
    fh.event_at                AS joined_at,
    fh.delta_action            AS action,       -- 'add' | 'remove'
    sp.start_date, sp.complete_date
  FROM silver.class_task_field_history FINAL fh
  JOIN silver.class_task_sprints FINAL sp
    ON sp.insight_source_id = fh.insight_source_id
   AND sp.sprint_id         = fh.delta_value_id
  WHERE fh.field_id = 'sprint'
    AND fh.delta_action IN ('add', 'remove')
),
sprint_commitments AS (
  SELECT insight_source_id, sprint_id,
         anyIf(issue_id, action='add' AND joined_at <= start_date) AS committed_issue,
         anyIf(issue_id, action='add' AND joined_at >  start_date) AS added_issue
  FROM issue_sprint_events
  GROUP BY insight_source_id, sprint_id, issue_id
),
sprint_resolved AS (
  SELECT sprint_id,
         countDistinct(committed_issue)                       AS committed_count,
         countDistinctIf(committed_issue, /* done by complete_date */ ...) AS done_from_committed,
         countDistinct(added_issue)                           AS added_count
  FROM sprint_commitments
  GROUP BY sprint_id
)
SELECT
  sprint_id,
  done_from_committed / greatest(committed_count, 1) * 100 AS scope_completion,
  added_count / greatest(committed_count, 1) * 100         AS scope_creep
FROM sprint_resolved
```

**Notes**:
- The metric is per-sprint, not per-day-per-person. To fit the FE daily-per-person
  shape, attribute to the sprint's complete_date and the assignee at close time.
- `ARRAY JOIN` needed because `fh.value_ids` is the multi-value array for
  multi-cardinality fields like sprint.

### 10. `bug_reopen_rate`

Same as `task_reopen_rate` but filtered to `issuetype = 'Bug'` at close time.
No additional data requirements.

## Missing fields — summary

| Field | Where from | Used by | Priority | Fix |
|---|---|---|---|---|
| `resolutiondate` | Jira `fields.resolutiondate` | on_time_delivery, avg_slip (exact timing vs status event) | Nice-to-have | AddFields in connector.yaml jira_issue |
| `time_original_estimate_sec` | Jira `fields.timeoriginalestimate` | estimation_accuracy, overrun_ratio | Required | AddFields |
| `time_spent_sec` | Jira `fields.timespent` | estimation_accuracy, overrun_ratio | Required | AddFields |
| `sprint` (tracked field) | Jira `customfield_<sprint-id>` | scope_completion, scope_creep | Required | Rust enrich track + config |
| `silver.class_task_issuetypes` | bronze `jira_issuetypes` | bugs_fixed (canonical type lookup) | Nice-to-have | dbt silver model (mirror of class_task_statuses) |

After those three AddFields entries + sprint-field tracking, **every metric in
the FE map is computable** using `silver.class_task_*`.

## Proposed dbt models

| Model | Grain | Sources | Shape |
|---|---|---|---|
| `class_tasks` | 1 row per task (current state) | `bronze.jira_issue` FINAL + lookups | source-neutral task projection with `issue_id, id_readable, project_key, title, type, status, status_category, priority, resolution, assignee_id, reporter_id, created_at, updated_at, resolved_at, due_date, time_original_estimate_sec, time_spent_sec, story_points` |
| `class_task_changelog` | 1 row per status / sprint / assignee / estimate change | `silver.class_task_field_history` | filtered projection for common fields |
| `fct_task` | 1 row per task | `class_tasks` + identity | adds `person_key`, `team`, `week`, `state_norm ∈ {open, in_progress, done}`, `cycle_time_h = resolved_at - created_at` |
| `fct_task_status_span` | 1 row per task × status span | from `class_task_changelog` | `(issue_id, status_id, status_category, span_start, span_end, person_key)` |
| `fct_task_sprint_membership` | 1 row per task × sprint × action | from `class_task_changelog` | `(issue_id, sprint_id, action, event_at)` |
| `mtr_task_person_daily` | `(tenant_id, person_key, day)` | all fct_* | `tasks_closed, bugs_fixed, task_dev_time_h, task_reopen_rate, due_date_compliance, on_time_delivery, avg_slip_days, estimation_accuracy, overrun_ratio, bug_reopen_rate` |
| `mtr_task_person_totals` | `(tenant_id, person_key)` | rollup of daily | lifetime totals for the Team Member table |
| `mtr_task_sprint` | `(tenant_id, sprint_id)` | `fct_task_sprint_membership` + `fct_task` | `scope_completion, scope_creep` |

`mtr_task_person_daily` is the direct replacement for the upstream
`insight.jira_closed_tasks` workaround table and will eventually feed
`insight.task_delivery_bullet_rows` once the workaround views are removed.

## What is deliberately NOT tracked here

- `team`, `org_unit_id` attribution — carried by `class_people` once identity
  resolution is wired. Until then `person_key` alone is enough for the
  `task_delivery_bullet_rows` view.
- Cross-connector unions (YouTrack) — `class_tasks` is defined with a
  `data_source` column so a second source just UNION ALL's into it.
- Subtask aggregation — all metrics treat every issue as one unit regardless
  of parent/child relation. Revisit if the FE asks for
  "parent story rollup".
