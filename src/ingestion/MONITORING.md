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

Every bronze source declares its own `freshness:` block + `loaded_at_field`
**per source in the connector's `schema.yml`** (not inherited from a project
default — `dbt_coverage.py` counts per-source declarations, so each source
declares explicitly). The declaration half is tracked under EPIC #1321 / #1322
(PR #1346 + the per-connector anchoring in PR SharedQA/insight#1); this
CronWorkflow only *runs* the check and acts on the verdict.

The critical choice is **which column** `loaded_at_field` points at, and it
depends on whether the connector is **incremental** or **windowed**:

- **Incremental** (git, jira, youtrack — rows land as events happen) →
  `loaded_at_field: _airbyte_extracted_at`. The technical extract timestamp
  tracks reality because the cursor only advances when new data arrives.
- **Windowed / vendor-analytics** (M365 Graph, ChatGPT/Claude Team, Slack
  analytics, Cursor daily, OpenAI usage — the API re-emits a fixed reporting
  window every sync) → anchor on the **business-date column**
  (`parseDateTimeBestEffortOrNull(<col>)`). Here `_airbyte_extracted_at` is
  re-stamped every run and would be **false-green**: verified on live data,
  M365 business data was 4.5 days stale while `_airbyte_extracted_at` was 10.6h
  fresh. See the "Trap to avoid" item in the new-connector checklist below.

Examples of the two forms:

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

Active per-source assignments. The `warn_after`/`error_after` thresholds are
**literal values in each source's `schema.yml`** (not Helm/env-var driven), so
there is a single source of truth and the local `dbt source freshness` reads
exactly what ships. Tiers used: **default** 36h/72h · **report** 72h/120h ·
**report_extended** 120h/168h · **event** 96h/168h.

| Source / table | `loaded_at_field` | Tier | Confidence |
|---|---|---|---|
| bronze_m365.*_activity | `parseDateTimeBestEffortOrNull(reportRefreshDate)` | report | **verified** (live) |
| bronze_chatgpt_team.{chat_activity,codex_user_daily} | `parseDateTimeBestEffortOrNull(date)` | report | **verified** |
| bronze_chatgpt_team.{subscription_usage,subscription_balance} | `parseDateTimeBestEffortOrNull(snapshot_date)` | report | inferred |
| bronze_claude_team.code_metrics | `parseDateTimeBestEffortOrNull(metric_date)` | report | **verified** |
| bronze_cursor.cursor_daily_usage | `parseDateTimeBestEffortOrNull(day)` | report | inferred (daily resync re-fetch) |
| bronze_slack.users_details | `parseDateTimeBestEffortOrNull(date)` | report_extended | inferred (3–5d Slack lag) |
| bronze_zoom.{meetings,participants} | `parseDateTimeBestEffortOrNull(end_time/join_time)` | event | inferred (30d window; quiet weekends real) |
| bronze_openai.usage_*/costs | `parseDateTimeBestEffortOrNull(bucket_start_time)` | report | inferred |
| bronze_claude_admin.{messages_usage,cost_report,code_usage} | `parseDateTimeBestEffortOrNull(date)` | report | inferred |
| bronze_claude_enterprise.summaries | `parseDateTimeBestEffortOrNull(date)` | report | inferred |
| bronze_github_copilot.{user,org}_metrics | `parseDateTimeBestEffortOrNull(day)` | report | inferred |
| bronze_confluence.wiki_pages / bronze_outline.wiki_pages | `parseDateTimeBestEffortOrNull(updated_at)` | event | inferred |
| git / jira / youtrack / figma / salesforce / hubspot / zulip_proxy | `_airbyte_extracted_at` | default | incremental — extracted_at tracks reality |
| rosters & lookups (bamboohr, ms_entra, workday, *_members, *_seats, *_users, *_statuses…) | `_airbyte_extracted_at` | default | full-refresh → sync-liveness signal |

"verified" rows are backed by measured `ext_age` vs `biz_age` on live data;
"inferred" rows are anchored on the connector's cursor column (verified to
exist) but their windowed behavior is not yet confirmed against data — they are
marked inline in each `schema.yml` with `confirm once ingested`.

