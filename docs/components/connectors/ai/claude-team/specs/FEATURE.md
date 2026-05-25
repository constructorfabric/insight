# Feature: Claude Team Connector — Bronze Bring-up

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Operator Deploys the Proxy + Connector](#operator-deploys-the-proxy--connector)
  - [Scheduled Sync Run](#scheduled-sync-run)
  - [sessionKey Rotation](#sessionkey-rotation)
  - [Permission-Limited Sync (overage_spend 403)](#permission-limited-sync-overage_spend-403)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Proxy Boot and CF Clearance](#proxy-boot-and-cf-clearance)
  - [Connector Per-Stream Read Loop](#connector-per-stream-read-loop)
  - [code_metrics Day-by-Day Walker](#code_metrics-day-by-day-walker)
- [4. States (CDSL)](#4-states-cdsl)
  - [Proxy Transport State](#proxy-transport-state)
  - [code_metrics Cursor State](#code_metrics-cursor-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Package Files Present](#package-files-present)
  - [Proxy Service Present](#proxy-service-present)
  - [Secret Example Present](#secret-example-present)
  - [Artefacts Registered in Airbyte](#artefacts-registered-in-airbyte)
  - [All Validators Pass](#all-validators-pass)
  - [Live Smoke Tests Pass](#live-smoke-tests-pass)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Traceability](#7-traceability)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

The Claude Team Connector ships a four-stream Airbyte declarative source plus a long-running
proxy service that fronts `claude.ai`'s Cloudflare-protected web API. This feature document
covers the end-to-end operational flows that make those streams land in Bronze: deploying the
proxy, configuring the connector, running scheduled syncs, rotating the `sessionKey`, and
handling the most-common permission failure (operator's session lacks `billing:view`).

### 1.2 Purpose

The connector replaces a hypothetical "Insight upgrades every customer to Enterprise plan"
move with a tractable path: extract whatever the Team-plan admin sees in the web UI, via the
same internal API the web UI consumes, behind a maintained Cloudflare bypass.

### 1.3 Actors

- **Platform Operator** — runs Helm commands, applies K8s Secrets, triggers reconcile, watches
  syncs. Owns the rotation procedure when `sessionKey` expires.
- **`claude.ai` Organisation Admin** — extracts the `sessionKey` cookie from a logged-in browser
  session. May be the same human as Operator in small deployments.
- **Airbyte server** — runs the manifest interpreter, schedules syncs, ships records.
- **Argo Workflows** — triggers Airbyte syncs on the descriptor's schedule.
- **`claude-team-proxy` pod** — headless Chromium + HTTP API; long-running.
- **Cloudflare edge** — issues JS challenge on first contact; passes subsequent requests with
  `__cf_bm` cookie.
- **ClickHouse destination** — writes records into `bronze_claude_team.*`.

### 1.4 References

- PRD: `./PRD.md`
- DESIGN: `./DESIGN.md`
- ADR-0015 (strict semver + full-refresh): `../../../airbyte-toolkit/specs/ADR/0015-semver-and-full-refresh.md`
- ADR-0016 (descriptor images block): `../../../airbyte-toolkit/specs/ADR/0016-descriptor-images-block.md`
  (informational — this connector has no images, but uses the descriptor schema)
- Proxy code: `../../../../src/backend/services/claude-team-proxy/`
- Connector code: `../../../../src/ingestion/connectors/ai/claude-team/`
- Sibling precedent: `../../collaboration/zulip-proxy/specs/` (same proxy-sidecar pattern for Zulip)

## 2. Actor Flows (CDSL)

### Operator Deploys the Proxy + Connector

```cdsl
flow OperatorDeploysProxyAndConnector
  actor: Platform Operator
  precondition: kind/prod cluster reachable; `sessionKey` extracted; `claude_org_id` known
  goal: ingest claude-team data into bronze_claude_team.*

  steps:
    1. operator extracts `sessionKey` from claude.ai DevTools
    2. operator runs:
         helm install claude-team-proxy ./helm \
           --namespace insight \
           --set-string sessionKey="$SESSION_KEY"
    3. wait_until: kubectl get pod -n insight -l app.kubernetes.io/name=claude-team-proxy
                   reports Running 1/1
    4. operator copies secrets/connectors/claude-team.yaml.example to claude-team.yaml,
       fills claude_org_id, applies with kubectl apply -n insight -f
    5. operator runs:
         bash src/ingestion/reconcile-connectors/main.sh reconcile --connector claude-team
       (or via in-cluster CronWorkflow)
    6. reconcile: registers manifest as source definition, creates Source, creates Connection,
                  creates CronWorkflow on descriptor.schedule
    7. operator triggers first sync (Airbyte UI or POST /api/v1/connections/sync)
    8. wait_until: sync job status='succeeded'
    9. verify: bronze_claude_team.{members,invites,overage_spend,code_metrics} exist

  postcondition: data flows; subsequent syncs scheduled by Argo
```

### Scheduled Sync Run

```cdsl
flow ScheduledSyncRun
  actor: Argo Workflows (system)
  precondition: connector configured per OperatorDeploysProxyAndConnector; proxy healthy
  goal: refresh bronze_claude_team.* with the latest claude.ai state

  steps:
    1. Argo CronWorkflow fires at descriptor.schedule (default: daily 04:00 UTC)
    2. workflow triggers Airbyte connection-sync API: POST /api/v1/connections/sync
    3. Airbyte worker runs manifest interpreter:
        for each stream in [members, invites, overage_spend, code_metrics]:
          for each slice in stream.cursor_slices  (cursor only for code_metrics)
            for each page in slice.pagination
              HTTP GET proxy_url + stream.path with stream.params
              proxy executes page.evaluate(fetch(claude.ai/...))
              records extracted per stream.record_selector
              records stamped with tenant_id/source_id/unique_key/collected_at/data_source
              records shipped to ClickHouse destination
    4. ClickHouse destination upserts/truncates per stream.destinationSyncMode
    5. job status flips to 'succeeded' (or 'failed' on hard error)
    6. workflow emits one line to logs; persists job stats in Airbyte db

  postcondition: bronze tables refreshed; cursor state advanced for code_metrics
```

### sessionKey Rotation

```cdsl
flow SessionKeyRotation
  actor: Platform Operator
  trigger: sync job reports HTTP 401 from claude.ai through proxy; OR proactive rotation
  goal: restore syncs without touching connector or its Secret

  steps:
    1. operator notices red sync (Airbyte UI / alert / runbook check)
    2. operator opens claude.ai in browser as admin user
    3. extracts new sessionKey from DevTools → Application → Cookies → sessionKey value
    4. operator runs:
         helm upgrade claude-team-proxy ./helm \
           --namespace insight \
           --reuse-values \
           --set-string sessionKey="$NEW_SESSION_KEY"
    5. Helm rolls a new pod (maxSurge=1, maxUnavailable=0 — new pod up before old drops)
    6. new pod runs transport.init():
         chromium.launch → context.addCookies(new sessionKey) → page.goto(claude.ai)
         → waits for CF clearance (~10-30s) → transport.isReady = true
    7. readiness probe flips new pod into Service rotation
    8. K8s drops old pod
    9. operator triggers a manual sync to verify (POST /api/v1/connections/sync)
   10. wait_until: sync status='succeeded'

  postcondition: subsequent scheduled syncs succeed against the new cookie; no connector
                 secret touched; no Source / Connection / CronWorkflow change
```

### Permission-Limited Sync (overage_spend 403)

```cdsl
flow PermissionLimitedSync
  actor: Airbyte worker
  trigger: sync reaches claude_team_overage_spend stream
  precondition: sessionKey's user lacks `billing:view` permission on claude.ai

  steps:
    1. worker issues HTTP GET proxy:3000/api/orgs/<id>/overage_spend_limits?page=1&per_page=100
    2. proxy forwards to claude.ai via page.evaluate(fetch)
    3. claude.ai returns HTTP 403 {"error":{"type":"permission_error","message":"Missing
       permissions: billing:view required..."}}
    4. proxy passes status+body verbatim → HTTP 403 to worker
    5. worker's error_handler for this stream matches http_codes: [403] with action: IGNORE
    6. worker logs WARN: "claude.ai returned 403 — sessionKey lacks billing:view permission"
    7. stream emits zero records; sync proceeds to next stream
    8. final job status: 'succeeded'
    9. claude_team_overage_spend bronze table either empty (first sync) or
       contains rows from previous (permitted) sync (rare; not our case)

  postcondition: sync is green; spend stream stays empty until operator rotates to
                 billing-capable sessionKey or upgrades the user's role on claude.ai
```

## 3. Processes / Business Logic (CDSL)

### Proxy Boot and CF Clearance

```cdsl
process ProxyBootAndCFClearance
  trigger: K8s creates the proxy pod
  goal: have transport.isReady() return true with a Cloudflare-cleared browser session

  steps:
    1. node:22-slim container starts; src/index.js runs
    2. config = loadConfig(env)
         required: SESSION_KEY
         optional: PORT, UPSTREAM_BASE_URL, HEADLESS, STARTUP_TIMEOUT_MS
         throws synchronously on missing/invalid
    3. transport = createPlaywrightTransport(config)
         no I/O; closure over private state
    4. server = createServer(transport)
         http.createServer(handleRequest)
    5. server.listen(PORT)
         readiness probe can now hit /health (returns 503 until transport.isReady)
    6. installShutdownHandlers(server, transport)
         SIGTERM / SIGINT → graceful drain (30s hard timeout)
    7. await transport.init():
         chromium.launch({headless, --disable-blink-features=AutomationControlled})
         context.addCookies(sessionKey, domain=.claude.ai)
         page.goto('https://claude.ai', timeout: STARTUP_TIMEOUT_MS)
         page.waitForFunction(() => !document.title.includes('Just a moment'))
    8. transport.ready = true; log "proxy ready"
    9. /health returns 200 {"ready": true, "transport": "playwright"}
   10. K8s readiness probe flips Service into routing this pod

  postcondition: pod is in Service rotation; can serve /api/* requests
```

### Connector Per-Stream Read Loop

```cdsl
process ConnectorPerStreamReadLoop
  trigger: Airbyte worker is executing the declarative manifest's stream[N].retriever
  goal: extract all records from that stream into Airbyte's destination pipeline

  steps:
    1. resolver builds url = url_base + path with config substitution
         e.g. http://claude-team-proxy.insight.svc.cluster.local:3000/api/organizations/<UUID>/members
    2. requester issues HTTP GET via standard CDK HttpRequester
         no authenticator (proxy holds it)
    3. response received:
         if requester.error_handler matches → apply action (IGNORE for 403 on overage_spend)
         else if 2xx → continue
         else → fail stream
    4. record_selector.extractor extracts records from response per field_path
         []      for members / invites
         [items] for overage_spend
         [users] for code_metrics
    5. paginator updates state from response:
         PageIncrement: next page = current + 1, stop when fewer than page_size records
         OffsetIncrement: next offset = current + page_size, same stop condition
         (no paginator for members / invites)
    6. for each record:
         apply transformations (inline AddFields per stream — validate-strict forbids
         the whole-object $ref pattern, so each stream carries its own copy):
           - tenant_id          = config.insight_tenant_id
           - source_id          = config.insight_source_id
           - unique_key         = '{tenant_id}-{source_id}-{<natural-key>}'
                                  (natural-key differs per stream: account.uuid for
                                  members, uuid for invites, account_uuid for
                                  overage_spend, metric_date+email for code_metrics)
           - collected_at       = now_utc()
           - data_source        = 'insight_claude_team'
           - metric_date        = cursor.start_time (code_metrics only)
         schema validation (informational; type-coerce or warn)
         ship to destination
    7. loop pagination until paginator says stop
    8. loop cursor slices until DatetimeBasedCursor says stop (code_metrics only)

  postcondition: all records for this stream/sync committed to destination; cursor state
                 persisted (incremental streams only)
```

### code_metrics Day-by-Day Walker

```cdsl
process CodeMetricsDayByDayWalker
  trigger: Airbyte worker is executing stream[3].retriever (claude_team_code_metrics)
  goal: walk one day per HTTP request across the configured backfill window

  steps:
    1. cursor.start_datetime = config.get('start_date') or day_delta(-7, '%Y-%m-%d'),
         floored to '2025-11-24'
    2. cursor.end_datetime = day_delta(-1, '%Y-%m-%d')   // yesterday in UTC
    3. step = P1D; cursor_granularity = P1D
    4. for each slice in range(cursor.start_datetime, cursor.end_datetime, step=P1D):
         slice.start_time = current day (e.g. 2026-05-18)
         slice.end_time   = same day (P1D step + cursor_granularity P1D)
         inject into request: start_date=slice.start, end_date=slice.start
         offset = 0
         loop:
           GET /api/claude_code/metrics_aggs/users?start_date=…&end_date=…&limit=100&offset=N&…
           extract records from response.users
           for each record: AddFields metric_date = slice.start_time
           ship to destination
           if response carries < 100 users: break (last page)
           offset += 100
    5. advance to next day; repeat until cursor.end_datetime reached
    6. persist cursor.current = cursor.end_datetime in Airbyte's state

  postcondition: claude_team_code_metrics holds rows for every day in the walked window;
                 cursor state persisted for incremental continuation on next sync
```

## 4. States (CDSL)

### Proxy Transport State

```cdsl
state ProxyTransportState
  identity: the singleton PlaywrightTransport closure inside a proxy pod

  fields:
    browser: Browser | null     — Playwright Browser instance
    context: BrowserContext | null
    page: Page | null
    ready: boolean              — exposed via isReady()
    kind: 'playwright'          — exposed for /health and x-proxy-transport header
    upstreamBaseUrl: string     — exposed for server.js to build URLs

  transitions:
    {ready: false, browser: null} → init() → {ready: true, browser: ≠null}
    {ready: true} → SIGTERM      → close() → {ready: false, browser: null}
    {ready: true} → browser.crash → /health 503; pod restarted by K8s liveness

  visibility:
    isReady()  — sync, used by /health and server.js path guards
    kind       — sync, used by /health and x-proxy-transport response header
    upstreamBaseUrl — sync, used by server.js to build upstream URLs
```

### code_metrics Cursor State

```cdsl
state CodeMetricsCursorState
  identity: per-connection, persisted in Airbyte's internal state store
  scope:    one Source instance × one connection_id × one stream

  fields:
    current_date: string (YYYY-MM-DD) — high-water mark; next sync starts here + 1 day

  transitions:
    fresh connection: current_date = null
                      → first sync walks from config.start_date (or default) to yesterday
                      → on success: current_date = yesterday

    subsequent sync:  current_date = T-1 (yesterday of previous sync)
                      → walks from current_date + 1 day to today's yesterday
                      → on success: current_date = today's yesterday

    operator overrides config.start_date to earlier date:
      next sync walks from new start_date to yesterday (re-fetches days; PK-merge upserts)

    on sync failure:
      current_date unchanged; next sync retries the same window

  persistence: Airbyte's state store (Postgres in airbyte ns); survives pod restarts
```

## 5. Definitions of Done

### Package Files Present

The repo includes these files under `src/ingestion/connectors/ai/claude-team/`:

- `descriptor.yaml` — orchestration metadata (name, version, schedule, dbt_select, connection
  namespace, required secret fields).
- `connector.yaml` — Airbyte v7.0.4 DeclarativeSource with 4 streams (`claude_team_members`,
  `claude_team_invites`, `claude_team_overage_spend`, `claude_team_code_metrics`), spec
  block, and inline AddFields transformations (one per stream — `validate-strict` rejects
  whole-object `$ref` definitions, so the attribution injection is duplicated rather than
  shared).

And under `src/ingestion/secrets/connectors/`:

- `claude-team.yaml.example` — K8s Secret template documenting required field
  (`claude_org_id`) and the auth-separation note (sessionKey lives in proxy's Helm Secret).

### Proxy Service Present

The repo includes the proxy service under `src/backend/services/claude-team-proxy/`:

- `package.json` — Node ≥20, type=module, deps: `playwright`, `playwright-extra`,
  `puppeteer-extra-plugin-stealth`.
- `src/index.js` — entry: config → transport → server, lifecycle, graceful shutdown.
- `src/config.js` — env parser with validation (`SESSION_KEY`, `PORT`, `HEADLESS`,
  `STARTUP_TIMEOUT_MS`, `UPSTREAM_BASE_URL`).
- `src/log.js` — minimal JSON-line logger.
- `src/server.js` — `node:http`-based router (`GET /api/*` + `GET /health`).
- `src/transport/index.js` — `AuthedTransport` contract (JSDoc) + factory.
- `src/transport/playwright.js` — Playwright + stealth implementation.
- `Dockerfile` — multi-stage build, non-root `node` user, Chromium pre-installed.
- `.dockerignore` — keeps `node_modules`, `.env`, build artefacts out of build context.
- `helm/Chart.yaml`, `helm/values.yaml`, `helm/templates/{deployment,service,secret}.yaml`,
  `helm/templates/_helpers.tpl` — Helm chart for k8s deploy.

### Secret Example Present

`src/ingestion/secrets/connectors/claude-team.yaml.example` is in git and documents:
- The required `claude_org_id` field.
- The auth-separation contract — sessionKey is NOT in this Secret; it's in the proxy's
  Helm-managed Secret.
- The rotation procedure for sessionKey.
- The annotations consumed by reconcile.

### Artefacts Registered in Airbyte

After `reconcile-connectors/main.sh reconcile --connector claude-team`:
- A `source_definition` exists in the workspace with the declarative manifest.
- A `source` instance exists for the operator's `claude_org_id`.
- A `connection` exists linking that source to the shared ClickHouse destination, with all
  four streams selected and configured (`full_refresh + overwrite` for snapshot streams,
  incremental for `code_metrics`).
- An Argo `CronWorkflow` exists in the `argo` namespace with schedule from `descriptor.yaml`.

### All Validators Pass

- `descriptor.yaml.version` matches `^\d+\.\d+\.\d+$` (ADR-0015).
- `descriptor.yaml` has no `images:` block (correct for declarative connectors per ADR-0016
  — only CDK/enrich connectors need it).
- Connector manifest validates against Airbyte's declarative-source schema (Airbyte's UI
  builder validation passes; reconcile's create/publish API call returns 2xx).

### Live Smoke Tests Pass

Against a kind cluster with proxy deployed:
- `curl http://claude-team-proxy.insight.svc.cluster.local:3000/health` returns 200 `{ready:true}`.
- Airbyte UI shows 4 streams under the `claude-team` source.
- A manual sync trigger from Airbyte UI or `POST /api/v1/connections/sync` returns a job that
  reaches status `succeeded` (overage_spend may have 0 records if sessionKey lacks
  `billing:view` — that is acceptable).
- ClickHouse query
  `SELECT count() FROM bronze_claude_team.claude_team_members` returns a positive number for
  the reference organisation.
- ClickHouse query
  `SELECT count() FROM bronze_claude_team.claude_team_code_metrics WHERE metric_date >= today() - 7`
  returns a positive number.

## 6. Acceptance Criteria

- `helm install` of `claude-team-proxy` reaches `Running 1/1` readiness within 60 seconds.
- `bash reconcile-connectors/main.sh reconcile --connector claude-team` produces no errors
  and registers all four streams.
- A first sync after registration commits records to all four streams except
  `claude_team_overage_spend` if the operator's session lacks `billing:view` (zero rows
  there, sync still green).
- `helm upgrade --reuse-values --set-string sessionKey=…` rotates the cookie without
  touching the connector or its Secret; the next scheduled sync succeeds.
- `descriptor.yaml.version` is `1.1.0` (initial release with 4 streams) and passes the
  strict-semver validator.
- Every row in every Bronze table carries `tenant_id`, `source_id`, `unique_key`,
  `collected_at`, `data_source: insight_claude_team`.

## 7. Traceability

| Spec | Implementation | Test artefact |
|------|----------------|---------------|
| PRD §5.1 members | `connector.yaml` streams[0] | `bronze_claude_team.claude_team_members` smoke query |
| PRD §5.2 invites | `connector.yaml` streams[1] | `bronze_claude_team.claude_team_invites` smoke query |
| PRD §5.3 overage_spend + 403 grace | `connector.yaml` streams[2].retriever.requester.error_handler | manual: trigger sync with billing-incapable sessionKey, expect green job + empty table |
| PRD §5.4 code_metrics + cursor | `connector.yaml` streams[3].incremental_sync | `bronze_claude_team.claude_team_code_metrics` per-day distribution check |
| PRD §5.5 CF bypass | `src/transport/playwright.js` `init()` | `curl proxy:3000/health` returns 200 within 60s of pod start |
| PRD §5.6 tenant attribution | `connector.yaml` inline `AddFields` transformations on each of streams[0..3] | sample row inspection: 5 attribution fields (`tenant_id`, `source_id`, `unique_key`, `collected_at`, `data_source`) non-null |
| DESIGN §3.2 component model — AuthedTransport seam | `src/transport/index.js` | swap test (mock transport): `createTransport({kind: 'mock'})` returns mock without code changes elsewhere — future work |
| DESIGN §3.8 deployment topology | `helm/templates/*.yaml`, `secrets/connectors/claude-team.yaml.example` | `kubectl get all -n insight -l app.kubernetes.io/name=claude-team-proxy` matches diagram |
| FEATURE OperatorDeploysProxyAndConnector | Operator runbook in `README.md` (TBD) | manual: walk through; first sync succeeds |
| FEATURE SessionKeyRotation | `helm upgrade --reuse-values --set-string sessionKey=…` | manual: rotation completes; subsequent sync succeeds |
