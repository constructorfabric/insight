# PRD — ChatGPT Team Connector

> Version 2.0 — June 2026
> Supersedes: v1.0 (March 2026) — data source corrected from the OpenAI Admin API to a customer-hosted browser proxy over `chatgpt.com`. See [ADR-001](./ADR/ADR-001-browser-proxy-architecture.md).
> Based on: `docs/CONNECTORS_REFERENCE.md` Source 19 (ChatGPT Team); reference connector `claude-team`.

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals](#13-goals)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Scope](#3-scope)
  - [3.1 In Scope](#31-in-scope)
  - [3.2 Out of Scope](#32-out-of-scope)
- [4. Functional Requirements](#4-functional-requirements)
  - [4.1 Seat Data Collection](#41-seat-data-collection)
  - [4.2 Chat Activity Data Collection](#42-chat-activity-data-collection)
  - [4.3 Codex Activity Data Collection](#43-codex-activity-data-collection)
  - [4.4 Subscription / Billing Collection](#44-subscription--billing-collection)
  - [4.5 Transport & Security](#45-transport--security)
  - [4.6 Identity Resolution](#46-identity-resolution)
  - [4.7 Silver / Gold Pipeline](#47-silver--gold-pipeline)
- [5. Non-Functional Requirements](#5-non-functional-requirements)
- [6. Open Questions](#6-open-questions)

<!-- /toc -->

---

## 1. Overview

### 1.1 Purpose

The ChatGPT Team connector collects seat assignments and daily AI-tool usage from an organization's **ChatGPT Team/Enterprise workspace** (`chatgpt.com`). It enables workspace admins and analytics teams to track AI assistant adoption, seat utilization, conversational usage, Codex (coding) usage, and subscription spend — unified with other AI tool sources (e.g. Claude Team) for cross-provider reporting.

### 1.2 Background / Problem Statement

Organizations running ChatGPT Team subscriptions have no centralized visibility into who actively uses the workspace, how usage splits across chat vs Codex, and how subscription spend accrues. This data is exposed only by the authenticated `chatgpt.com/backend-api/*` web endpoints — there is **no public API** for it, and the surface sits behind a browser session cookie plus Cloudflare bot protection.

This connector therefore does **not** use the OpenAI Admin API. The Admin API (`api.openai.com/v1/organization/*`) exposes only the *programmatic* surface (org users, projects, API keys, per-key/per-model completions usage, costs, audit logs); that surface is already owned by the separate `openai` connector and feeds `class_ai_api_usage`. ChatGPT Team covers a **different billing model** (flat per-seat), **different clients** (`web`, `desktop`, `mobile`, Codex surfaces), and a **different analytics purpose** (adoption, not pay-per-token API spend).

To reach `chatgpt.com` data without placing the customer's session cookie inside Insight infrastructure, the connector mirrors the `claude-team` architecture: a **customer-hosted browser proxy** holds the session and clears Cloudflare; Insight talks to the proxy over HTTP with a shared bearer token. See [ADR-001](./ADR/ADR-001-browser-proxy-architecture.md).

### 1.3 Goals

- Collect the complete seat roster and daily per-user chat, Codex, and subscription data from the ChatGPT Team workspace via the customer-hosted proxy.
- Resolve user identity (`email`) to canonical `person_id` for cross-system analytics.
- Feed `class_ai_assistant_usage` (conversational) and `class_ai_dev_usage` (Codex/coding) Silver streams, unified with the equivalent Claude sources.
- Enable Gold-level metrics: active users, conversation/message volume, model & client breakdown, Codex adoption, seat utilization, subscription spend.
- Keep the customer's `chatgpt.com` session out of Insight infrastructure.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Seat | An assigned ChatGPT Team subscription slot for a specific user |
| Chat activity | Daily per-user conversational usage: messages, sessions, by client/seat type |
| Codex activity | Daily per-user coding usage (Codex CLI/IDE/web/exec): credits, threads, turns, tokens, lines |
| `claude-team` | Reference connector using the same browser-proxy architecture |
| browser proxy | Customer-deployed headless-Chromium service (`secure-enclave/proxies/chatgpt_team`) that holds the `chatgpt.com` session and exposes `/api/*` to Insight |
| `session cookie` | The `chatgpt.com` browser session held **only** on the proxy; never seen by Insight |
| `access_token` | Short-lived bearer token derived from the session (via `GET /api/auth/session`) that some `backend-api` endpoints require |
| `account_id` / `org_id` | Two distinct ChatGPT identifiers used by different `backend-api` endpoints |
| `person_id` | Canonical cross-system person identifier resolved by the Identity Manager |
| `class_ai_assistant_usage` | Silver stream for conversational AI tool usage (ChatGPT Team chat + Claude web/mobile) |
| `class_ai_dev_usage` | Silver stream for IDE/coding AI usage (ChatGPT Codex + Claude Code + Cursor) |
| `class_ai_api_usage` | Silver stream for programmatic API usage — distinct; owned by the `openai` connector |

---

## 2. Actors

### 2.1 Human Actors

#### Workspace Administrator

**ID**: `cpt-insightspec-actor-chatgpt-team-admin`

**Role**: Manages the ChatGPT Team subscription, grants/revokes seat access, monitors usage.
**Needs**: Visibility into seat utilization, inactive seats, and adoption trends across chat and Codex.

#### Analytics Engineer

**ID**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

**Role**: Designs and maintains the Silver/Gold pipeline consuming ChatGPT Team Bronze data.
**Needs**: Reliable, schema-stable Bronze tables with consistent identity fields for joining with other sources.

#### Proxy Operator (Customer)

**ID**: `cpt-insightspec-actor-chatgpt-team-proxy-operator`

**Role**: Deploys and operates the customer-hosted browser proxy; installs and rotates the `chatgpt.com` session.
**Needs**: A simple, documented way to bootstrap and refresh the session; clear signals when the session expires.

### 2.2 System Actors

#### ChatGPT Team Web API (via customer-hosted proxy)

**ID**: `cpt-insightspec-actor-chatgpt-team-web-api`

**Role**: Source of seat, chat, Codex, and subscription data. The `chatgpt.com/backend-api/*` endpoints, reached through the customer-hosted browser proxy. **Not** the OpenAI Admin API.

#### Identity Manager

**ID**: `cpt-insightspec-actor-chatgpt-team-identity-mgr`

**Role**: Resolves `email` from Bronze tables to canonical `person_id` used in Silver/Gold layers.

---

## 3. Scope

### 3.1 In Scope

- Current seat assignments (who has a seat, role, status, activity timestamps) from the workspace.
- Daily per-user **chat** activity (messages/sessions, by client/seat type).
- Daily per-user **Codex** activity (credits, threads, turns, tokens, lines accepted).
- **Subscription** usage (per-model spend) and balance for the workspace billing cycle.
- Connector execution logging for monitoring and observability.
- Identity resolution of `email` → `person_id` in the Silver step.
- Feeding `class_ai_assistant_usage` (chat) and `class_ai_dev_usage` (Codex) Silver streams.
- A customer-hosted browser proxy as the transport (deployment owned by the customer; see DESIGN).

### 3.2 Out of Scope

- Programmatic OpenAI API usage (org users, projects, API keys, per-key completions, costs, audit logs) — owned by the `openai` connector (`class_ai_api_usage`).
- Real-time or sub-daily granularity — only daily aggregates are exposed.
- Historical backfill beyond the earliest date the `backend-api` endpoints return data.
- Versioning/history of seat assignment changes (current-state snapshot only).
- Conversation content or prompt/response text — only aggregate counts.

---

## 4. Functional Requirements

### 4.1 Seat Data Collection

#### Collect seat roster

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-seats-collect`

The connector **MUST** collect all current seat assignments from `chatgpt.com/backend-api/accounts/{account_id}/users` (via the proxy), capturing each user's identifier, email, role, status, and activity timestamps.

**Rationale**: Seat roster is the foundation for utilization reporting — without it, activity cannot be attributed to provisioned users.
**Actors**: `cpt-insightspec-actor-chatgpt-team-admin`, `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Represent seat data as current-state snapshot

- [ ] `p2` - **ID**: `cpt-insightspec-fr-chatgpt-team-seats-snapshot`

The seat collection **MUST** represent current-state only (one row per user, no historical versioning), consistent with the source's snapshot model.

**Rationale**: The source does not provide seat change history; the Bronze table must accurately reflect its capabilities.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

### 4.2 Chat Activity Data Collection

#### Collect daily chat activity

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-activity-collect`

The connector **MUST** collect daily per-user chat activity from the workspace analytics endpoints (e.g. `/backend-api/accounts/{account_id}/analytics/user_list`, `/backend-api/wham/analytics/daily-sessions-messages-counts`), capturing message and session counts per day per user, including available breakdowns (e.g. GPT / tool / connector / project messages) and credits used.

**Rationale**: Daily chat activity is the primary adoption signal — frequency of use and message volume per user.
**Actors**: `cpt-insightspec-actor-chatgpt-team-admin`, `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Walk a configurable backfill window per day

- [ ] `p2` - **ID**: `cpt-insightspec-fr-chatgpt-team-activity-backfill`

Daily activity collection **MUST** support an incremental cursor over `date` with a configurable start, walking one day (or one supported window) per request, and **MUST** stop at the earliest date the endpoint returns data.

**Rationale**: The endpoints require explicit date ranges and return empty before the earliest data point; a date cursor enables both backfill and steady-state incremental syncs (same pattern as `claude_team_code_metrics`).
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Log connector execution

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-collection-runs`

The connector **MUST** record each execution run with start/end time, status, per-stream record counts, request count, and error count.

**Rationale**: Execution logs are required for monitoring data freshness, diagnosing failures, and auditing pipeline health.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

### 4.3 Codex Activity Data Collection

#### Collect daily Codex usage per user

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-codex-collect`

The connector **MUST** collect daily per-user Codex usage from the workspace usage endpoints (e.g. `/backend-api/wham/analytics/usage-leaderboard`), capturing credits, threads, turns, tokens, and lines accepted, attributed by `email`.

**Rationale**: Codex (coding) usage is a distinct adoption signal that maps to `class_ai_dev_usage` alongside Claude Code and Cursor.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Normalize Codex client surface

- [ ] `p2` - **ID**: `cpt-insightspec-fr-chatgpt-team-codex-surface`

Codex collection **SHOULD** normalize the raw client surface enum (e.g. `CODEX_CLI` → `cli`, `CODEX_IDE_VSCODE` → `vscode`) into a stable, documented set of values at Silver time, preserving the raw value in Bronze.

**Rationale**: Stable surface values enable consistent client-breakdown analytics; preserving the raw value avoids data loss on enum changes.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

### 4.4 Subscription / Billing Collection

#### Collect subscription usage and balance

- [ ] `p2` - **ID**: `cpt-insightspec-fr-chatgpt-team-subscription-collect`

The connector **MUST** collect workspace subscription usage (per-model spend) and current balance for the billing cycle from `/backend-api/subscriptions/{org_id}/usage`.

**Rationale**: Subscription spend complements seat adoption with cost context for the flat-seat plan; it is not available from the per-key `openai` Admin-API cost stream.
**Actors**: `cpt-insightspec-actor-chatgpt-team-admin`, `cpt-insightspec-actor-chatgpt-team-analytics-eng`

### 4.5 Transport & Security

#### Read all data through the customer-hosted proxy

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-proxy-transport`

The connector **MUST** read all data from the customer-hosted browser proxy over HTTP, authenticating with a shared bearer `proxy_auth_token`. The connector **MUST NOT** call `chatgpt.com` directly and **MUST NOT** hold or receive the `chatgpt.com` session cookie or derived access token.

**Rationale**: Keeping the session on the customer side is the core security property of this architecture (ADR-001); it also satisfies Cloudflare, which only a real browser clears.
**Actors**: `cpt-insightspec-actor-chatgpt-team-proxy-operator`, `cpt-insightspec-actor-chatgpt-team-web-api`

### 4.6 Identity Resolution

#### Resolve email to person_id

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-identity-resolve`

The Silver pipeline **MUST** resolve `email` from the seat and activity Bronze tables to a canonical `person_id` via the Identity Manager.

**Rationale**: Cross-system analytics (joining AI tool usage with HR or task-tracker data) requires a stable, source-independent person identifier.
**Actors**: `cpt-insightspec-actor-chatgpt-team-identity-mgr`, `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Use email as the sole identity key

- [ ] `p2` - **ID**: `cpt-insightspec-fr-chatgpt-team-identity-key`

The connector **MUST** treat `email` as the primary identity key for resolution. The ChatGPT-internal `user_id`/`account_id` **MUST NOT** be used for cross-system identity resolution.

**Rationale**: Internal IDs are not meaningful outside the OpenAI ecosystem; email is the stable cross-system key.
**Actors**: `cpt-insightspec-actor-chatgpt-team-identity-mgr`

### 4.7 Silver / Gold Pipeline

#### Feed class_ai_assistant_usage and class_ai_dev_usage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-silver-tool-usage`

Daily chat activity **MUST** feed `class_ai_assistant_usage` (unified with Claude web/mobile), and daily Codex activity **MUST** feed `class_ai_dev_usage` (unified with Claude Code / Cursor).

**Rationale**: Unified adoption analytics require single Silver streams per usage class spanning all providers.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Keep tool/dev usage separate from API usage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-silver-separation`

ChatGPT Team conversational and Codex usage **MUST NOT** be merged into `class_ai_api_usage` (the `openai` connector's programmatic stream). Cross-stream analysis **MUST** be performed at Gold level using `person_id`.

**Rationale**: Conversational (flat-seat), coding (Codex), and programmatic API usage serve distinct purposes and have incompatible schemas.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

---

## 5. Non-Functional Requirements

#### Data freshness

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-chatgpt-team-freshness`

The connector **MUST** be runnable on a daily schedule such that activity for day D is available by the start of day D+2.

**Threshold**: ≤ 48 hours end-to-end latency from activity to Bronze availability.
**Rationale**: Daily adoption reports require timely data; 48h accommodates source reporting lag.

#### Schema stability

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-chatgpt-team-schema-stability`

Bronze table schemas **MUST** remain stable across connector versions; breaking changes **MUST** be versioned with migration guidance.

**Threshold**: Zero unannounced breaking changes to field names/types in the Bronze tables.
**Rationale**: Downstream Silver/Gold pipelines depend on stable Bronze schemas.

#### Session isolation

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-chatgpt-team-session-isolation`

The `chatgpt.com` session cookie and any derived access token **MUST** reside only in the customer-hosted proxy and **MUST NOT** be transmitted to, stored by, or logged by Insight.

**Threshold**: No Insight component or log contains the session cookie or access token; Insight holds only `proxy_url` + `proxy_auth_token`.
**Rationale**: Core security property of the architecture (ADR-001).

#### Graceful degradation on permission/auth gaps

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-chatgpt-team-graceful-degradation`

If a stream's endpoint is unavailable for the current session (e.g. missing permission, or an expired access token mid-run), that stream **MUST** be skipped without failing the whole sync, mirroring `claude-team`'s 403-tolerant handling.

**Threshold**: A single failing/forbidden stream leaves other streams' data intact and the run status actionable.
**Rationale**: Sessions and permissions vary; partial data beats a fully failed run.

---

## 6. Open Questions

### OQ-CGT-1: ChatGPT Team vs OpenAI API for the same user

**Status**: CLOSED. `class_ai_assistant_usage`/`class_ai_dev_usage` (this connector) and `class_ai_api_usage` (the `openai` connector) are separate Silver streams; cross-stream analysis by `person_id` is done at Gold.

### OQ-CGT-2: Unified Silver schema — explicit columns vs `extras`

ChatGPT Team and Claude sources have similar but not identical activity shapes. Should source-specific fields (OpenAI `reasoning_tokens`; Claude `tool_use_count`, `cache_*_tokens`) be explicit nullable columns or a JSON `extras` blob?

**Status**: OPEN. Decide during Silver design; Bronze keeps raw per-source fields regardless.

### OQ-CGT-3: Access-token exchange and TTL (de-risk first)

Does `chatgpt.com/backend-api/*` for the target endpoints require the bearer `access_token` in addition to the session cookie, and what is its TTL/refresh path (`GET /api/auth/session`)? This determines whether the proxy must perform token exchange + periodic refresh or can rely on the session cookie alone (as `claude_team` does).

**Status**: OPEN — first de-risking task; gates the proxy design.

### OQ-CGT-4: `account_id` vs `org_id`

Different `backend-api` endpoints require `account_id` vs `org_id`. Are both always derivable for a Team workspace, and should the connector accept both as config or derive one from the other?

**Status**: OPEN — confirm against the live instance during de-risking.

### OQ-CGT-5: Seat roster overlap with the `openai` connector

The `openai` Admin-API connector collects org users. ChatGPT Team seats are a different hierarchy (workspace account users). Confirm they are distinct enough to avoid double counting, or define how they reconcile at Silver.

**Status**: OPEN.

### OQ-CGT-6: Per-model / reasoning-token availability

v1.0 assumed per-model token breakdown (incl. `reasoning_tokens`) for chat activity. The browser analytics endpoints observed in the prototype expose message/session counts and credits, not per-model chat token splits. Confirm whether any browser endpoint provides per-model chat tokens; otherwise drop those fields from `chatgpt_team_chat_activity`.

**Status**: OPEN — verify on the live instance.
