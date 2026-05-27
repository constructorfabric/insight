# PRD — Claude Team Connector

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
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Team Member Roster Collection](#51-team-member-roster-collection)
  - [5.2 Pending Invite Collection](#52-pending-invite-collection)
  - [5.3 Per-Seat Spend Collection](#53-per-seat-spend-collection)
  - [5.4 Per-User Code-Metrics Collection](#54-per-user-code-metrics-collection)
  - [5.5 Cloudflare Bot-Management Bypass](#55-cloudflare-bot-management-bypass)
  - [5.6 Connector Operations and Data Integrity](#56-connector-operations-and-data-integrity)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [UC-001 Deploy claude-team-proxy](#uc-001-deploy-claude-team-proxy)
  - [UC-002 Configure and Run First Sync](#uc-002-configure-and-run-first-sync)
  - [UC-003 Rotate Expired sessionKey](#uc-003-rotate-expired-sessionkey)
  - [UC-004 Operator Lacks billing:view Permission](#uc-004-operator-lacks-billingview-permission)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Claude Team Connector extracts collaboration and AI-usage data from a `claude.ai` Team-plan
organisation into the Insight platform's Bronze layer. It is a two-component system: a
**claude-team-proxy** service that holds a browser session with `claude.ai` and exposes
that session as a plain HTTP API, and an **Airbyte declarative connector** that reads from
the proxy and writes to Bronze.

The connector ships four streams: the organisation roster, pending invites, per-seat overage
spend, and per-user-per-day Claude Code usage metrics. Together they cover identity, billing,
and developer-productivity signals from the `claude.ai` Team plan.

This connector is a **strict sibling** of the existing `claude-admin` and `claude-enterprise`
connectors. All three deliver overlapping data into the `bronze_claude_*` schemas:

| Connector | Plan | Auth | Surface |
|-----------|------|------|---------|
| `claude-admin` | Pro / Team / Enterprise with Admin API access | `sk-ant-admin-*` API key | `api.anthropic.com/v1/organizations/*` |
| `claude-enterprise` | Enterprise (Analytics API entitlement) | `sk-ant-analytics-*` API key | `api.anthropic.com/v1/organizations/analytics/*` |
| `claude-team` (this) | Team plan (no Admin/Analytics API entitlement) | Web `sessionKey` cookie | `claude.ai/api/*` (web app's internal API) |

### 1.2 Background / Problem Statement

The `claude.ai` Team plan does **not** provide programmatic access to the Anthropic Admin or
Analytics APIs — those entitlements ship only with the Enterprise plan. Team-plan administrators
can see usage analytics in the `claude.ai` web UI (Settings → Analytics) but have no scriptable
API surface to pull the same data into a data warehouse.

The web UI is rendered by a React frontend that calls a private `claude.ai/api/*` endpoint set.
These endpoints are reachable with the user's session cookie (`sessionKey`) but are **fronted by
Cloudflare bot management**. A plain HTTPS client (curl, Python `requests`, Airbyte's built-in
HTTP requester) is rejected with `HTTP 403 cf-mitigated: challenge` before the request reaches
the application server. Cloudflare requires that the client execute the JS challenge served on
first contact, then carry the resulting `__cf_bm` clearance cookie on all subsequent requests.

The two implications drive the architecture:

1. **Insight must hold a `claude.ai` session cookie** (no static API key exists for this plan).
2. **A real browser must mediate the request flow** to satisfy Cloudflare's bot-management JS
   challenge. Direct HTTP fails by default.

The `claude-team-proxy` service solves both problems: a headless Chromium with `puppeteer-extra-
plugin-stealth` runs inside the cluster, holds the `sessionKey` cookie, navigates `claude.ai`
once at startup (clearing the CF challenge), then forwards `GET /api/*` requests from the
connector through the cleared browser session via `page.evaluate(fetch(...))`.

The split — proxy holds credentials and CF state, connector holds Airbyte plumbing — also
serves a defence-in-depth purpose: the connector is **credential-less**, so connector code and
descriptor have no value to an attacker; `sessionKey` rotation is a single Helm-release update,
not a connector-secret update.

**Target Users**:

- Platform operators who deploy the proxy + connector pair, hold the `sessionKey` (or have
  access to an admin user who can extract one), and rotate it on expiry.
- Data analysts who consume Team-plan member rosters, spend, and Claude Code usage in Silver
  and Gold layers.
- Security stakeholders who require credential separation between Insight components.

**Key Problems Solved**:

- Bringing `claude.ai` Team-plan analytics into Insight without an Enterprise upgrade.
- Surviving Cloudflare's bot-management JS challenge with a maintained, well-known toolchain
  (Playwright + stealth) rather than ad-hoc HTTP-spoofing tricks.
- Concentrating `claude.ai` credentials in one rotatable Helm release rather than spreading
  them across connector secrets.

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- Insight receives a refreshed Team-plan member roster at least once per scheduled run
  (Baseline: not collected; Target: v1.0).
- Insight receives per-user-per-day Claude Code metrics with at most a 24-hour collection lag
  (Baseline: not collected; Target: v1.0).
- The connector exposes zero credentials to its own descriptor or Bronze records — all auth
  state lives in the proxy's Helm-managed Secret.
- A `sessionKey` rotation completes in under five minutes of operator wall-clock time without
  redeploying the connector or restarting any sync.

**Capabilities**:

- Pull the organisation roster (`claude_team_members`) on every sync — full snapshot, no
  pagination, no cursor.
- Pull pending invites (`claude_team_invites`) on every sync — full snapshot, no pagination.
- Pull per-seat overage spend (`claude_team_overage_spend`) on every sync — page-paginated
  snapshot; gracefully tolerates `HTTP 403` when the operator's session lacks the
  `billing:view` permission.
- Pull per-user-per-day Claude Code metrics (`claude_team_code_metrics`) over a configurable
  date window — incremental by day, offset-paginated within day.
- Stamp every Bronze row with `tenant_id`, `source_id`, `unique_key`, `collected_at`, and
  `data_source: insight_claude_team`. `unique_key` is a per-row composite of
  `tenant_id`-`source_id`-`<natural-key>` (e.g. `account.uuid` for members) — the universal
  join scope across all connectors.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| `claude.ai` Team plan | Anthropic's mid-tier subscription for small teams. Provides web-UI analytics but no Admin/Analytics API. |
| Web API | The private set of HTTPS endpoints under `https://claude.ai/api/*` that the `claude.ai` frontend calls. Not officially documented or stable. |
| `sessionKey` | The HttpOnly cookie issued by `claude.ai` on user login. Acts as the bearer credential for all `/api/*` calls. Lifetime varies (observed: days–weeks); invalidated on logout, password change, or session sweep. |
| `__cf_bm` | Cloudflare Bot Management clearance cookie. Set by Cloudflare after a successful JS challenge. Lifetime ~30 minutes; auto-refreshed by the browser on use. |
| CF challenge | The JS-execution test Cloudflare serves on first contact to verify the client is a real browser. Returns HTML page with title "Just a moment..." until cleared. |
| Headless Chromium | Browser engine running without a graphical display. Used here to execute the CF challenge in a real V8 runtime so subsequent fetches are accepted. |
| Stealth plugin | `puppeteer-extra-plugin-stealth` — a collection of JS patches that hide common automation tells (`navigator.webdriver`, missing `chrome` object, fingerprint anomalies). |
| Proxy | The `claude-team-proxy` service: long-running pod that runs headless Chromium + Express-style HTTP server. Exposes `GET /api/*` and `GET /health` to in-cluster clients. |
| `claude_org_id` | UUID of the `claude.ai` organisation. One per Team plan. Provided to the connector via K8s Secret (separate from `sessionKey`). |
| Backfill anchor | The earliest date the `claude_team_code_metrics` stream walks back to. Defaults to 7 days back; operator can override via `start_date` config. Hard floor `2025-11-24` (earliest observed data for the reference organisation). |

## 2. Actors

### 2.1 Human Actors

- **Platform Operator** — deploys the proxy Helm release, manages the `sessionKey` secret,
  applies the connector's K8s Secret with `claude_org_id`, triggers reconcile, monitors syncs,
  rotates credentials on expiry.
- **`claude.ai` Organisation Owner** — has admin access on `claude.ai` and can extract their
  own `sessionKey` from a browser DevTools session. Typically the same human as Platform
  Operator in single-org deployments; may be different (operator asks owner to refresh the
  token periodically) in segregated-duty deployments.
- **Data Analyst / Downstream Consumer** — reads from `bronze_claude_team.*` (or its Silver /
  Gold descendants) for membership audits, productivity dashboards, spend analysis.

### 2.2 System Actors

- **`claude.ai` web stack** — Cloudflare edge + Anthropic application servers. Source of truth
  for all four streams.
- **`claude-team-proxy` pod** — long-running headless Chromium + HTTP server. Receives
  `GET /api/*` from the connector, executes the same path in-page, returns the upstream
  response verbatim.
- **Airbyte server / worker / temporal** — registers the connector's declarative manifest as a
  source definition, schedules syncs, runs the manifest interpreter, ships records to the
  ClickHouse destination.
- **ClickHouse destination** — writes records to `bronze_claude_team.<stream>` tables.

## 3. Operational Concept & Environment

The connector runs as a regular Airbyte declarative source in the same workspace as every
other Insight connector. The proxy is a separate long-running Deployment in the `insight`
namespace, addressable by in-cluster DNS at `claude-team-proxy.insight.svc.cluster.local:3000`.

Two K8s Secrets exist:

| Secret | Owner | Contents | Rotation trigger |
|--------|-------|----------|------------------|
| `claude-team-proxy` (Helm-managed) | proxy Helm release | `SESSION_KEY` | Cookie expires / user logs out |
| `insight-claude-team-main` (operator-applied) | Insight Airbyte reconcile | `claude_org_id` | Organisation UUID change (rare) |

The connector never reads the proxy's Secret. The proxy never reads the connector's Secret. Each
component holds only what it needs.

### 3.1 Module-Specific Environment Constraints

- **Single-replica proxy.** The browser session is per-pod state — running >1 replica would
  multiply the CF clearance ceremony and the seat usage (each browser counts as a session in
  `claude.ai`'s eyes). Use a single replica with `strategy.maxUnavailable: 0, maxSurge: 1` so
  rolling updates create a new pod, wait for it to be ready, then drop the old one.
- **Network egress to `https://claude.ai`** must be reachable from the proxy pod. The
  connector pod has no egress requirement beyond the proxy Service DNS.
- **Cluster DNS** must resolve `claude-team-proxy.insight.svc.cluster.local`. The connector's
  default `proxy_url` assumes the proxy lives in the `insight` namespace.

## 4. Scope

### 4.1 In Scope

- Four streams (`claude_team_members`, `claude_team_invites`, `claude_team_overage_spend`,
  `claude_team_code_metrics`) into Bronze.
- The proxy service: headless Chromium + HTTP server, Helm chart, Dockerfile.
- `sessionKey`-based authentication and its rotation runbook.
- Cloudflare JS-challenge clearance via Playwright + stealth.
- Graceful handling of `HTTP 403 permission_error` from `/overage_spend_limits` when the
  operator's session lacks `billing:view`.
- Date-range incremental walking for `claude_team_code_metrics` (one request per day,
  configurable backfill anchor).
- All four streams stamped with `tenant_id`, `source_id`, `unique_key`, `collected_at`,
  `data_source: insight_claude_team`.

### 4.2 Out of Scope

- **Silver-layer dbt models.** Bronze persistence only in this issue. Silver mapping to
  `class_collab_chat_activity` / `class_ai_dev_usage` is follow-up work.
- **Identity Manager input wiring.** `claude_team_members.account.email_address` is a strong
  identity anchor and will feed `identity_inputs` once Silver lands, but the wiring is not in
  this issue's scope.
- **Automated `sessionKey` rotation.** Per #458 decision, MVP uses manual extraction (operator
  opens DevTools, copies the cookie). Automated email-code login via Playwright is out of
  scope.
- **`pr_attribution` / `top_users_by_prs` / `top_users_by_lines_of_code` response extras.**
  These top-level (not per-user) fields are present in the `metrics_aggs/users` response but
  empty in the reference organisation. If they become populated, they warrant a separate
  per-day-per-org stream rather than denormalisation into every user row.
- **Multi-organisation support.** Single `claude_org_id` per Source instance. Multi-org
  deployments use separate Sources / Secrets / proxy releases.
- **Production CI for the proxy image.** The proxy ships with a Dockerfile but no
  `.github/workflows/build-images.yml` entry yet — operator builds locally and pushes manually
  for now. Follow-up work to integrate with ADR-0016 patterns (which currently cover
  connectors only, not backend services).

## 5. Functional Requirements

### 5.1 Team Member Roster Collection

The connector **MUST** call `GET /api/organizations/{claude_org_id}/members` through the proxy
on every sync run and persist every returned row to `bronze_claude_team.claude_team_members`.

- The endpoint returns a plain JSON array of member objects (no pagination wrapper).
- Each member object has an `account` sub-object with `uuid`, `tagged_id`, `full_name`,
  `email_address`. The connector preserves the `account` nesting as-is in Bronze; Silver flattens.
- The connector **MUST** use `account.uuid` as the primary key.
- The connector **MUST** stamp every row with `tenant_id`, `source_id`, `unique_key`,
  `collected_at`, `data_source`. `unique_key` for members is built as
  `{tenant_id}-{source_id}-{account.uuid}`.

### 5.2 Pending Invite Collection

The connector **MUST** call `GET /api/organizations/{claude_org_id}/invites` through the proxy
on every sync run and persist every returned row to `bronze_claude_team.claude_team_invites`.

- The endpoint returns a plain JSON array of invite objects (no pagination wrapper).
- The endpoint returns **only currently-pending invites**; accepted or expired invites drop
  out of the response. The connector cannot reconstruct historical invite events from this
  endpoint, only the current pending set.
- The connector **MUST** use `uuid` as the primary key.

### 5.3 Per-Seat Spend Collection

The connector **MUST** call `GET /api/organizations/{claude_org_id}/overage_spend_limits` with
`page=N&per_page=100` through the proxy on every sync run, walking pages until the upstream
indicates no more, and persist every returned row to `bronze_claude_team.claude_team_overage_spend`.

- The endpoint returns an envelope: `{items: [...], page, per_page, total, total_pages}`.
- The connector **MUST** extract records from the `items` array.
- The connector **MUST** use `PageIncrement` pagination (1..total_pages).
- The connector **MUST** use `account_uuid` as the primary key.
- The connector **MUST** treat `HTTP 403` from this endpoint as a non-fatal condition. The
  proxy passes through the upstream status verbatim; the connector's error handler
  recognises `403` and commits zero records for the stream while leaving the overall sync
  status as succeeded. Once the operator rotates to a `sessionKey` whose user has the
  `billing:view` permission, the next sync starts populating rows automatically.

### 5.4 Per-User Code-Metrics Collection

The connector **MUST** call `GET /api/claude_code/metrics_aggs/users` through the proxy with
the following request parameters:

- `organization_uuid={claude_org_id}` (constant)
- `customer_type=claude_ai` (constant)
- `subscription_type=team` (constant)
- `sort_by=total_lines_accepted&sort_order=desc` (constant — does not affect correctness, kept
  for parity with the web UI)
- `start_date={cursor.start}&end_date={cursor.start}` (single-day window, both ends inclusive)
- `limit=100&offset=N` (offset pagination within a day)

The connector **MUST** walk one calendar day per request using a `DatetimeBasedCursor` with
`step: P1D, cursor_granularity: P1D`. The cursor's start is configurable via the `start_date`
spec field (default: 7 days back), bounded below by `2025-11-24` and above by yesterday.

The connector **MUST** inject the cursor's current value into every record as `metric_date` via
an `AddFields` transformation, so rows from different days do not collide on the composite
primary key `(metric_date, email)`.

The connector **MUST** persist every returned user record to
`bronze_claude_team.claude_team_code_metrics`.

### 5.5 Cloudflare Bot-Management Bypass

The proxy **MUST** complete a Cloudflare JS challenge at startup before serving any `GET /api/*`
request. Until the challenge is cleared, the proxy's `/health` endpoint **MUST** return
`HTTP 503` with body `{"ready": false}`; once cleared, `/health` **MUST** return `HTTP 200` with
body `{"ready": true, "transport": "playwright"}`.

The proxy **MUST** execute all upstream HTTP calls **inside the page context** via
`page.evaluate(fetch(...))`, so the request carries the Chromium TLS fingerprint, the `sessionKey`
cookie, and the `__cf_bm` clearance cookie set by Cloudflare. A request executed directly from
the Node.js runtime (bypassing the page) is rejected by Cloudflare on first contact.

The proxy **MUST** restart its browser session and re-clear Cloudflare on pod restart. K8s
liveness probe failures result in pod restart; the new pod re-clears Cloudflare during its init
phase before the readiness probe flips it back into service rotation.

### 5.6 Connector Operations and Data Integrity

- **Idempotence**: streams `members`, `invites`, `overage_spend` are full snapshots — repeated
  syncs do not duplicate rows because Airbyte's `full_refresh + overwrite` mode truncates Bronze
  before each load. Stream `code_metrics` uses `(metric_date, email)` PK; re-running the same
  date window upserts on the PK.
- **Schema stability**: declared inline in `connector.yaml.streams[*].schema_loader`. Changes
  trigger a MINOR semver bump per ADR-0015; breaking changes (renamed/removed fields, changed
  PK) trigger a MAJOR bump and `dbt --full-refresh` for any downstream Silver.
- **Tenant attribution**: every record carries `tenant_id` from `config.insight_tenant_id`,
  resolved at sync time from either env override or the cluster `data/insight-config`
  ConfigMap.

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

- **Freshness**: `members`, `invites`, `overage_spend` reflect upstream state within one sync
  cadence (default daily at 04:00 UTC). `code_metrics` lags upstream by 24 hours (cursor's
  `end_datetime` is yesterday).
- **Credential blast radius**: the connector descriptor holds no credentials; the K8s Secret
  applied by reconcile holds only `claude_org_id` (a non-secret UUID). All credential material
  (`sessionKey`) lives in the proxy's Helm-managed Secret, separate Helm release, separate
  rotation procedure.
- **Idempotence**: full-refresh streams are idempotent by construction. The incremental stream
  uses a deterministic composite PK so re-running the same window converges.
- **Availability target**: the proxy is single-replica by design; brief unavailability during
  pod restart (~30 seconds for Chromium boot + CF clearance) is acceptable because the
  connector's natural cadence is hours, not seconds.

### 6.2 NFR Exclusions

- High availability of the proxy is **not** a goal in v1.0. The connector retries failed syncs;
  a missed sync window is tolerable.
- Real-time data is **not** a goal. Per-user-per-day code metrics are deliberately delayed by
  one day to let upstream aggregations settle.

## 7. Public Library Interfaces

### 7.1 Public API Surface

The proxy exposes:

| Route | Method | Purpose |
|-------|--------|---------|
| `GET /api/*` | GET | Forward to `${UPSTREAM_BASE_URL}/api/*` via `page.evaluate(fetch)`; pass-through status, body, content-type |
| `GET /health` | GET | Readiness probe; reports `transport.isReady()` |

The proxy does **not** expose `POST/PUT/DELETE` (yet — `claude.ai`'s web API uses GET for the
endpoints we care about). The proxy does **not** translate, transform, or filter response
bodies — it forwards verbatim.

The connector's declarative manifest exposes the standard Airbyte source interface:
`spec`, `check`, `discover`, `read` per the Airbyte protocol.

### 7.2 External Integration Contracts

- **Upstream**: `https://claude.ai/api/*` (Anthropic's web app, unstable, no public contract).
  The connector's stream schemas are derived from observed responses and may need updating if
  Anthropic changes the response shape. Schema drift is detected at sync time via Airbyte's
  type-coercion errors.
- **Downstream**: Airbyte's ClickHouse destination connector writes to
  `bronze_claude_team.{members, invites, overage_spend, code_metrics}` tables with the
  standard `_airbyte_*` metadata columns (`_airbyte_raw_id`, `_airbyte_extracted_at`,
  `_airbyte_meta`, `_airbyte_generation_id`).

## 8. Use Cases

### UC-001 Deploy claude-team-proxy

**Trigger**: Operator wants to bring Claude Team analytics into Insight for the first time.

**Preconditions**:
- `sessionKey` extracted from `claude.ai` DevTools by a user with admin access.
- Insight kind / production cluster reachable via `kubectl`.

**Main Flow**:
1. Operator runs `helm install claude-team-proxy ./helm --namespace insight --set-string sessionKey="$SESSION_KEY"`.
2. Pod schedules; init container is empty (no migrations).
3. Container starts; Node.js process boots; `transport.init()` launches Chromium, injects
   `sessionKey`, navigates to `https://claude.ai`, waits for CF clearance (~10–30s).
4. `transport.isReady()` flips to true; `/health` returns 200.
5. K8s readiness probe succeeds; Service starts routing traffic to the pod.

**Postcondition**: `claude-team-proxy.insight.svc.cluster.local:3000` is reachable from other
in-cluster clients.

### UC-002 Configure and Run First Sync

**Trigger**: Operator wants to start ingesting from a newly-deployed proxy.

**Preconditions**: UC-001 complete; `claude_org_id` known.

**Main Flow**:
1. Operator copies `secrets/connectors/claude-team.yaml.example` to `claude-team.yaml`,
   fills `claude_org_id`, applies with `kubectl apply -n insight -f`.
2. Operator runs `bash src/ingestion/reconcile-connectors/main.sh reconcile --connector claude-team`.
3. Reconcile reads the Secret, registers the declarative manifest as a source definition in
   Airbyte, creates a Source with the operator's config, creates an Argo CronWorkflow.
4. Operator triggers the first sync manually (Airbyte UI or `POST /api/v1/connections/sync`).
5. Airbyte worker runs the manifest interpreter; per stream it issues HTTP requests to the
   proxy, which forwards them through Chromium to `claude.ai`.
6. Records land in `bronze_claude_team.*` tables.

**Postcondition**: Bronze tables populated; subsequent runs scheduled by Argo per
`descriptor.yaml.schedule`.

### UC-003 Rotate Expired sessionKey

**Trigger**: Operator notices syncs failing with `HTTP 401` from the proxy (or proactively
rotates per a security policy).

**Preconditions**: New `sessionKey` extracted by the admin user.

**Main Flow**:
1. Operator runs `helm upgrade claude-team-proxy ./helm --namespace insight --reuse-values --set-string sessionKey="$NEW_SESSION_KEY"`.
2. Helm rolls a new pod; new Chromium session boots, injects the new cookie, clears CF.
3. K8s drops the old pod once the new one is ready.

**Postcondition**: Next scheduled sync succeeds. Connector config and Secret unchanged.

### UC-004 Operator Lacks billing:view Permission

**Trigger**: Operator's `sessionKey` belongs to a user who does not have the `billing:view`
permission. This is common because the simplest path is to extract a cookie from any owner /
admin account, not specifically a billing-admin account.

**Preconditions**: UC-002 complete with a session that lacks `billing:view`.

**Main Flow**:
1. Sync runs; reaches `claude_team_overage_spend` stream.
2. Proxy forwards the request; `claude.ai` returns `HTTP 403` with
   `{"error":"permission_error","message":"Missing permissions: billing:view required"}`.
3. Proxy passes the 403 + body through verbatim.
4. Connector's `error_handler` matches `http_codes: [403]` with `action: IGNORE`.
5. Stream commits zero records; sync continues to the next stream.
6. Job ends with status `succeeded`; `claude_team_overage_spend` table stays empty.

**Postcondition**: Sync stays green; operator can later upgrade the user's role on `claude.ai`
or rotate to a different sessionKey, at which point the next sync starts populating spend rows
automatically with no code change.

## 9. Acceptance Criteria

- The proxy launches in under 60 seconds end-to-end (cold start, including CF clearance) on a
  development kind cluster.
- After UC-001 + UC-002, all four streams appear in `bronze_claude_team` schema; at least
  `members` and `code_metrics` have rows (for a reference org that has Claude Code usage).
- Triggering a sync with a billing-incapable sessionKey yields a green job and an empty
  `claude_team_overage_spend` table — never a red job.
- Rotating sessionKey via `helm upgrade` rolls the proxy pod without intervention in the
  connector or its Secret; the next scheduled sync succeeds against the new cookie.
- The connector descriptor passes the strict-semver check per ADR-0015 (`version` matches
  `^\d+\.\d+\.\d+$`).
- Connector adds the standard tenant attribution fields (`tenant_id`, `source_id`,
  `unique_key`, `collected_at`, `data_source`) to every Bronze record.

## 10. Dependencies

- **Airbyte v2.0+** with declarative source v7.0.4 manifest interpreter; supports
  `DatetimeBasedCursor`, `OffsetIncrement`, `PageIncrement`, `error_handler` with
  `IGNORE` action.
- **ClickHouse destination connector** v1.0+ with `Nullable(JSON)` support for the nested
  `account` object on members.
- **Reconcile toolkit** (`src/ingestion/reconcile-connectors/`) handles definition registration,
  source create, connection create, Argo CronWorkflow create. The connector contributes only
  `connector.yaml` and `descriptor.yaml` to the toolkit's input set.
- **Playwright v1.49+ + `playwright-extra` + `puppeteer-extra-plugin-stealth`** in the proxy.
  These versions are pinned in `src/backend/services/claude-team-proxy/package.json`.
- **Chromium browser binary** managed by Playwright; downloaded at proxy image-build time via
  `npx playwright install --with-deps chromium`.
- **kind / k8s cluster** with the `insight` namespace, RBAC for the operator to apply Secrets
  and run Helm against `insight`.

## 11. Assumptions

- `claude.ai`'s web API contract is **stable enough at MVP timescale** that the connector's
  hard-coded URLs and response-shape assumptions hold across a typical sync cadence. We accept
  the risk that Anthropic could change endpoints or response shapes; detection is at sync time
  via record-validation failures.
- The `sessionKey` lifetime is at least one week. We have not observed shorter lifetimes
  empirically; if Anthropic shortens session expiry, rotation cadence increases proportionally.
- Cloudflare's bot-management posture does not include a CAPTCHA tier for unauthenticated
  reads. We have observed only the JS challenge tier on `claude.ai`. If CAPTCHA is enabled,
  Playwright + stealth alone will not solve it and the architecture needs a CAPTCHA-solver
  service.
- The reference organisation has Claude Code usage on most days. Streams that produce zero rows
  for a fresh org (no Claude Code installed, no overage spend) are still valid empty syncs.

## 12. Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Cloudflare adds CAPTCHA tier | Low | High (architecture-wide redesign) | Monitor sync logs for CF challenge failures; have CloakBrowser CDP or curl-impersonate as fallback options |
| Anthropic changes the web API shape | Medium | Medium (per-stream re-spec) | Schema drift detected at sync time; descriptor MINOR bumps cover additive changes; MAJOR bump + dbt full-refresh covers breaking changes per ADR-0015 |
| `sessionKey` rotation friction | Medium | Low | Documented runbook (UC-003); single Helm-upgrade command; no connector changes required |
| Proxy single-replica → brief unavailability | High | Low | Sync cadence is hours; missed sync window is tolerable; pod-restart is typically <60s |
| Operator credential extraction is manual | High | Low | Documented procedure; future work to automate via email-code Playwright flow (out of MVP scope per #458) |