Re-categorizing a connector across tiers is an engineering change (it usually
comes with a `loaded_at_field` revisit), not an ops dial — that's why the
mapping lives in connector `schema.yml`, literal per source.

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

Concrete tier values are the literal `warn_after`/`error_after` in the source's
`schema.yml`. Tiers: `default` 36h/72h, `report` 72h/120h, `report_extended`
120h/168h, `event` 96h/168h.

`error` and `runtime error` flip the workflow to Failed so Argo retains the
run in `failedJobsHistoryLimit`. Warn-only runs stay Successful — the breach
is still printed to the workflow log and POSTed in the notification payload,
but on-call doesn't get paged on a single missed sync.

### How it stays generic

The CronWorkflow is connector-agnostic: it runs `dbt source freshness --select
source:*` and acts on whatever every `bronze_*` source declares. A new
connector is covered the moment its `schema.yml` carries a `freshness:` block —
no per-connector pipeline plumbing. The declaration (which column, which tier)
is the connector author's call, captured per source in `schema.yml`.

## Adding a new connector — freshness checklist

1. In the new `connectors/<category>/<name>/dbt/schema.yml`, declare the
   `bronze_*` source with a `freshness:` block + `loaded_at_field`.
2. Pick the `loaded_at_field` by sync shape:
   - **Incremental / event-cursor** (commits, issues, edits) —
     `loaded_at_field: _airbyte_extracted_at`, `default` tier (36h/72h). The
     extract timestamp tracks reality because the cursor only advances on new data.
   - **Windowed / vendor-analytics** (re-fetches a fixed reporting window every
     run — Graph reports, Slack/Cursor/OpenAI daily) —
     `loaded_at_field: parseDateTimeBestEffortOrNull(<business_date_col>)` with the
     `report` (or `report_extended` for 3–5d lag) tier. **Do not** leave it on
     `_airbyte_extracted_at` — it re-stamps every sync and goes false-green.
   - **Event-style with quiet weekends** (Confluence/Zoom — a zero-row Saturday is
     normal) — anchor on the event timestamp with the `event` tier (96h/168h):

     ```yaml
     freshness:
       warn_after:  { count: 96,  period: hour }
       error_after: { count: 168, period: hour }
     loaded_at_field: parseDateTimeBestEffortOrNull(<event_ts>)
     ```

3. Confirm the windowed-vs-incremental call against live data when the connector
   has rows: compare `now() - max(_airbyte_extracted_at)` against
   `now() - max(<business_date>)`. A large gap = windowed (anchor on the business
   date); roughly equal = incremental (`_airbyte_extracted_at` is fine).
4. Roster/lookup tables (full-refresh, rarely-changing) keep `_airbyte_extracted_at`
   on the `default` tier — the re-stamped extract time is a useful "sync alive"
   signal. Use a per-table `freshness: null` opt-out only for streams that are
   *incremental* and legitimately go quiet for days (where `_airbyte_extracted_at`
   would false-alarm).
5. Coverage is measured by the QA-owned `dbt_coverage.py` gate (EPIC #1321),
   which counts per-source declarations — which is why each source declares
   explicitly rather than inheriting a project default.

6. **Trap to avoid**: if your connector re-emits a fixed window every run
   (`SELECT count(), max(_airbyte_extracted_at) - min(_airbyte_extracted_at)
   FROM bronze_<x>.<table>` shows all rows extracted within the last 24h
   even though the table covers many days of history), `_airbyte_extracted_at`
   will look fresh forever (the false-green failure mode). Anchor on the
   business-date column instead. This is a judgment call about the source
   shape — confirm it against live data and record it in the assignment table
   above.

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
| `constructorfabric/insight` repo Issues | One issue per persistent breach (>2 consecutive runs) opened by the on-call | Within 1 business day of the breach | Hand off to the connector owner |
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
