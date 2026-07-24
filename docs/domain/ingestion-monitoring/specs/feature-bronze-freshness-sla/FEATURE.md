---
status: proposed
date: 2026-05-04
---

# Feature: Bronze Freshness SLA

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Trigger Freshness Check on Schedule](#trigger-freshness-check-on-schedule)
  - [Triage a Page-Worthy Breach](#triage-a-page-worthy-breach)
  - [Wire a New Connector into Freshness](#wire-a-new-connector-into-freshness)
  - [Switch Notification Driver in a Tenant Overlay](#switch-notification-driver-in-a-tenant-overlay)
  - [Override SLA Threshold for One Connector](#override-sla-threshold-for-one-connector)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Resolve dbt Selector to Existing Bronze Schemas](#resolve-dbt-selector-to-existing-bronze-schemas)
  - [Classify Source Status (PASS / WARN / ERROR / RUNTIME ERROR)](#classify-source-status-pass--warn--error--runtime-error)
  - [Render Notification Body per Driver](#render-notification-body-per-driver)
  - [Detect Re-Emit Trap](#detect-re-emit-trap)
  - [Choose SLA Tier per Source](#choose-sla-tier-per-source)
- [4. States (CDSL)](#4-states-cdsl)
  - [Source Freshness Status](#source-freshness-status)
  - [Trap Suspect Lifecycle](#trap-suspect-lifecycle)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Per-Source Thresholds](#per-source-thresholds)
  - [Per-Table Opt-Out](#per-table-opt-out)
  - [Workflow Execution](#workflow-execution)
  - [Notification Delivery](#notification-delivery)
  - [Trap Detection](#trap-detection)
  - [Local Verification](#local-verification)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

Daily check that every Airbyte-managed `bronze_*` source has received fresh
rows within the last warn-after window for its tier. The mechanism is a
single dbt project policy plus an Argo `CronWorkflow` — no per-connector
code changes are required for new sources to be covered.

## 1. Feature Context

### 1.1 Overview

Today the ingestion pipeline has no signal for "data is missing".
Connectors that silently stop emitting rows (expired tokens, upstream API
outage, Argo CronWorkflow failures, sync ran but produced 0 rows) only
become visible when a downstream metric goes flat days later.

This feature wires `dbt source freshness` against the
`_airbyte_extracted_at` column that every Airbyte ClickHouse destination
writes per row, or against a business-date column for report-style
connectors. A source is `pass` when `MAX(<anchor>)` is within the tier's
warn-after window, `warn` between warn-after and error-after, `error` past
error-after, and `runtime error` if the freshness query itself fails.

### 1.2 Purpose

Catch ingestion-layer breaches before they propagate to silver/gold
metrics and to the dashboard. Keep the cost of adding new connectors low
— a new connector is covered the moment it declares a `freshness:` block on
its bronze source; the CronWorkflow picks it up via `source:*` with no
per-connector plumbing.

**Requirements**: `cpt-insightspec-fr-mon-daily-cronworkflow`,
`cpt-insightspec-fr-mon-thresholds-sot`,
`cpt-insightspec-fr-mon-four-tiers`,
`cpt-insightspec-fr-mon-driver-selection`

**Principles**: `cpt-insightspec-principle-mon-trap-advisory`,
`cpt-insightspec-principle-mon-vendor-thresholds`

### 1.3 Actors

| Actor | Role in Feature |
|---|---|
| Ingestion on-call | Reads Argo workflow status; triages `error` runs |
| Connector owner | Receives a follow-up issue from on-call when their connector breaches |
| `dbt-source-freshness-check` CronWorkflow | Runs daily at 13:00 UTC, parses `target/sources.json`, fans the result out to the configured notification driver |
| Notification consumer | Receives the webhook / Zulip / Slack / Teams / SMTP payload |

### 1.4 References

- Operational runbook: [`src/ingestion/MONITORING.md`](../../../../../src/ingestion/MONITORING.md) — verification steps, on-call matrix, parser exit codes, payload shape
- Workflow template: [`charts/insight/templates/ingestion/dbt-source-freshness.yaml`](../../../../../charts/insight/templates/ingestion/dbt-source-freshness.yaml)
- Threshold config: literal `warn_after`/`error_after` per source in each connector's `dbt/schema.yml` (declaration half tracked under EPIC #1321 / #1322 — PR #1346 + the per-connector business-date anchoring in PR SharedQA/insight#1)
- PRD: [../PRD.md](../PRD.md)
- DESIGN: [../DESIGN.md](../DESIGN.md)
- Per-source declarations: every connector's `dbt/schema.yml` carries
  `loaded_at_field` at source level (dbt does not propagate this
  property from project config). Streaming connectors anchor on
  `_airbyte_extracted_at`; report-style connectors (M365 Graph reports,
  Slack admin.analytics) anchor on the report's own business-day column
  wrapped in `parseDateTimeBestEffortOrNull(...)`.

## 2. Actor Flows (CDSL)

### Trigger Freshness Check on Schedule

- [ ] `p1` - **ID**: `cpt-insightspec-flow-bronze-freshness-sla-trigger`

**Actors**:
- `dbt-source-freshness-check` CronWorkflow (Argo)
- `dbt-source-freshness` WorkflowTemplate (Argo)
- ClickHouse

**Success Scenarios**:
- Cron fires; selector narrowing finds at least one deployed `bronze_*`
  database; `dbt source freshness` writes `target/sources.json`; parser
  exits 0 (no breach) or 1 (breach) and dispatches when a driver is
  configured.

**Error Scenarios**:
- `target/sources.json` missing because dbt crashed → parser exits 2.
- ClickHouse unreachable → dbt fails; parser exits 2 because no report
  was produced.

**Steps**:
1. [ ] - `p1` - Argo evaluates `ingestion.freshness.schedule` and spawns a Workflow from the template - `inst-cron-spawn`
2. [ ] - `p1` - Workflow removes any stale `target/sources.json` (workflow template line 252) - `inst-clean-stale`
3. [ ] - `p1` - Workflow queries `system.databases` for existing `bronze_*` schemas (lines 262–297) and builds the effective selector - `inst-narrow-selector`
4. [ ] - `p1` - Workflow runs `dbt source freshness --select <effective>` (lines 305–307) - `inst-run-dbt`
5. [ ] - `p1` - Workflow invokes the inline Python parser - `inst-call-parser`
6. [ ] - `p1` - Workflow invokes the trap detector after the parser; its exit code is captured but not used to override the parser's verdict - `inst-call-trap`
7. [ ] - `p1` - Workflow exits with the parser's exit code - `inst-final-exit`

### Triage a Page-Worthy Breach

- [ ] `p1` - **ID**: `cpt-insightspec-flow-bronze-freshness-sla-triage`

**Actors**:
- Ingestion on-call
- Connector owner

**Success Scenarios**:
- On-call reads the payload, identifies the source, hands off a tracked
  issue to the connector owner; next run is green.

**Error Scenarios**:
- Payload missing `cluster` / `tenant` labels → on-call cannot route
  multi-tenant fan-out; configuration bug to fix in Helm overlay.
- Receiver outage hides the page → fall back to Argo
  `failedJobsHistoryLimit` retention to find the breach in the UI.

**Steps**:
1. [ ] - `p1` - On-call receives the notification carrying `[cluster=…, tenant=…] N bronze source(s) breaching freshness SLA` - `inst-receive-page`
2. [ ] - `p1` - On-call identifies the breaching `source` and `age_hours` from `breaches[]` - `inst-read-payload`
3. [ ] - `p1` - On-call cross-references the source's tier in DESIGN to confirm page-worthiness - `inst-check-tier`
4. [ ] - `p1` - On-call inspects last sync in Argo / Airbyte UI to distinguish "sync stopped" from "sync ran but produced no rows" - `inst-inspect-sync`
5. [ ] - `p1` - On-call hands off to the connector owner with the payload as evidence - `inst-handoff`
6. [ ] - `p1` - Connector owner fixes the connector or revisits the SLA tier in `schema.yml` - `inst-fix-connector`

### Wire a New Connector into Freshness

- [ ] `p1` - **ID**: `cpt-insightspec-flow-bronze-freshness-sla-wire`

**Actors**:
- Connector author
- CI lint

**Success Scenarios**:
- New connector declares a `freshness:` block + `loaded_at_field` at source or
  table level; the `dbt_coverage.py` gate counts it; next daily run includes it.

**Error Scenarios**:
- Author leaves a windowed connector on `_airbyte_extracted_at` → false-green
  (no breach raised while data is stale). Caught by confirming `ext_age` vs
  `biz_age` on live data, not by a lint.
- Author anchors on a non-timestamp / wrong column → dbt emits `runtime error`
  for that source (visible, paged), not a silent miss.

**Steps**:
1. [ ] - `p1` - Author adds a `sources:` entry with a literal `freshness:` block in the connector's `dbt/schema.yml` - `inst-add-source-entry`
2. [ ] - `p1` - Author selects the anchor: `_airbyte_extracted_at` for incremental, a `parseDateTimeBestEffortOrNull(<business_date>)` expression for windowed - `inst-pick-anchor`
3. [ ] - `p1` - Author sets the literal tier (`default`/`report`/`report_extended`/`event`) `warn_after`/`error_after` per source - `inst-pick-tier`
4. [ ] - `p1` - Author opens a PR; the `dbt_coverage.py` gate (EPIC #1321) confirms the source is declared - `inst-coverage-gate`
5. [ ] - `p1` - PR merges; the next daily freshness run includes the new source with no further plumbing - `inst-included-next-run`

### Switch Notification Driver in a Tenant Overlay

- [ ] `p2` - **ID**: `cpt-insightspec-flow-bronze-freshness-sla-switch-driver`

**Actors**:
- Platform engineer

**Success Scenarios**:
- Engineer flips the driver in a Helm overlay; `helm upgrade` re-renders
  the WorkflowTemplate; next run dispatches to the new channel.

**Error Scenarios**:
- New driver's `urlSecret.name` references a missing Secret → workflow
  pod fails to start; Argo controller logs surface the error; engineer
  creates the Secret and retries.

**Steps**:
1. [ ] - `p2` - Engineer creates / updates the Secret carrying the new driver's URL or SMTP password - `inst-create-secret`
2. [ ] - `p2` - Engineer sets `ingestion.freshness.notification.driver: <new>` and the driver's `urlSecret.{name,key}` (or `email.smtp.*`) in the overlay - `inst-edit-overlay`
3. [ ] - `p2` - `helm upgrade` re-renders the WorkflowTemplate; the `secretKeyRef` binding switches to the new driver's branch - `inst-helm-upgrade`
4. [ ] - `p2` - Engineer triggers an ad-hoc workflow run to confirm dispatch to the new channel - `inst-adhoc-run`

### Override SLA Threshold for One Connector

- [ ] `p2` - **ID**: `cpt-insightspec-flow-bronze-freshness-sla-override`

**Actors**:
- Connector author

**Success Scenarios**:
- Author re-tiers the connector by editing its `schema.yml`; next run
  uses the new tier's warn / error pair.

**Error Scenarios**:
- Author tries to override thresholds at project level via
  `+loaded_at_field` → silently ignored by dbt; PRD-level rationale +
  CI lint redirect them to per-source declarations.

**Steps**:
1. [ ] - `p2` - Author updates the per-source `freshness:` block in `schema.yml` to reference a different tier env var (e.g. switch from `FRESHNESS_*_DEFAULT_H` to `FRESHNESS_*_REPORT_H`) - `inst-edit-tier`
2. [ ] - `p2` - Author documents rationale in the commit message (vendor publish-lag evidence, observation window) - `inst-document-rationale`
3. [ ] - `p2` - PR merges; the next run uses the new tier - `inst-take-effect`

## 3. Processes / Business Logic (CDSL)

### Resolve dbt Selector to Existing Bronze Schemas

- [ ] `p1` - **ID**: `cpt-insightspec-algo-bronze-freshness-sla-resolve-selector`

**Input**: configured `dbt_select` Helm value (default `source:*`),
ClickHouse `system.databases` rows.

**Output**: effective dbt selector string used in
`dbt source freshness --select <effective>`.

**Steps**:
1. [ ] - `p1` - Read `dbt_select` parameter (default `source:*`) - `inst-read-select`
2. [ ] - `p1` - Query ClickHouse: `SELECT name FROM system.databases WHERE name LIKE 'bronze_%'` - `inst-query-databases`
3. [ ] - `p1` - **IF** caller supplied a more specific selector than `source:*`, pass it through verbatim - `inst-passthrough-specific`
4. [ ] - `p1` - **ELSE** narrow `source:*` to `source:<bronze_db_1> source:<bronze_db_2> ...` for each existing bronze database - `inst-narrow-list`
5. [ ] - `p1` - **RETURN** the joined selector - `inst-return-selector`

### Classify Source Status (PASS / WARN / ERROR / RUNTIME ERROR)

- [ ] `p1` - **ID**: `cpt-insightspec-algo-bronze-freshness-sla-classify`

**Input**: `target/sources.json` results array.

**Output**: list of `breaches` records `{source, status, max_loaded_at,
age_hours, empty}`; workflow exit code (0 / 1 / 2).

**Steps**:
1. [ ] - `p1` - **IF** `target/sources.json` is missing → exit 2 (workflow template line 323) - `inst-missing-report`
2. [ ] - `p1` - **FOR EACH** result in `results[]` - `inst-iter-results`
   1. [ ] - `p1` - Read `unique_id`, `status`, `max_loaded_at`, `max_loaded_at_time_ago_in_s` - `inst-read-fields`
   2. [ ] - `p1` - **IF** `max_loaded_at.startswith("1970-01-01")` → mark `empty=true`, set `max_loaded_at="(table is empty)"`, `age_hours=null` (lines 348–360) - `inst-empty-sentinel`
   3. [ ] - `p1` - **ELSE** convert `max_loaded_at_time_ago_in_s` to hours - `inst-compute-age`
   4. [ ] - `p1` - Append `{source, status, max_loaded_at, age_hours, empty}` to `breaches[]` if status != `pass` - `inst-append-breach`
3. [ ] - `p1` - **IF** any `breaches[i].status ∈ {error, runtime error}` → exit 1 (page-worthy) - `inst-decide-page`
4. [ ] - `p1` - **ELSE** exit 0 (line 559) - `inst-decide-clean`

### Render Notification Body per Driver

- [ ] `p1` - **ID**: `cpt-insightspec-algo-bronze-freshness-sla-render`

**Input**: canonical breach payload `{topic, cluster, tenant, summary,
breaches[]}`, active `driver`.

**Output**: driver-shaped HTTP body or SMTP message.

**Steps**:
1. [ ] - `p1` - Build summary prefix: include `cluster=…` and `tenant=…` only when non-empty (workflow template lines 393–397) - `inst-build-prefix`
2. [ ] - `p1` - **MATCH** driver - `inst-match-driver`
   1. [ ] - `p1` - `webhook` → JSON body verbatim, `Content-Type: application/json` - `inst-render-webhook`
   2. [ ] - `p1` - `zulip` → rewrite path `/external/json` → `/external/slack`; `application/x-www-form-urlencoded` with Slack-compat fields; Slack mrkdwn (lines 445–478) - `inst-render-zulip`
   3. [ ] - `p1` - `slack` → JSON `{text, channel?}`; GitHub-style mrkdwn (lines 480–487) - `inst-render-slack`
   4. [ ] - `p1` - `teams` → MessageCard JSON; `themeColor` red if any page-worthy, orange otherwise (lines 489–500) - `inst-render-teams`
   5. [ ] - `p1` - `email` → text/plain body; `Subject: <prefix> <summary>` (lines 502–531) - `inst-render-email`
3. [ ] - `p1` - **RETURN** rendered body / message - `inst-return-body`

### Detect Re-Emit Trap

- [ ] `p2` - **ID**: `cpt-insightspec-algo-bronze-freshness-sla-trap`

**Input**: bronze table rows (sampled), optional
`meta.bronze_business_date_col` SQL expression.

**Output**: list of trap suspects `{source, table, kind, evidence}`.

**Steps**:
1. [ ] - `p2` - **IF** `meta.bronze_freshness_trap_check == "skip"` → return [] for this source/table - `inst-honour-skip`
2. [ ] - `p2` - **MODE 1** (heuristic, no config) - `inst-mode-fullreemit`
   1. [ ] - `p2` - Compute pct of rows with `_airbyte_extracted_at` in last 30 h - `inst-pct-recent`
   2. [ ] - `p2` - Compute distinct count of `toDate(_airbyte_extracted_at)` - `inst-distinct-days`
   3. [ ] - `p2` - **IF** pct ≥ 95 % AND distinct days ≤ 2 AND row count ≥ 100 → flag `kind=full-reemit` (script lines 188–200) - `inst-flag-fullreemit`
3. [ ] - `p2` - **MODE 2** (opt-in) - `inst-mode-incremental-topup`
   1. [ ] - `p2` - **IF** source declares `meta.bronze_business_date_col` → compute `MAX(<expr>)` - `inst-compute-bdate`
   2. [ ] - `p2` - Compute `MAX(_airbyte_extracted_at)` - `inst-compute-extracted`
   3. [ ] - `p2` - **IF** gap ≥ 24 h → flag `kind=incremental-topup` (lines 202–227) - `inst-flag-incremental`
4. [ ] - `p2` - **RETURN** suspects (advisory only — does not change parser exit code) - `inst-return-suspects`

### Choose SLA Tier per Source

- [ ] `p2` - **ID**: `cpt-insightspec-algo-bronze-freshness-sla-tier`

**Input**: connector cadence (streaming / event / report /
report-extended), vendor-documented publish-lag evidence.

**Output**: per-source `freshness:` block in `schema.yml` referencing
the chosen tier's env vars.

**Steps**:
1. [ ] - `p2` - **IF** connector is streaming with daily cron and no natural quiet days → `default` (30/48 h) - `inst-tier-default`
2. [ ] - `p2` - **ELSE IF** connector has natural quiet days (Confluence edits, Zoom meetings) → `event` (72/96 h) - `inst-tier-event`
3. [ ] - `p2` - **ELSE IF** vendor documents 24–48 h publish lag (Microsoft Graph reports baseline) → `report` (48/96 h) - `inst-tier-report`
4. [ ] - `p2` - **ELSE IF** vendor publishes with ~3-day baseline lag (Slack admin.analytics typical) → `report_extended` (72/120 h) - `inst-tier-report-extended`
5. [ ] - `p2` - Encode the choice as a per-source `freshness:` block referencing the tier's env vars (`FRESHNESS_WARN_*_H` / `FRESHNESS_ERROR_*_H`) - `inst-encode-tier`

## 4. States (CDSL)

### Source Freshness Status

- [ ] `p1` - **ID**: `cpt-insightspec-state-bronze-freshness-sla-source-status`

A bronze source transitions between four classification states on each
daily run. The state is recomputed from the anchor every time — there is
no persistence between runs.

| From | Event | To |
|---|---|---|
| (any) | `MAX(<anchor>)` within warn-after | `pass` |
| `pass` | anchor ages past warn-after, before error-after | `warn` |
| `warn` | anchor ages past error-after | `error` |
| (any) | freshness query itself fails | `runtime error` |
| `error` / `warn` | new rows land within warn-after | `pass` |
| `pass` (zero rows ever) | empty-table sentinel detected | `pass` (with `empty=true` flag in payload) |

`pass` does not page; `warn` shows up in the payload but is not
page-worthy; `error` and `runtime error` page on the daily run.

### Trap Suspect Lifecycle

- [ ] `p2` - **ID**: `cpt-insightspec-state-bronze-freshness-sla-trap-suspect`

A trap suspect has a single advisory-only state per run — it is logged
but never persisted.

| From | Event | To |
|---|---|---|
| (none) | mode-1 heuristic threshold crossed | `flagged-fullreemit` (advisory) |
| (none) | mode-2 business-date divergence detected | `flagged-incremental-topup` (advisory) |
| `flagged-*` | next run no longer matches | (none) |
| (any) | `meta.bronze_freshness_trap_check: skip` | `suppressed` (no state machine entry) |

## 5. Definitions of Done

### Per-Source Thresholds

- [ ] `p1` - **ID**: `cpt-insightspec-dod-bronze-freshness-sla-thresholds`

The system **MUST** ensure every `bronze_*` source declared anywhere
under `src/ingestion/connectors/` is included in `dbt source freshness`
without per-connector wiring. Each source **MUST** declare its own
`freshness:` block + `loaded_at_field` literally in its `schema.yml`
(the `dbt_coverage.py` gate counts per-source declarations); the
CronWorkflow runs `--select source:*` and needs no project default.

**Implements**:
- `cpt-insightspec-flow-bronze-freshness-sla-wire`
- `cpt-insightspec-algo-bronze-freshness-sla-tier`

**Covers (PRD)**:
- `cpt-insightspec-fr-mon-thresholds-sot`
- `cpt-insightspec-fr-mon-four-tiers`

**Touches**:
- `src/ingestion/connectors/*/*/dbt/schema.yml`

### Per-Table Opt-Out

- [ ] `p1` - **ID**: `cpt-insightspec-dod-bronze-freshness-sla-optout`

`freshness: null` on a table **MUST** exclude it from the breach count
without removing the source itself from the report. Every opt-out
**MUST** carry a one-line rationale comment next to it, and is reserved
for *incremental* streams that legitimately go quiet for days (a
full-refresh roster/lookup keeps `_airbyte_extracted_at` as a
sync-liveness signal instead).

**Implements**:
- `cpt-insightspec-flow-bronze-freshness-sla-wire`

**Covers (PRD)**:
- `cpt-insightspec-fr-mon-optout-pertable`
- `cpt-insightspec-fr-mon-optout-rationale`

**Touches**:
- `src/ingestion/connectors/*/*/dbt/schema.yml`

### Workflow Execution

- [ ] `p1` - **ID**: `cpt-insightspec-dod-bronze-freshness-sla-workflow`

The daily run **MUST** complete within `activeDeadlineSeconds: 1200` for
the current connector set (~20 sources, ~80 tables; observed end-to-end
~2 min on a warm CH). A stale-data fixture **MUST** produce parser
exit code 1 with a log entry naming the source, max anchor, and lag in
hours. A clean run **MUST** produce parser exit code 0 and a one-line
"all sources within SLA" log.

**Implements**:
- `cpt-insightspec-flow-bronze-freshness-sla-trigger`
- `cpt-insightspec-algo-bronze-freshness-sla-resolve-selector`
- `cpt-insightspec-algo-bronze-freshness-sla-classify`

**Covers (PRD)**:
- `cpt-insightspec-fr-mon-daily-cronworkflow`
- `cpt-insightspec-fr-mon-selector-narrowing`
- `cpt-insightspec-fr-mon-stale-report-cleanup`
- `cpt-insightspec-fr-mon-exit-rederivation`
- `cpt-insightspec-nfr-mon-activation-deadline`
- `cpt-insightspec-nfr-mon-exit-codes`

**Touches**:
- `charts/insight/templates/ingestion/dbt-source-freshness.yaml`

### Notification Delivery

- [ ] `p1` - **ID**: `cpt-insightspec-dod-bronze-freshness-sla-notification`

When `notification.driver` is `""`, the workflow **MUST** succeed (or
fail on `error`) without attempting a fan-out. When set, the rendered
payload **MUST** be POSTed (or sent via SMTP). Delivery failures
**MUST** be logged but **MUST NOT** change the workflow's primary exit
code.

**Implements**:
- `cpt-insightspec-flow-bronze-freshness-sla-switch-driver`
- `cpt-insightspec-algo-bronze-freshness-sla-render`

**Covers (PRD)**:
- `cpt-insightspec-fr-mon-driver-selection`
- `cpt-insightspec-fr-mon-credential-isolation`
- `cpt-insightspec-fr-mon-dispatch-impl`
- `cpt-insightspec-fr-mon-delivery-failure-isolation`
- `cpt-insightspec-fr-mon-identity-helm`
- `cpt-insightspec-fr-mon-identity-summary`
- `cpt-insightspec-fr-mon-identity-payload`

**Touches**:
- `charts/insight/templates/ingestion/dbt-source-freshness.yaml`
- `charts/insight/values.yaml`
- `charts/insight/values.schema.json`

### Trap Detection

- [ ] `p2` - **ID**: `cpt-insightspec-dod-bronze-freshness-sla-trap`

The trap detector **MUST** run after the parser, support both heuristic
full-reemit detection and opt-in business-date divergence, honour
`meta.bronze_freshness_trap_check: skip`, and never override the
parser's exit code.

**Implements**:
- `cpt-insightspec-algo-bronze-freshness-sla-trap`

**Covers (PRD)**:
- `cpt-insightspec-fr-mon-trap-post-parser`
- `cpt-insightspec-fr-mon-trap-two-modes`
- `cpt-insightspec-fr-mon-trap-skip-annotation`
- `cpt-insightspec-fr-mon-trap-advisory`
- `cpt-insightspec-nfr-mon-trap-advisory-mode`

**Touches**:
- `src/ingestion/scripts/freshness-trap-detect.py`
- `charts/insight/templates/ingestion/dbt-source-freshness.yaml`

### Local Verification

- [ ] `p2` - **ID**: `cpt-insightspec-dod-bronze-freshness-sla-local-verify`

`dbt source freshness --select 'source:bronze_*'` **MUST** run from the
`insight-toolbox` image against `clickhouse-local` (HTTP `:8123`,
`host.docker.internal`) using the workspace dbt profile, with no
per-source environment-specific overrides.

**Touches**:
- `src/ingestion/dbt/profiles.yml`
- `src/ingestion/MONITORING.md`

## 6. Acceptance Criteria

- [ ] Every bronze source under `src/ingestion/connectors/*/*/dbt/schema.yml` declares a reachable `loaded_at_field` (source / table / opt-out).
- [ ] `dbt source freshness --select 'source:bronze_*'` from a clean checkout produces a non-empty `target/sources.json` with one result per declared (source, table) pair, no `runtime error` due to missing `loaded_at_field`.
- [ ] CronWorkflow renders cleanly when `templates.enabled=true` and `toolboxImage` is pinned (no `:latest` defaults).
- [ ] Stale-data fixture (truncate or backdate `_airbyte_extracted_at` on a single source) produces exit code 1 and the source appears in the log + payload.
- [ ] On-call matrix and webhook payload contract documented in [`MONITORING.md`](../../../../../src/ingestion/MONITORING.md).
- [ ] Synthetic re-emit fixture is flagged by the trap detector with `kind=full-reemit`; opt-in business-date divergence fixture is flagged with `kind=incremental-topup`.
- [ ] Switching `notification.driver` in a Helm overlay re-points dispatch to the new channel after `helm upgrade`, with no Secret value visible in rendered manifests.
