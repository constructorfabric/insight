# DESIGN — ChatGPT Team Connector

> Version 2.0 — June 2026
> Supersedes: v1.0 (March 2026) — source corrected from OpenAI Admin API to a customer-hosted browser proxy over `chatgpt.com`. See [ADR-001](./ADR/ADR-001-browser-proxy-architecture.md).
> Reference implementation: `claude-team` (connector) + `secure-enclave/proxies/claude_team` (proxy).

<!-- toc -->

- [1. Architecture overview](#1-architecture-overview)
- [2. Configuration](#2-configuration)
- [3. Proxy (secure-enclave)](#3-proxy-secure-enclave)
- [4. Streams](#4-streams)
- [5. Bronze Tables](#5-bronze-tables)
- [6. Identity Resolution](#6-identity-resolution)
- [7. Silver / Gold Mappings](#7-silver--gold-mappings)
- [8. Risks & Open Items](#8-risks--open-items)

<!-- /toc -->

---

## 1. Architecture overview

```text
  ┌──────────────────────────┐                   ┌────────────────────────────┐
  │   Insight cluster        │                   │   Customer environment     │
  │                          │                   │                            │
  │   reconcile-loop         │                   │   chatgpt-team-proxy        │
  │      │                   │                   │   (Docker container,       │
  │      ▼                   │   HTTPS +         │    Playwright + Chromium,   │
  │   Airbyte source ────────┼───Bearer token───►│    holds chatgpt.com        │
  │   (declarative)          │   (proxy_auth_    │    session + access token   │
  │      │                   │    token)         │    in memory only)          │
  │      ▼                   │                   │            │               │
  │   ClickHouse Bronze      │                   │            ▼               │
  │   bronze_chatgpt_team.*  │                   │       chatgpt.com           │
  └──────────────────────────┘                   │       /backend-api/*        │
                                                 └────────────────────────────┘
```

Same shape as `claude-team`. The Insight side runs an Airbyte declarative source against a customer-hosted HTTP proxy and never sees the `chatgpt.com` session. The proxy is opaque to Insight and lives in `secure-enclave/proxies/chatgpt_team/`, operationally owned by the customer. This document covers the **Insight-side** connector plus the proxy contract it depends on.

The connector is **not** the OpenAI Admin API connector (`openai`). It collects the conversational / Codex / subscription surface that only `chatgpt.com/backend-api/*` exposes.

---

## 2. Configuration

### 2.1 Spec fields (Airbyte connection_specification)

| Field | Required | Source | Notes |
|---|---|---|---|
| `proxy_url` | yes | K8s Secret | Base URL of the customer-hosted `chatgpt-team-proxy` (no default) |
| `proxy_auth_token` | yes | K8s Secret | Shared bearer token; masked in logs |
| `chatgpt_account_id` | yes | K8s Secret | ChatGPT workspace account UUID — used by `/accounts/{account_id}/*` endpoints |
| `chatgpt_org_id` | no | K8s Secret | ChatGPT organization UUID — used ONLY by the subscription streams (`/subscriptions/{org_id}/*`). Optional: blank for `analytics-viewer` accounts without billing visibility → subscription streams skip (403/404 ignored). NOT used by `/wham/*` (usage-leaderboard takes no org in its path). |
| `insight_tenant_id` | yes | reconcile loop | Tenant slug, injected as `tenant_id` |
| `insight_source_id` | yes | Secret annotation | `insight.cyberfabric.com/source-id` |
| `start_date` | no | K8s Secret | Earliest date for daily activity backfill (YYYY-MM-DD); default 7 days ago |

There is **no** OpenAI admin key and **no** `chatgpt.com` session field here. The session lives only on the proxy and is installed via the proxy's `POST /admin/session-key`.

> `chatgpt_account_id` vs `chatgpt_org_id`: distinct identifiers used by different `backend-api` endpoints (OQ-CGT-4). Both are accepted as config until the live instance confirms whether one can be derived from the other.

### 2.2 K8s Secret shape

Mirrors `claude-team`. Discovered by the reconcile loop via label `app.kubernetes.io/part-of=insight` and annotations `insight.cyberfabric.com/{connector,source-id}`. `stringData` carries `proxy_url`, `proxy_auth_token`, `chatgpt_account_id`, `chatgpt_org_id`, optional `start_date`.

---

## 3. Proxy (secure-enclave)

New proxy `secure-enclave/proxies/chatgpt_team`, forked from `claude_team`:

- **Transport**: Playwright headless Chromium + stealth plugin; clears Cloudflare on `chatgpt.com` (waits out the challenge page), then performs `fetch(..., {credentials:'include'})` inside the page so the session cookie is attached automatically.
- **HTTP contract** (unchanged from `claude_team`): `GET /health` (open), `POST /admin/session-key` (bearer), `GET /api/*` (bearer) → passthrough to `https://chatgpt.com/backend-api/*`. Insight authenticates with `proxy_auth_token`.
- **New vs `claude_team` — access-token exchange**: `chatgpt.com` likely requires a short-lived bearer `access_token` (from `GET /api/auth/session`) on `backend-api` calls, in addition to the session cookie. The proxy is expected to derive and refresh this token internally so Insight's requests stay as plain `GET /api/*`. Whether this is required, and its TTL, is **OQ-CGT-3** — the first de-risking task; it determines how much of this section is needed.
- **Bootstrap & rotation**: operator installs the session string via `POST /admin/session-key`; rotation when `/health` starts returning 503 or `/api/*` returns auth errors. One proxy per workspace.

---

## 4. Streams

All streams emit Bronze rows with a common envelope: `tenant_id`, `source_id`, `unique_key`, `collected_at`, `data_source = "insight_chatgpt_team"`, plus stream-specific fields. Auth header on every request: `Authorization: Bearer {proxy_auth_token}`; `url_base = {proxy_url}`.

| Stream | Endpoint (via proxy → `chatgpt.com/backend-api`) | Sync mode | Pagination | Cursor / PK |
|---|---|---|---|---|
| `chatgpt_team_seats` | `/accounts/{account_id}/users` | Full snapshot | offset/limit | PK `user_id` |
| `chatgpt_team_chat_activity` | `/accounts/{account_id}/analytics/user_list` (`start_date`,`end_date`,`page_size`,`after_cursor`) | Incremental | cursor (`after_cursor`) | cursor `date`; PK (`date`,`email`) |
| `chatgpt_team_codex_user_daily` | `/wham/analytics/usage-leaderboard` (`start_date`,`end_date`,`window_days`,`page`,`page_size`) | Incremental | page-number | cursor `date`; PK (`date`,`email`) |
| `chatgpt_team_subscription_usage` | `/subscriptions/{org_id}/usage` (`start_date`,`end_date`) | Full refresh (daily snapshot) | none | PK (`snapshot_date`,`model`); 403/404 ignored |
| `chatgpt_team_subscription_balance` | `/subscriptions/{org_id}/usage` (`start_date`,`end_date`) | Full refresh (daily snapshot) | none | PK `snapshot_date`; 403/404 ignored |
| `chatgpt_team_collection_runs` | connector-generated | — | — | **NOT implemented** — a declarative source cannot emit it; deferred / platform-side |

Date-windowed streams use a `DatetimeBasedCursor` over `date` with `step: P1D` (or the largest window the endpoint supports), `start_datetime` from `start_date`, and a `min_datetime` floor at the earliest date the endpoint returns data — the same shape as `claude_team_code_metrics`. As with that stream, `date` is injected onto each row from the cursor interval because the per-user objects do not carry it.

Auxiliary endpoints seen in the prototype (`/wham/analytics/daily-sessions-messages-counts`, `/wham/usage/daily-enterprise-token-usage-breakdown`, `/subscriptions/{account_id}/usage/export` CSV) are candidate alternates/supplements to validate during de-risking; they are not in the Phase-1 stream set above unless a target field is missing from the chosen endpoint.

---

## 5. Bronze Tables

> Field lists are grounded in the `data_collector/apps/openai` prototype and **must be verified against the live instance** during de-risking (see OQ-CGT-3/4/6). Field naming: snake_case, preserved as-is at Bronze.

### `chatgpt_team_seats` — Seat assignment and status

| Field | Type | Description |
|---|---|---|
| `user_id` | String | ChatGPT user ID (primary key) |
| `email` | String | User email — cross-system identity key |
| `name` | String | Display name |
| `role` | String | e.g. `standard-user` / `account-admin` / `account-owner` (verified live) |
| `seat_type` | String | Seat type, e.g. `default` (verified live) |
| `added_at` | DateTime64(3) | When the seat was assigned — promoted from the API's `created_time` field (verified live) |

One row per user. Current-state only — no versioning. `unique_key = {tenant}-{source}-{user_id}`.

> Phase 1 promotes only the fields above. `status` / `last_active_at` are not surfaced by this endpoint and are not promoted (present only in the raw record if the API returns them).

### `chatgpt_team_chat_activity` — Daily chat usage per user

| Field | Type | Description |
|---|---|---|
| `date` | Date | Activity date (injected from cursor) |
| `email` | String | User email — identity key |
| `name` | String | Display name |
| `seat_type` | String | Seat/plan type for the user on that day |
| `messages` | Float64 | Total messages sent |
| `gpt_messages` | Float64 | Messages to GPT models |
| `tool_messages` | Float64 | Tool-invocation messages |
| `connector_messages` | Float64 | Connector messages |
| `project_messages` | Float64 | Project messages |
| `credits_used` | Float64 | Credits consumed that day |

`unique_key = {tenant}-{source}-{date}-{email}`. (Per-model chat token breakdown / `reasoning_tokens` are **not** confirmed available from this endpoint — see OQ-CGT-6; add only if verified.)

### `chatgpt_team_codex_user_daily` — Daily Codex usage per user

| Field | Type | Description |
|---|---|---|
| `date` | Date | Activity date (injected from cursor) |
| `email` | String | User email — identity key |
| `user_id` | String | ChatGPT user ID |
| `name` | String | Display name |
| `credits` | Float64 | Codex credits consumed |
| `n_threads` | Float64 | Codex threads |
| `n_turns` | Float64 | Codex turns |
| `current_streak` | Float64 | Active-day streak |
| `text_tokens` | Float64 | Text tokens |
| `lines_added` | Float64 | Lines accepted/added |

`unique_key = {tenant}-{source}-{date}-{email}`. Feeds `class_ai_dev_usage` (parallel to `claude_team_code_metrics`).

### `chatgpt_team_subscription_usage` — Subscription usage per model (daily snapshot)

| Field | Type | Description |
|---|---|---|
| `snapshot_date` | Date | Collection date (injected); daily snapshots accumulate |
| `model` | String | Model / usage-detail label |
| `amount` | Float64 | Spend amount over the queried window |

`unique_key = {tenant}-{source}-{snapshot_date}-{model}`. Window = `[start_date or -30d, today]`; billing-cycle (14th) alignment is deferred. Requires `chatgpt_org_id`; 403/404 ignored when absent.

### `chatgpt_team_subscription_balance` — Current subscription balance

| Field | Type | Description |
|---|---|---|
| `snapshot_date` | Date | Collection date (injected) |
| `current_balance` | Float64 | Current balance for the cycle |

`unique_key = {tenant}-{source}-{snapshot_date}`. Requires `chatgpt_org_id`; 403/404 ignored when absent.

### `chatgpt_team_collection_runs` — Connector execution log

> **Not implemented in Phase 1.** A declarative Airbyte source cannot emit a connector-generated run-log stream — run/observability data comes from Airbyte job history instead. Schema kept for reference / a possible future custom component.

| Field | Type | Description |
|---|---|---|
| `run_id` | String | Unique run identifier |
| `started_at` | DateTime64(3) | Run start time |
| `completed_at` | DateTime64(3) | Run end time |
| `status` | String | `running` / `completed` / `failed` |
| `seats_collected` | Float64 | Rows collected for `chatgpt_team_seats` |
| `chat_records_collected` | Float64 | Rows for `chatgpt_team_chat_activity` |
| `codex_records_collected` | Float64 | Rows for `chatgpt_team_codex_user_daily` |
| `requests` | Float64 | Proxy requests made |
| `errors` | Float64 | Errors encountered |
| `settings` | String (JSON) | Collection configuration (account/org id, window) |

Monitoring table — not an analytics source.

---

## 6. Identity Resolution

`email` in the seat, chat, and Codex tables is the primary identity key — resolved to canonical `person_id` via the Identity Manager in Silver step 2. `user_id` / `account_id` are OpenAI-internal and **not** used for cross-system resolution.

---

## 7. Silver / Gold Mappings

| Bronze table | Silver target | Status |
|---|---|---|
| `chatgpt_team_seats` | *(seat roster reference)* | Available — feeds utilization context |
| `chatgpt_team_chat_activity` | `class_ai_assistant_usage` | Implemented — `chatgpt_team__ai_assistant_usage` (tool='chatgpt', surface='chat') |
| `chatgpt_team_codex_user_daily` | `class_ai_dev_usage` | Implemented — `chatgpt_team__ai_dev_usage` (tool='codex') |
| `chatgpt_team_subscription_usage` / `_balance` | *(spend reference)* | Planned — billing context, no unified stream yet |

Bronze→RMT promotion + `class_ai_*` staging follow the `claude-team` dbt pattern (`promote_bronze_to_rmt` + `ReplacingMergeTree(_version)` + `unique_key`, tagged `chatgpt-team`).

**Gold**: AI tool adoption metrics — active users, conversation/message volume, model & client breakdown, Codex adoption, seat utilization, subscription spend — alongside Claude sources for cross-provider analytics, joined by `person_id`.

---

## 8. Risks & Open Items

- **OQ-CGT-3 (gating)** — access-token exchange/refresh on the proxy. De-risk first; it determines proxy complexity (§3).
- **OQ-CGT-4** — `account_id` vs `org_id` derivation for a Team workspace.
- **OQ-CGT-6** — per-model chat tokens / `reasoning_tokens` availability; drop from `chatgpt_team_chat_activity` if not exposed.
- **OQ-CGT-5** — overlap of `chatgpt_team_seats` with the `openai` connector's org users; reconcile at Silver.
- **Browser-proxy fragility** (inherited from `claude-team`): Cloudflare/stealth churn, short session/token TTL, manual bootstrap, one proxy per workspace.
- **Field accuracy**: all Bronze schemas above are prototype-derived and provisional until verified on the live instance.
