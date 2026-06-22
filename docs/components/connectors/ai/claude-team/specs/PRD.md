# PRD — Claude Team Connector

| Field | Value |
|---|---|
| Component | `connectors/ai/claude-team` |
| Status | MVP |
| Owner | Insight Ingestion |
| Related issue | #458 |

## 1. Overview

### 1.1 Purpose

Ingest team-level data from a customer's **claude.ai Team** subscription
into the Insight Bronze layer. Four streams:

1. **Team roster** — current paying members with role, seat tier, join date.
2. **Pending invites** — open invites that have not yet been accepted.
3. **Per-seat overage spend** — current spend state per seat (optional;
   requires `billing:view`).
4. **Per-user Claude Code metrics** — daily usage (sessions, lines
   accepted, cost) by user account.

Downstream uses (Silver, Gold, dashboards) are out of scope for this PRD.

### 1.2 Background

claude.ai's internal web API has no public OAuth flow. The only way to
call it is from an authenticated browser session — a logged-in
sessionKey cookie, plus a real Chromium TLS fingerprint to pass
Cloudflare's bot-management challenge.

Because that session cookie is sensitive (it's the customer's logged-in
account credential), shipping it into Insight's infrastructure is
unacceptable from a privacy/legal standpoint.

**Architecture:** the customer hosts a small `claude-team-proxy`
container themselves. The proxy holds the cookie, drives the headless
browser, and exposes a narrow HTTP API. Insight authenticates to the
proxy with a shared bearer token and treats it as an opaque upstream.

