---
status: proposed
date: 2026-05-04
---

# PRD — Ingestion Monitoring

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
  - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
  - [3.2 Expected Scale](#32-expected-scale)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Freshness Check](#51-freshness-check)
  - [5.2 Threshold Tiers](#52-threshold-tiers)
  - [5.3 Opt-Out Hygiene](#53-opt-out-hygiene)
  - [5.4 Trap Detection](#54-trap-detection)
  - [5.5 Notification Delivery](#55-notification-delivery)
  - [5.6 Identity Labels](#56-identity-labels)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

Catch ingestion-layer breaches — bronze tables that have stopped receiving
fresh rows, or whose freshness anchor is masking a re-emit pattern — before
the silence propagates to silver/gold metrics and to the dashboard. The
domain owns observability *of what we collect*; it deliberately does not
own deployment health, volume baselines, or vendor-job attribution (see
[§4.2](#42-out-of-scope)).

### 1.2 Background / Problem Statement

The ingestion pipeline previously had no signal for "data is missing".
Connectors that silently stopped emitting rows (expired tokens, upstream
API outages, Argo CronWorkflow failures, syncs that ran but produced 0
rows) only became visible when a downstream metric went flat days later.

Two classes of silent failure motivated the domain:

1. **Sync stops, anchor goes stale.** The straightforward case — Airbyte
   connector dies, no new rows land, `MAX(_airbyte_extracted_at)` ages
   out of the warn window.
2. **Sync runs, anchor stays green, upstream has stopped publishing.**
   The trap case. Some connectors (Microsoft Graph reports, Slack
   admin.analytics, Cursor daily_usage, Confluence page versions)
   re-fetch a fixed window every run. Even when the upstream advances
   no new business days, Airbyte keeps writing rows for older days, so
   `_airbyte_extracted_at` looks fresh forever. The local CH dump on
   2026-05-04 had M365 9 hours fresh on `_airbyte_extracted_at` but
   `reportRefreshDate` 96 hours behind reality (see
   [feature-bronze-freshness-sla/FEATURE.md](feature-bronze-freshness-sla/FEATURE.md)).

### 1.3 Goals (Business Outcomes)

- Every Airbyte-managed `bronze_*` source has a verifiable
  PASS/WARN/ERROR freshness verdict produced once per day.
- Adding a new connector inherits monitoring without per-pipeline
  plumbing — a `loaded_at_field` line in the connector's `schema.yml` is
  enough.
- Operators receive breaches via a notification driver of their choice
  (webhook, Zulip, Slack, Teams, email) — without exposing the channel's
  credentials in rendered manifests or in the Argo UI.
- Connector authors who pick the wrong anchor are caught either at PR
  time (CI lint) or at runtime (trap detector), not by silent green
  dashboards.

### 1.4 Glossary

| Term | Meaning |
|---|---|
| Bronze | The first ClickHouse-side schema written by Airbyte: `bronze_<connector>.<stream>`. |
| Freshness anchor | The SQL expression in `loaded_at_field` against which `dbt source freshness` measures lag. |
| Streaming connector | Connector whose Airbyte cursor follows business time; rows land in bronze approximately when they happen. Anchor = `_airbyte_extracted_at`. |
| Report-style connector | Connector that re-fetches a fixed window every run (Microsoft Graph reports, Slack admin.analytics). Anchor = a business-date column. |
| Event-style connector | Streaming connector with legitimate quiet days (Confluence edits, Zoom meetings). Uses a wider tier. |
| SLA tier | A pair `(warn_after, error_after)` shared by connectors with similar cadence: `default`, `event`, `report`, `report_extended`. |
| Trap | A bronze table whose `_airbyte_extracted_at` looks fresh while the upstream has stopped publishing — the freshness check passes for the wrong reason. |
| Driver | A notification dispatch backend selected via `ingestion.freshness.notification.driver`: `webhook` / `zulip` / `slack` / `teams` / `email`. |

## 2. Actors

### 2.1 Human Actors

#### Ingestion On-Call

Triages freshness breaches. Reads Argo workflow status / notification
payloads. Acts on `error` and `runtime error` runs.

#### Connector Owner

Owns a specific `bronze_*` source. Receives an issue handed off by
on-call when their connector breaches. Fixes the connector or revisits
the SLA tier.

#### Tenant On-Call (post-MVP)

Per-deployment triage when one bot fans out to multiple installations.
Reads the notification payload's `cluster` / `tenant` labels. Same
triage as ingestion on-call, scoped to one deployment.

Rotation / `CODEOWNERS` for `src/ingestion/connectors/` is **not yet
assigned** — see
[`src/ingestion/MONITORING.md` §"Rotation / ownership — not assigned"](../../../../src/ingestion/MONITORING.md).

### 2.2 System Actors

#### `dbt-source-freshness` WorkflowTemplate (Argo)

Stateless template — runs `dbt source freshness`, parses
`target/sources.json`, dispatches to the active driver. Defined in
[`charts/insight/templates/ingestion/dbt-source-freshness.yaml`](../../../../charts/insight/templates/ingestion/dbt-source-freshness.yaml).

#### `dbt-source-freshness-check` CronWorkflow (Argo)

Schedules the template once a day at 13:00 UTC; carries the
per-deployment Helm config into the template's parameters. Same file.

#### Inline freshness parser (Python)

Reads `target/sources.json`, classifies breaches, calls the driver
dispatcher, sets the workflow exit code. Inline in the workflow
template, lines 317–562.

#### Trap detector

[`src/ingestion/scripts/freshness-trap-detect.py`](../../../../src/ingestion/scripts/freshness-trap-detect.py)
— advisory script that spots full-reemit and incremental-topup patterns
the dbt freshness check cannot see. Runs after the parser.

#### CI lint

[`src/ingestion/scripts/lint-bronze-freshness.py`](../../../../src/ingestion/scripts/lint-bronze-freshness.py)
— fails any PR that introduces a `bronze_*` source without a reachable
`loaded_at_field`, or a `freshness: null` opt-out without
`meta.freshness_optout_reason`.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Production: Argo Workflows + ClickHouse in a Kubernetes cluster; the
  `dbt-source-freshness-check` CronWorkflow runs once per day at 13:00
  UTC, sitting past every connector's sync window (02:00–11:00 UTC) plus
  a 2 h grace.
- Local development: Kind K8s cluster (`insight`) with the same Helm
  values; ClickHouse reachable on `host.docker.internal:8123` for the
  trap detector.
- The `insight-toolbox` image must carry `dbt`, `dbt-clickhouse`, and
  the freshness scripts (`freshness-trap-detect.py`,
  `lint-bronze-freshness.py`).
- Notification driver credentials must live in Kubernetes Secrets
  referenced by `secretKeyRef`; the rendered `WorkflowTemplate` /
  `CronWorkflow` YAML must never carry the raw URL or password.

### 3.2 Expected Scale

| Dimension | Current | Projected |
|---|---|---|
| Bronze sources monitored | 12 (one per declared `bronze_*` schema) | 50+ |
| Bronze tables monitored | ~80 (82 entries across 12 sources today) | 200+ |
| Notification drivers supported | 5 | 5 (stable) |
| SLA tiers | 4 | 4 (stable) |
| Workflow runtime (warm CH) | ~2 min | < 5 min |
| `activeDeadlineSeconds` budget | 1200 s | 1200 s |

## 4. Scope

### 4.1 In Scope

- **Bronze freshness SLA** — the `dbt source freshness` check against
  `_airbyte_extracted_at` (streaming) or a business-date column
  (report/event), with four threshold tiers (`default` 30/48 h, `event`
  72/96 h, `report` 48/96 h, `report_extended` 72/120 h). Source:
  [`feature-bronze-freshness-sla/FEATURE.md`](feature-bronze-freshness-sla/FEATURE.md).
- **CI lint of connector schemas** — every `bronze_*` source must
  declare a reachable `loaded_at_field` (source-level, per-table, or
  explicit `freshness: null` with `meta.freshness_optout_reason`).
  Source: [`lint-bronze-freshness.py`](../../../../src/ingestion/scripts/lint-bronze-freshness.py).
- **Runtime trap detector** — heuristic full-reemit detection (no
  config) plus opt-in `meta.bronze_business_date_col` divergence check;
  advisory only, never affects workflow exit code. Source:
  [`freshness-trap-detect.py`](../../../../src/ingestion/scripts/freshness-trap-detect.py).
- **Multi-driver notification fan-out** — five drivers
  (webhook / zulip / slack / teams / email) selectable via Helm; URL /
  SMTP password sourced from a Kubernetes Secret so the credential
  never appears in rendered manifests or Argo UI. Source: workflow
  template lines 161–194 and 412–540.
- **Identity labels in payload** — `cluster` and `tenant` carried into
  every payload's summary prefix and JSON body so a shared bot can
  route between installations.

### 4.2 Out of Scope

- **Deployment health** — "is a connector that should be deployed
  actually deployed?" is a separate concern (Helm probes / sync
  workflow status). Tracked as
  [issue #272](https://github.com/cyberfabric/insight/issues/272). The
  freshness workflow narrows its selector to bronze databases that
  exist (workflow template lines 254–297) precisely so it does *not*
  alert on connectors a tenant hasn't deployed.
- **Volume baseline anomaly detection** — "API returned 50 rows
  instead of 5000". Listed under "Open work" in
  [`MONITORING.md`](../../../../src/ingestion/MONITORING.md).
- **Source-vs-bronze attribution** — distinguishing "Airbyte sync
  failed" from "Airbyte sync ran but pulled 0 rows" requires polling
  Airbyte's Jobs API; listed under "Open work" in
  [`MONITORING.md`](../../../../src/ingestion/MONITORING.md).
- **Notification rotation / `CODEOWNERS`** — see
  [`MONITORING.md` §"Rotation / ownership — not assigned"](../../../../src/ingestion/MONITORING.md).

## 5. Functional Requirements

### 5.1 Freshness Check

#### Daily Argo CronWorkflow

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-daily-cronworkflow`

The system **MUST** schedule a single Argo `CronWorkflow`
(`dbt-source-freshness-check`) that runs once per day on the schedule
supplied by `ingestion.freshness.schedule` (default `0 13 * * *`,
charts/insight/values.yaml:151).

#### Selector Narrowing

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-selector-narrowing`

The workflow **MUST** narrow the dbt selector from `source:*` to the
`bronze_*` databases that actually exist in `system.databases` (workflow
template lines 262–297) — connectors a tenant did not deploy never
produce a phantom ERROR.

#### Stale Report Cleanup

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-stale-report-cleanup`

The workflow **MUST** remove a stale `target/sources.json` before each
run (workflow template line 252) so a mid-run dbt crash maps to exit
code 2 rather than misclassifying using leftover data.

#### Exit Code Re-Derivation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-exit-rederivation`

`dbt source freshness` is invoked with `--select <effective>` and its
exit code **MUST** be swallowed (line 305–307). The Python parser
re-derives the workflow outcome from `target/sources.json`.

### 5.2 Threshold Tiers

#### Single Source of Truth

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-thresholds-sot`

All thresholds **MUST** be sourced once from
`ingestion.freshness.thresholds.*`
(charts/insight/values.yaml:159–183), passed as `WorkflowTemplate`
parameters, exported as `FRESHNESS_*_H` env vars (workflow template
lines 197–212), and read by dbt via
`env_var('FRESHNESS_WARN_DEFAULT_H', '30')` in
[`src/ingestion/dbt/dbt_project.yml`](../../../../src/ingestion/dbt/dbt_project.yml)
and per-connector `schema.yml`.

#### Four Tiers

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-four-tiers`

The system **MUST** support exactly four tiers, each with documented
rationale:

- `default` (30/48 h) — streaming connectors with daily cron cadence.
- `event` (72/96 h) — natural-quiet-day connectors (Confluence
  edits, Zoom meetings).
- `report` (48/96 h) — vendor analytics with documented 24–48 h
  publish lag (Microsoft Graph reports baseline).
- `report_extended` (72/120 h) — vendor analytics with ~3-day
  baseline lag (Slack admin.analytics typical).

#### Tier Assignment

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-tier-assignment`

Tier assignment **MUST** be per-connector and live in the connector's
`schema.yml`. Re-tiering is an engineering change; Helm tunes the
*values* of each tier, not which source falls in which tier.

### 5.3 Opt-Out Hygiene

#### Per-Table Opt-Out

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-optout-pertable`

Slow-moving roster / catalog tables (e.g. `cursor_members`,
`wiki_spaces`) **MUST** be allowed to opt out via `freshness: null`
per-table in `schema.yml`.

#### Mandatory Rationale

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-optout-rationale`

The CI lint
([`lint-bronze-freshness.py`](../../../../src/ingestion/scripts/lint-bronze-freshness.py))
**MUST** reject any opt-out lacking
`meta.freshness_optout_reason: "<rationale>"` so the audit surface
stays grep-able.

#### Mandatory Anchor

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-mandatory-anchor`

The lint **MUST** reject any `bronze_*` source with no reachable
`loaded_at_field` (source-level, per-table, or explicit opt-out).

### 5.4 Trap Detection

#### Post-Parser Execution

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-trap-post-parser`

After the freshness parser exits, the workflow **MUST** run
[`freshness-trap-detect.py`](../../../../src/ingestion/scripts/freshness-trap-detect.py)
(workflow template lines 564–579).

#### Two Detection Modes

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-trap-two-modes`

The detector **MUST** support two modes:

- **Full re-emit (heuristic, no config)** — fires when ≥ 95 % of
  rows have `_airbyte_extracted_at` within the last 30 h *and*
  `_airbyte_extracted_at` covers ≤ 2 distinct calendar days *and*
  the table has ≥ 100 rows (script constants
  `SUSPECT_PCT_RECENT`, `SUSPECT_MAX_DISTINCT_DAYS`,
  `RECENT_WINDOW_HOURS`, `MIN_ROWS`).
- **Business-date divergence (opt-in)** — sources that declare
  `meta.bronze_business_date_col: <SQL expr>` get
  `MAX(<expr>)` compared with `MAX(_airbyte_extracted_at)`; a
  ≥ 24 h gap is flagged.

#### Skip Annotation

- [ ] `p2` - **ID**: `cpt-insightspec-fr-mon-trap-skip-annotation`

A source **MUST** be allowed to opt out of mode 1 via
`meta.bronze_freshness_trap_check: skip` at source or table level.

#### Advisory-Only Verdict

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-trap-advisory`

Findings **MUST** be advisory: `trap_exit=1` only logs a notice. The
workflow's page/no-page decision remains owned by the freshness
parser (workflow template lines 575–579).

### 5.5 Notification Delivery

#### Driver Selection

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-driver-selection`

`ingestion.freshness.notification.driver` **MUST** select one of `""`,
`webhook`, `zulip`, `slack`, `teams`, `email`
(charts/insight/values.yaml:220, schema enum at
charts/insight/values.schema.json:37).

#### Credential Isolation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-credential-isolation`

Each URL-driven driver **MUST** read its credential from a
`secretKeyRef` branched in the workflow template (lines 161–194). The
URL is bound to `NOTIFICATION_URL` only when the driver is configured;
email's SMTP password binds to `NOTIFICATION_EMAIL_SMTP_PASSWORD`.
Plain settings (Zulip stream, Slack channel, SMTP host/port) flow
through normal parameters because they are not credentials.

#### Dispatch Implementation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-dispatch-impl`

Driver dispatch **MUST** happen in the inline parser via the
`dispatchers` dict (workflow template lines 533–539).

#### Delivery Failure Isolation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-delivery-failure-isolation`

Notification failures **MUST** be caught and logged (lines 550–554) —
they never change the workflow's primary exit code (the breach signal
is more important than the delivery success).

### 5.6 Identity Labels

#### Helm Plumbing

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-identity-helm`

`ingestion.freshness.cluster` and `.tenant` (values.yaml:193–194)
**MUST** flow into the parser as `CLUSTER` / `TENANT` env vars.

#### Summary Prefix

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-identity-summary`

The summary line **MUST** carry every set label as a
`[cluster=…, tenant=…] ` prefix (workflow template lines 393–397);
empty labels drop out so single-deployment installs see plain
`N bronze source(s) breaching ...`.

#### Payload Routing Fields

- [ ] `p1` - **ID**: `cpt-insightspec-fr-mon-identity-payload`

The webhook payload **MUST** include both `cluster` and `tenant` as
raw fields so receivers can route by either dimension.

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Idempotency

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-mon-idempotency`

Re-running the workflow on the same data **MUST** produce the same exit
code and the same payload (no stateful side-effects,
`target/sources.json` is purged before each run).

#### Daily Cadence

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-mon-daily-cadence`

The default cadence **MUST** be daily; the cron schedule is Helm-tunable
but the parser is stateless and can run ad-hoc.

#### Credential Isolation

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-mon-credential-isolation`

Webhook URL / SMTP password **MUST** be pulled from k8s Secrets via
`secretKeyRef`; the rendered `WorkflowTemplate` / `CronWorkflow` YAML
and the Argo UI never see the raw value.

#### Deterministic Exit Codes

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-mon-exit-codes`

The parser **MUST** emit deterministic exit codes: `0` no breach or
warn-only; `1` at least one `error` / `runtime error`; `2`
`target/sources.json` missing (dbt crashed).

#### CI-Gated Schema

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-mon-ci-gated`

`lint-bronze-freshness.py` **MUST** run on every PR touching
`src/ingestion/connectors/*/dbt/schema.yml`.

#### Advisory-Fail Mode for Traps

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-mon-trap-advisory-mode`

Trap detector findings **MUST NOT** override the freshness parser's
verdict (page-worthy stays page-worthy; warn-only stays warn-only).

#### Activation Deadline

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-mon-activation-deadline`

The workflow **MUST** carry `activeDeadlineSeconds: 1200` (workflow
template line 110); current connector set finishes in ~2 min on a
warm CH.

#### Empty-Table Sentinel

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-mon-empty-sentinel`

A source with zero rows yields `MAX(...) = NULL → 1970-01-01T…`; the
parser **MUST** flag it explicitly as `(table is empty — no rows ever
ingested)` rather than reporting ~500 000 h of lag (workflow template
lines 348–360).

### 6.2 NFR Exclusions

- ClickHouse cluster performance tuning (infrastructure concern).
- Argo Workflows controller HA (infrastructure concern).
- Notification receiver SLAs (Zulip / Slack / Teams uptime is out of
  this domain's control).
- Vendor-side publish-lag changes (Microsoft Graph / Slack
  admin.analytics window changes are tracked in `MONITORING.md`, not
  enforced as NFR thresholds here).

## 7. Public Library Interfaces

### 7.1 Public API Surface

Not applicable — this domain has no public library API. The monitoring
surface is consumed via Helm values + Argo UI + dbt, not via a Rust /
Python / TypeScript library exposed to other domains. See
[DESIGN §3.3](DESIGN.md#33-api-contracts) for the workflow / dbt /
notification protocol contracts.

### 7.2 External Integration Contracts

#### Webhook Driver Contract

Generic JSON POST to `NOTIFICATION_URL`; canonical payload shape is
documented in [DESIGN §3.4 / §3.6](DESIGN.md#34-driver-contracts).
Carries `topic`, `cluster`, `tenant`, `summary`, `breaches[]`.

#### Zulip Driver Contract

POST to a Zulip incoming-webhook URL. The dispatcher rewrites
`/api/v1/external/json` to `/api/v1/external/slack` so operators who
pasted the JSON URL still get formatted output. Form-urlencoded body
with Slack mrkdwn. See [DESIGN §3.4.2](DESIGN.md#342-zulip).

#### Slack Driver Contract

POST to a Slack incoming-webhook URL with `application/json`
`{"text": "<markdown>"}`, optionally with `channel` override. See
[DESIGN §3.4.3](DESIGN.md#343-slack).

#### Teams Driver Contract

POST to a Microsoft Teams incoming webhook with `application/json`
`MessageCard`. `themeColor` is `FF0000` (red) when any breach is
page-worthy, `FFA500` (orange) otherwise. See
[DESIGN §3.4.4](DESIGN.md#344-teams).

#### Email Driver Contract

`smtplib`-based dispatch (no extra deps). `SMTP_SSL` when port=465,
otherwise plain `SMTP` with optional `STARTTLS`. Required Helm values:
`notification.email.smtp.host`, `notification.email.from`,
`notification.email.to`. See [DESIGN §3.4.5](DESIGN.md#345-email).

## 8. Use Cases

#### Use Case 1: Daily Freshness Check

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-mon-daily-check`

**Actors**: `dbt-source-freshness-check` CronWorkflow,
`dbt-source-freshness` WorkflowTemplate, ClickHouse, ingestion on-call.

**Preconditions**: Helm values `ingestion.freshness.enabled: true`,
the `bronze_*` databases exist in `system.databases`, the dbt project
declares `loaded_at_field` for every source.

**Main Flow**:

1. Argo triggers `dbt-source-freshness-check` at the configured cron.
2. The workflow narrows `source:*` to bronze databases that exist.
3. `dbt source freshness` runs against ClickHouse and writes
   `target/sources.json`.
4. The inline Python parser classifies every source as PASS / WARN /
   ERROR / RUNTIME ERROR.
5. The trap detector runs and logs any heuristic findings without
   changing the verdict.
6. The driver dispatcher posts to the configured channel whenever the
   `breaches` list is non-empty — including `warn`-only runs, so
   on-call sees latency creeping up before it crosses into
   page-worthy territory.
7. The workflow exits with `1` only when at least one breach is
   `error` or `runtime error`; `warn`-only runs exit `0` so the
   payload is informational, not paging.

**Postconditions**: A canonical breach record exists in the workflow
log; if a driver is configured and any breach was page-worthy, the
external sink received the payload.

#### Use Case 2: Triage a Paging Breach

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-mon-triage-breach`

**Actors**: ingestion on-call, connector owner.

**Preconditions**: a notification carrying `[cluster=…, tenant=…] N
bronze source(s) breaching freshness SLA` has fired.

**Main Flow**:

1. On-call opens the payload, identifies the breaching `source` and
   `age_hours`.
2. On-call cross-references the source's tier in
   [DESIGN §3.5](DESIGN.md#35-sla-tier-mapping) to confirm it was
   page-worthy (not warn-only).
3. On-call inspects the connector's last sync in Argo / Airbyte UI to
   distinguish "sync stopped" from "sync ran but produced no rows".
4. On-call hands off to the connector owner with the payload as
   evidence.
5. Connector owner fixes the connector or revisits the SLA tier in
   `schema.yml`.

**Postconditions**: Either the connector is healthy on the next daily
run, or the tier has been changed (with rationale in commit message).

#### Use Case 3: Add a New Bronze Source

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-mon-add-bronze-source`

**Actors**: connector author, CI lint.

**Preconditions**: a new connector under
`src/ingestion/connectors/<class>/<source>/` with a `dbt/schema.yml`.

**Main Flow**:

1. Author declares `loaded_at_field` at source or table level
   (`_airbyte_extracted_at` for streaming, a business-date expression
   for report-style).
2. Author optionally selects a tier by referencing
   `env_var('FRESHNESS_WARN_*_H')` in a per-source `freshness:` block;
   the default tier applies otherwise.
3. Author opens a PR. The CI lint asserts that every `bronze_*` source
   has a reachable anchor and that any `freshness: null` opt-out
   carries `meta.freshness_optout_reason`.
4. PR merges; the next daily freshness run includes the new source.

**Postconditions**: The new source is monitored on the same SLA tier
as comparable connectors with no further plumbing.

#### Use Case 4: Switch Notification Driver in a Tenant Overlay

- [ ] `p2` - **ID**: `cpt-insightspec-usecase-mon-switch-driver`

**Actors**: platform engineer.

**Preconditions**: a Helm values overlay for the target tenant; a
Kubernetes Secret carrying the new driver's URL or SMTP password.

**Main Flow**:

1. Engineer creates / updates the Secret in the same namespace as the
   workflow.
2. Engineer sets `ingestion.freshness.notification.driver: <new>` and
   the driver's `urlSecret.{name,key}` (or `email.smtp.*`) in the
   overlay.
3. `helm upgrade` re-renders the `WorkflowTemplate`; the `secretKeyRef`
   binding switches to the new driver's branch.
4. The next scheduled run dispatches to the new channel; the previous
   driver's secret reference is no longer rendered.

**Postconditions**: The tenant's freshness payload now arrives in the
new channel; rendered manifests and Argo UI carry the `secretKeyRef`,
not the credential.

## 9. Acceptance Criteria

- [ ] Every `bronze_*` source under `src/ingestion/connectors/*/*/dbt/schema.yml` has a reachable `loaded_at_field` (source / table / opt-out) — verified by `python3 src/ingestion/scripts/lint-bronze-freshness.py` exiting 0.
- [ ] Every `freshness: null` opt-out carries `meta.freshness_optout_reason` — same lint enforces this.
- [ ] `dbt source freshness --select 'source:bronze_*'` from a clean checkout produces `target/sources.json` with one result per declared (source, table) pair, no `runtime error` due to missing `loaded_at_field`.
- [ ] Each tier produces the expected verdict on its representative connectors:
  - `default` — Bitbucket, Jira, BambooHR, Cursor (non-daily), GitHub, OpenAI, Claude (PASS within 30 h of last sync).
  - `event` — Confluence `wiki_page_versions`, Zoom `meetings`/`participants` (PASS within 72 h, weekend-tolerant).
  - `report` — M365 `*_activity` (PASS within 48 h of `reportRefreshDate`).
  - `report_extended` — Slack `users_details` (PASS within 72 h of `date`).
- [ ] A stale-data fixture (truncate or backdate `_airbyte_extracted_at` on one source) produces parser exit code 1, a log line naming the source / `max_loaded_at` / `age_hours`, and a notification payload (when a driver is configured).
- [ ] The notification payload always carries both `cluster` and `tenant` keys (empty string when unset), matching the canonical webhook contract documented in [`MONITORING.md`](../../../../src/ingestion/MONITORING.md).
- [ ] The configured driver's credential **does not appear** in `kubectl get workflowtemplate dbt-source-freshness -o yaml` or in `argo get` output — only the `secretKeyRef` does.
- [ ] An empty `bronze_*` table is reported as `(table is empty)` instead of `~500000h` lag (workflow template lines 348–360).
- [ ] Synthetic Confluence-style fixture (full table re-emit within 24 h, 1 distinct extract day, ≥ 100 rows) is flagged by the trap detector with `kind=full-reemit` (script lines 188–200).
- [ ] Synthetic incremental-topup fixture (`MAX(_airbyte_extracted_at)` fresh, `MAX(<bronze_business_date_col>)` ≥ 24 h behind) is flagged with `kind=incremental-topup` (script lines 202–227).
- [ ] `values.schema.json` rejects an unknown driver name (only `""`, `webhook`, `zulip`, `slack`, `teams`, `email` are accepted — schema line 37).

## 10. Dependencies

- **Argo Workflows** controller in the `argo` namespace — provides
  `WorkflowTemplate` / `CronWorkflow` reconciliation and the
  `failedJobsHistoryLimit` retention used as the no-fan-out fallback.
- **dbt-clickhouse** adapter inside the `insight-toolbox` image —
  provides `dbt source freshness` and `target/sources.json`. The
  adapter does not support metadata-based freshness, so a missing
  `loaded_at_field` produces `runtime error` rather than silent skip.
- **ClickHouse** — readable `system.databases` (for selector
  narrowing) and HTTP interface on port 8123 (used by the trap
  detector).
- **Kubernetes Secret access** — `secretKeyRef` for both
  `clickhouse.passwordSecret` and the active driver's credential
  Secret.
- **Helm umbrella chart** — `ingestion.freshness.*` block in
  `charts/insight/values.yaml`, validated by
  `charts/insight/values.schema.json`.

## 11. Assumptions

- Every Airbyte-managed `bronze_*` source carries a usable
  `_airbyte_extracted_at` column or a business-date column suitable as
  a freshness anchor.
- Vendor publish-lag windows (Microsoft Graph 24–48 h; Slack
  admin.analytics ~3 d) remain within the configured tier bounds; if a
  vendor changes its cadence, the affected source is re-tiered via a
  schema.yml edit, not by widening the global tier.
- Operators have access to a Kubernetes Secret store and accept
  `secretKeyRef` as the credential carrier — no in-tree plaintext
  fallback is provided.
- Argo Workflows is healthy enough to schedule the daily run; if the
  controller is down, monitoring goes silent (tracked under
  "Open work" in `MONITORING.md`).

## 12. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Trap heuristic false positive (full-reemit mode 1) on a legitimate burst-load source | On-call sees a noisy advisory line | Findings are advisory only; opt out via `meta.bronze_freshness_trap_check: skip`. |
| Driver receiver outage hides real breaches | Page-worthy breach silently dropped | Notification failure does not change the workflow's exit code; Argo's `failedJobsHistoryLimit` keeps the breach visible in the UI. |
| Connector author picks the wrong anchor (uses `_airbyte_extracted_at` for a report-style source) | Trap-class silent green | CI lint requires an anchor; trap detector flags re-emit at runtime; PRD §1.2 documents the failure mode. |
| Vendor changes its publish window | Tier bounds become wrong; pages too eagerly or too late | Re-tiering is a schema.yml edit; tier values are Helm-tunable as a safety valve. |
| Multi-tenant fan-out from one bot mixes installations | On-call cannot route | `cluster` / `tenant` labels are mandatory in payload; receivers route by either dimension. |
