# Ingestion Monitoring — operator runbook

Goal: detect "data is missing for several days" before users notice. Today's
coverage is **freshness only** — newly arrived rows in bronze tables. Volume
and Airbyte-job-level signals are listed under [Open work](#open-work).

This file is the operator-facing runbook (verification steps, on-call
matrix, parser exit codes, payload shape). The feature design itself —
purpose, threshold inheritance, acceptance criteria — lives in
[`docs/domain/ingestion-monitoring/specs/feature-bronze-freshness-sla/FEATURE.md`](../../docs/domain/ingestion-monitoring/specs/feature-bronze-freshness-sla/FEATURE.md).
The companion PRD and DESIGN for the broader monitoring domain live next
to it: [`PRD.md`](../../docs/domain/ingestion-monitoring/specs/PRD.md),
[`DESIGN.md`](../../docs/domain/ingestion-monitoring/specs/DESIGN.md).

## What's wired

### Bronze freshness (live)

Every Airbyte-managed bronze source inherits a 30h-warn / 48h-error
threshold against `_airbyte_extracted_at`. The threshold is defined once at
project level in `src/ingestion/dbt/dbt_project.yml`:

```yaml
sources:
  ingestion:
    +freshness:
      warn_after:  { count: 30, period: hour }
      error_after: { count: 48, period: hour }
```

Each source's `schema.yml` then declares the field to check. The right
choice depends on whether the connector is **streaming** or
**report-style**:

```yaml
# Streaming connector — Airbyte cursor follows business time. Rows land in
# bronze approximately when they happen, so the technical extracted-at
# timestamp tracks reality.
sources:
  - name: bronze_<connector>
    schema: bronze_<connector>
    loaded_at_field: _airbyte_extracted_at
    tables: ...
```

```yaml
# Report-style connector — Airbyte re-fetches a fixed window every run
# (e.g. Microsoft Graph reports, Slack admin.analytics.getFile). Even when
# the upstream has not advanced, the sync writes "fresh" rows for older
# business days, so `_airbyte_extracted_at` stays green forever. Anchor on
# the report's own business-day column instead. ISO-8601 strings sort
# lexically, so wrapping in `parseDateTimeBestEffortOrNull(...)` works
# directly inside `loaded_at_field`.
sources:
  - name: bronze_<connector>
    schema: bronze_<connector>
    loaded_at_field: parseDateTimeBestEffortOrNull(reportRefreshDate)
    tables: ...
```

Active per-source assignments. Threshold tier ("default" = 30h/48h, "event"
= 72h/96h) is set in the same `schema.yml` block; values are
Helm-controlled via `ingestion.freshness.thresholds.*` (envvar fallthrough
keeps local `dbt source freshness` working without Helm rendering).

| Source / table | `loaded_at_field` | Tier | Notes |
|---|---|---|---|
| bronze_bamboohr (employees) | `_airbyte_extracted_at` | default | Full-refresh roster — sync-alive signal |
| bronze_bitbucket_cloud.* | `_airbyte_extracted_at` | default | Mostly incremental (insert-on-event) |
| bronze_claude_admin.* | `_airbyte_extracted_at` | default | |
| bronze_claude_enterprise.* | `_airbyte_extracted_at` | default | |
| bronze_confluence.wiki_page_versions | `parseDateTime64BestEffortOrNull(created_at, 3)` | **event** | Connector re-emits version table; quiet weekends real |
| bronze_cursor.cursor_daily_usage | `fromUnixTimestamp64Milli(toInt64OrZero(toString(date)))` | default | Daily aggregate; weekend rows still emitted |
| bronze_cursor (other tables) | `_airbyte_extracted_at` | default | |
| bronze_github.* | `_airbyte_extracted_at` | default | |
| bronze_jira.* | `_airbyte_extracted_at` | default | Event-style streams (incremental cursor) |
| bronze_m365.{teams,email,onedrive,sharepoint}_activity | `parseDateTimeBestEffortOrNull(reportRefreshDate)` | **report** (48h/96h) | Microsoft Graph reports document 24-48h publish lag; warn at upper baseline |
| bronze_openai.* | `_airbyte_extracted_at` | default | |
| bronze_slack.users_details | `parseDateTimeBestEffortOrNull(date)` | **report_extended** (72h/120h) | Slack admin.analytics typical 3-day lag, up to 5 during Slack maintenance |
| bronze_zoom.meetings | `parseDateTimeBestEffortOrNull(start_time)` | **event** | Re-fetches 30-day window; quiet weekends real |
| bronze_zoom.participants | `parseDateTimeBestEffortOrNull(join_time)` | **event** | |
| bronze_zoom.users | (opted out via `freshness: null`) | — | Roster |

Re-categorizing a connector across tiers is an engineering change (it
usually comes with a `loaded_at_field` revisit), not an ops dial — that's
why the mapping lives in connector `schema.yml`, not in Helm values.

`loaded_at_field` is a dbt **property**, not a config — `+loaded_at_field`
at project level is silently ignored. The dbt-clickhouse adapter does not
support metadata-based freshness, so a source missing `loaded_at_field`
fails with `runtime error` instead of falling back to a default.

A single CronWorkflow `dbt-source-freshness-check` runs `dbt source freshness`
daily at 13:00 UTC (after every connector's sync window of 02:00–11:00 UTC)
and parses `target/sources.json`. Any source in `warn` or `error` is logged
with name, max-loaded-at and lag; if a notification driver is configured
under `ingestion.freshness.notification.driver` (one of `webhook`, `zulip`,
`slack`, `teams`, `email`), the breach list is dispatched through the
matching driver. Driver shapes and credential bindings are documented in
[`docs/domain/ingestion-monitoring/specs/DESIGN.md` §3.3](../../docs/domain/ingestion-monitoring/specs/DESIGN.md#33-api-contracts).

| Status | Meaning | Workflow exit | What to do |
|--------|---------|---------------|------------|
| `pass` | MAX(`<anchor>`) within the source's tier `warn_after` window | 0 | Nothing |
| `warn` | between `warn_after` and `error_after` (one missed run for daily-cadence sources) | 0 (visible in log + payload) | Investigate during business hours |
| `error` | past `error_after` for the source's tier | 1 (Argo Failed) | Page |
| `runtime error` | dbt couldn't even check the source (CH down, schema drift, query failure) | 1 (Argo Failed) | Page — investigate before trusting other sources |

Concrete tier values come from the connector's source-level `freshness:`
block and the Helm overlay (defaults `default` 30h/48h, `event` 72h/96h,
`report` 48h/96h, `report_extended` 72h/120h).

`error` and `runtime error` flip the workflow to Failed so Argo retains the
run in `failedJobsHistoryLimit`. Warn-only runs stay Successful — the breach
is still printed to the workflow log and POSTed in the notification payload,
but on-call doesn't get paged on a single missed sync.

### How it stays generic

New connectors **do not** need to repeat the freshness block. They get the
default the moment they declare a `bronze_*` source under
`src/ingestion/connectors/.../dbt/schema.yml`. Two knobs the connector author
controls:

1. **Per-table opt-out** — slow-moving lookup/catalog streams (e.g.,
   `jira_statuses`, `claude_admin_workspaces`, `bamboohr.meta_fields`) set
   `freshness: null` next to the table in `schema.yml`. A quiet day is
   legitimate for those.
2. **Tighter SLA** — sub-daily connectors (none today) override at the source
   level in `schema.yml` with their own `freshness:` block.

That's it. No per-connector pipeline plumbing.

## Adding a new connector — freshness checklist

1. In the new `connectors/<category>/<name>/dbt/schema.yml`, declare the
   `bronze_*` source.
2. Pick **one** of the three valid forms:
   - **Streaming / event-cursor** (commits, issues, edits, etc.) — `loaded_at_field: _airbyte_extracted_at`. Inherits the default tier (30h/48h).
   - **Report-style daily snapshot** (a connector that re-fetches a fixed window every run — e.g. Microsoft Graph reports, Slack admin.analytics, Cursor daily_usage) — `loaded_at_field: parseDateTimeBestEffortOrNull(<business_date_col>)`. Inherits default tier; weekend rows still expected because the upstream API publishes them.
   - **Event-style with quiet weekends** (Confluence edits, Zoom meetings — connectors where a Saturday with zero rows is normal) — `loaded_at_field: ...(event_ts)` AND a source-level `freshness:` block that anchors on the event tier:

     ```yaml
     freshness:
       warn_after:  { count: "{{ env_var('FRESHNESS_WARN_EVENT_H',  '72') | int }}", period: hour }
       error_after: { count: "{{ env_var('FRESHNESS_ERROR_EVENT_H', '96') | int }}", period: hour }
     ```

3. For roster / catalog tables (rarely-changing lookup streams) add `freshness: null` per-table.

4. CI runs `src/ingestion/scripts/lint-bronze-freshness.py` against every PR
   that touches connector schemas — it fails the build if any `bronze_*`
   source is missing `loaded_at_field` and is not opted out. The check can
   be run locally:

   ```bash
   python3 src/ingestion/scripts/lint-bronze-freshness.py
   ```

   (uses PyYAML; the toolbox image has it already)

5. **Trap to avoid**: if your connector re-emits a fixed window every run
   (`SELECT count(), max(_airbyte_extracted_at) - min(_airbyte_extracted_at)
   FROM bronze_<x>.<table>` shows all rows extracted within the last 24h
   even though the table covers many days of history), `_airbyte_extracted_at`
   will look fresh forever. Use the report-style or event-style form
   instead. The lint can't detect this — it's a judgment call about the
   source shape, documented per-source in the assignment table below.

## Who consumes the signal

Per-environment ownership matrix until the delivery channel is wired (see
[Open work](#open-work)).

Ownership for "ingestion on-call" and "connector owner" is not assigned yet
(no rotation document, no `CODEOWNERS` for `src/ingestion/connectors/*` as
of this commit). The matrix below describes the *roles* the freshness
signal expects to land on; see [Open work](#open-work) for the rotation gap.

| Role | What they read | When | Action |
|---|---|---|---|
| Ingestion on-call (TBD) | Argo UI / `kubectl get workflows -n argo --sort-by=.metadata.creationTimestamp` for the `dbt-source-freshness-check` runs | Daily, after the 13:00 UTC run | Triage `error` / `runtime error` runs |
| `cyberfabric/insight` repo Issues | One issue per persistent breach (>2 consecutive runs) opened by the on-call | Within 1 business day of the breach | Hand off to the connector owner |
| Connector owner (TBD per connector) | The issue body — includes the failing source, max-loaded-at, lag in hours | On issue assignment | Fix the connector or update the SLA |
| Tenant on-call (post-MVP) | Webhook payload (Zulip / email / generic POST) routed by `cluster` field | Real-time | Same triage as above, scoped to one deployment |

Until the webhook channel lands, the **only** push mechanism is Argo's
failed-runs list — on-call must check `kubectl get workflows -n argo
--sort-by=.metadata.creationTimestamp` at least once per business day. The
`failedJobsHistoryLimit: 5` ensures the latest five breaching runs are
retained for inspection.

`pass`-only runs leave no trace beyond Argo's success history (kept by
`successfulJobsHistoryLimit: 3`) — silent green is the desired steady state.

## Open work

### Rotation / ownership — not assigned

The matrix above describes roles, not people. There is no documented
ingestion on-call rotation as of this commit, and `src/ingestion/connectors/`
has no `CODEOWNERS` entries assigning per-connector owners. Until that
lands, the freshness signal lives in Argo's failed-runs list with no
named consumer.

Action: agree on an on-call rotation (or a single owner during MVP) and
add a `CODEOWNERS` block listing the per-connector owner so the breach
hand-off has a real target.

### Delivery channel — driver-based

`charts/insight/values.yaml` exposes notification routing under
`ingestion.freshness.notification.*` (driver selector + per-driver
subblock). When `driver: ""` (the default), breaches surface only via:

- Argo UI / `kubectl get workflows -n argo -l app.kubernetes.io/component=ingestion-monitoring`
- Workflow exit status (failed runs accumulate in
  `failedJobsHistoryLimit: 5`)

Setting `driver` to one of `webhook`, `zulip`, `slack`, `teams`, `email`
activates the matching driver's dispatcher; the URL or SMTP password is
read from a Kubernetes Secret on a `secretKeyRef` env binding so the
credential never lands in rendered manifests or Argo UI. Driver shapes,
secret bindings, and observed quirks (Zulip endpoint rewrite, Cloudflare
UA, Slack mrkdwn vs Zulip native) are documented in
[`docs/domain/ingestion-monitoring/specs/DESIGN.md` §3.3](../../docs/domain/ingestion-monitoring/specs/DESIGN.md#33-api-contracts).

The webhook driver's body remains provider-agnostic so a custom relay
can sit in front of any of the above:

```json
{
  "topic": "ingestion-freshness",
  "cluster": "prod",
  "tenant": "acme-co",
  "summary": "[cluster=prod, tenant=acme-co] 3 bronze source(s) breaching freshness SLA",
  "breaches": [
    {
      "source": "source.ingestion.bronze_jira.jira_issue",
      "status": "error",
      "max_loaded_at": "2026-04-28T03:14:21Z",
      "age_hours": 51.2,
      "empty": false
    }
  ]
}
```

`cluster` and `tenant` come from `ingestion.freshness.{cluster,tenant}`
in values overrides — both empty by default; the `summary` drops empty
labels from its prefix.

### Volume baseline (next iteration)

Freshness catches "no rows arrived" but misses "API returned 50 rows instead
of 5000". Plan: a singular SQL test (or dbt operation) that compares today's
row count per stream against a 14-day median, alerts on <30%. Reuses the same
`dbt-source-freshness` workflow shell — just a different selector.

### Source vs bronze attribution

Today the freshness check flags "no rows in bronze" but cannot tell whether
the upstream Airbyte sync ran. To distinguish:

- *source/credential issue*: Airbyte sync ✅, bronze pulled 0 rows
- *pipeline issue*: Airbyte sync ❌ (didn't run / errored)

We'd need a sidecar that polls the Airbyte Jobs API after each sync and
appends a row to `staging.airbyte_runs (connection_id, status,
records_emitted, started_at, ended_at)`. Then the freshness step can JOIN
against that table and label each breach with a root cause. Future PR.

## How to verify locally

```bash
# After dev-up.sh + at least one successful sync
kubectl create -n argo -f - <<'EOF'
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: freshness-adhoc-
  namespace: argo
spec:
  workflowTemplateRef:
    name: dbt-source-freshness
  arguments:
    parameters:
      - name: dbt_select
        value: "source:bronze_jira"
      - name: toolbox_image
        value: "insight-toolbox:local"
      - name: clickhouse_host
        value: "insight-clickhouse.insight.svc.cluster.local"
      - name: clickhouse_port
        value: "8123"
      - name: clickhouse_user
        value: "default"
EOF

# Watch the run
kubectl logs -n argo -l workflows.argoproj.io/workflow=<name> -f
```