Proxy source code, Dockerfile, and deployment instructions:
**[gitlab.constr.dev/insight/secure-enclave → proxies/claude_team/](https://gitlab.constr.dev/insight/secure-enclave)**

### 1.3 Goals

- Insight receives daily snapshots of the four streams, freshness ≤ 24 h.
- The customer's claude.ai sessionKey never leaves the customer's
  infrastructure.
- Cookie rotation is the customer's responsibility and does not require
  any change on the Insight side.
- Bearer-token rotation requires only a coordinated update of the K8s
  Secret on the Insight side and the env var on the proxy side.

### 1.4 Glossary

| Term | Meaning |
|---|---|
| sessionKey | claude.ai's session cookie; the user's logged-in credential |
| proxy | The customer-deployed `claude-team-proxy` container |
| proxy_auth_token | Shared bearer token between Insight and the customer's proxy |
| Bronze | Append-only landing layer in ClickHouse |
| CF | Cloudflare (used by claude.ai for bot management) |

## 2. Actors

| Actor | Role |
|---|---|
| Insight operator | Configures the K8s Secret with `proxy_url` + `proxy_auth_token` + `claude_org_id` |
| Customer ops | Hosts and operates the proxy container; rotates sessionKey on expiry |
| Insight reconcile loop | Renders Airbyte source/destination/connection from the descriptor |
| Airbyte | Executes the declarative source against the proxy |

## 3. Scope

### 3.1 In scope (MVP)

- Four streams as listed in §1.1.
- Declarative Airbyte source (`connector.yaml`) — no CDK Python.
- Authenticated bearer-token call to a customer-hosted proxy.
- Graceful skip when `overage_spend_limits` returns HTTP 403 (missing
  `billing:view`).
- Backfill window for `claude_team_code_metrics` configurable via
  `start_date` (default: 7 days ago).

### 3.2 Out of scope

- Proxy implementation. Lives in `secure-enclave`. The Insight side
  treats it as an opaque HTTP endpoint.
- Silver / Gold transformations were out of scope for the Bronze MVP
  but have since landed (`dbt_select: 'tag:claude-team+'`): Silver
  `claude_team__ai_dev_usage` → `class_ai_dev_usage` (INSIGHT-458) and
  `claude_team__ai_overage` → `class_ai_overage` (descriptor 1.3.0,
  Gold bullet `cc_overage`). See DESIGN §4.4.
- Real-time / streaming sync. Daily cron only.
- Multi-org. One connector instance per claude.ai org. To serve
  multiple orgs, deploy multiple proxy containers (one per org) and
  multiple Insight connector instances.
- API-key based access to claude.ai (no such API exists at the
  Team-plan level).

## 4. Functional Requirements

### 4.1 Members stream (`claude_team_members`)

- Endpoint: `GET /api/organizations/{org}/members`
- Full snapshot. No pagination. Plain JSON array.
- Primary key: `account.uuid`.
- Schema: `account.{uuid, tagged_id, full_name, email_address}`,
  `role`, `seat_tier`, `created_at`, `updated_at`.

### 4.2 Invites stream (`claude_team_invites`)

- Endpoint: `GET /api/organizations/{org}/invites`
- Full snapshot. Returns only **pending** invites — accepted / expired
  invites drop from the response and are not historically recoverable
  from this endpoint.
- Primary key: `uuid`.
- Schema: `uuid`, `email_address`, `role`, `status`, `created_at`,
  `expires_at`.

### 4.3 Overage spend stream (`claude_team_overage_spend`)

- Endpoint: `GET /api/organizations/{org}/overage_spend_limits`
- Envelope `{items: [...], page, per_page, total, total_pages}`.
- Page-paginated, 100/page.
- Primary key: `account_uuid`.
- **403 handling**: if the sessionKey lacks `billing:view`, the
  endpoint returns 403. Connector treats this as "no records" (sync
  stays GREEN). Once a billing-capable sessionKey is rotated in on
  the proxy, the next sync starts populating rows automatically — no
  change required on the Insight side.

### 4.4 Code metrics stream (`claude_team_code_metrics`)

- Endpoint: `GET /api/claude_code/metrics_aggs/users` with query
  parameters `organization_uuid`, `customer_type=claude_ai`,
  `subscription_type=team`, `sort_by=total_lines_accepted`,
  `sort_order=desc`, `start_date=YYYY-MM-DD`, `end_date=YYYY-MM-DD`,
  `limit=100`, `offset=N`.
- **Date semantics**: the API treats `start_date` and `end_date` as
  inclusive single-day filters; the connector walks one day per
  request via a `DatetimeBasedCursor` with `step: P1D`, injecting the
  cursor's current value into both params.
- Primary key: `(metric_date, email)`. `metric_date` is **injected by
  the connector** from the cursor — the API does not include it in
  per-user objects, so without injection two days' rows would collide
  on the primary key.
- Offset-paginated within a day, 100/page.
- Backfill hard floor: `2025-11-24` — earliest observed data point for
  the reference org; earlier dates return empty pages.
- Cost fields (`avg_cost_per_day`, `total_cost`) come back as **strings**
  (decimal-as-string convention). Bronze keeps them as strings; Silver
  is expected to cast.

### 4.5 Cross-stream

- Connector injects `tenant_id`, `source_id`, `unique_key`,
  `collected_at`, `data_source` on every row.
- `Authorization: Bearer <proxy_auth_token>` header on every request.
- On bearer-token mismatch, proxy returns 401 → sync fails RED.

## 5. Non-functional requirements

| Concern | Target |
|---|---|
| Sync cadence | Daily, 04:00 UTC (configurable per tenant) |
| Freshness | ≤ 24 h end-to-end |
| Sync duration (4 streams, ~150-seat org, 7-day backfill) | < 5 minutes |
| Failure isolation | A single stream failure (e.g. 403 on overage) does not fail the whole sync |
| Audit | Bronze rows are append-only, every row carries `collected_at` |

## 6. Operational concerns

| Concern | Owner |
|---|---|
| sessionKey rotation on expiry | Customer ops (POST /admin/session-key on proxy) |
| `proxy_auth_token` rotation | Coordinated: Insight Secret + proxy env (requires container restart) |
| `claude_org_id` change | Coordinated: Insight Secret + proxy env |
| Proxy container restart / health | Customer ops |
| Sync failure alerts | Insight (Airbyte → Argo workflow status) |

## 7. Open questions

- Customer-side mTLS or auth-proxy in front of the proxy: out of scope
  for this PRD; documented as a deployment hint in
  `secure-enclave/proxies/claude_team/README.md`.
- Multi-region failover for the proxy: not addressed; single container
  per org.

## 8. Acceptance

A claude-team connector deployment is considered functional when:

- `connector.yaml validate-strict` passes against the declarative
  source schema.
- The K8s Secret (with the 3 required keys) reconciles into an Airbyte
  source + connection without manual intervention.
- A test sync produces non-zero records for `claude_team_members` and
  (if the org has Claude Code activity) `claude_team_code_metrics` in
  `bronze_claude_team.*`.
- `claude_team_overage_spend` is empty if and only if the proxy's
  sessionKey lacks `billing:view`; the sync still completes GREEN.
- A sessionKey rotation on the proxy side (via
  `POST /admin/session-key`) does not require any Insight-side change
  and the next sync runs cleanly.
