# Windsurf Connector Specification

> Version 1.0 — March 2026
> Based on: `docs/CONNECTORS_REFERENCE.md` Source 9 (Windsurf)

Standalone specification for the Windsurf (AI Dev Tool) connector. Expands Source 9 in the main Connector Reference with full table schemas, identity mapping, Silver/Gold pipeline notes, and open questions.

<!-- toc -->

- [Overview](#overview)
- [Bronze Tables](#bronze-tables)
  - [`windsurf_daily_usage` — Daily aggregated usage per user](#windsurfdailyusage-daily-aggregated-usage-per-user)
  - [`windsurf_events` — Individual AI invocation events](#windsurfevents-individual-ai-invocation-events)
  - [`windsurf_collection_runs` — Connector execution log](#windsurfcollectionruns-connector-execution-log)
- [Identity Resolution](#identity-resolution)
- [Silver / Gold Mappings](#silver-gold-mappings)
- [Open Questions](#open-questions)
  - [OQ-WS-1: Windsurf vs Cursor feature mapping for Silver unification](#oq-ws-1-windsurf-vs-cursor-feature-mapping-for-silver-unification)
  - [OQ-WS-2: Token fields nullable conditions](#oq-ws-2-token-fields-nullable-conditions)

<!-- /toc -->

---

## Overview

**API**: Windsurf / Codeium Admin API (team dashboard)

**Category**: AI Dev Tool

**Authentication**: API key (Windsurf team account)

**Identity**: `email` in `windsurf_daily_usage` and `user_email` in `windsurf_events` — resolved to canonical `person_id` via Identity Manager.

**Field naming**: snake_case — Windsurf API uses Python-style snake_case; preserved as-is at Bronze level (contrast with Cursor which uses camelCase).

**Why two tables**: Same logical model as Cursor — daily aggregates + individual invocation events. Windsurf does not require a separate token usage table because token fields are included inline in `windsurf_events` (all nullable when absent).

**Key differences from Cursor:**

| Aspect | Cursor | Windsurf |
|--------|--------|----------|
| Primary AI feature | Chat, Composer, Agent (separate surfaces) | Cascade (unified chat + agent) |
| Completion type | Standard tab completion | Standard + Supercomplete |
| Token usage | Separate `cursor_events_token_usage` table | Inline in `windsurf_events` (nullable) |
| Field naming | camelCase | snake_case |
| API billing tracking | `subscriptionIncludedReqs`, `usageBasedReqs`, `apiKeyReqs` | `subscription_included_reqs`, `usage_based_reqs` |

---

## Bronze Tables

### `windsurf_daily_usage` — Daily aggregated usage per user

| Field | Type | Description |
|-------|------|-------------|
| `email` | String | User email — identity key |
| `user_id` | String | Windsurf / Codeium platform user ID |
| `date` | Date | Activity date |
| `is_active` | Bool | Whether user had any activity this day |
| `completions_shown` | Float64 | AI completion suggestions shown |
| `completions_accepted` | Float64 | Suggestions accepted (tab) |
| `supercomplete_shown` | Float64 | Supercomplete suggestions shown |
| `supercomplete_accepted` | Float64 | Supercomplete suggestions accepted |
| `lines_accepted` | Float64 | Lines of code accepted from AI suggestions |
| `cascade_chat_requests` | Float64 | Cascade chat interactions |
| `cascade_agent_requests` | Float64 | Cascade agent (multi-step) interactions |
| `cascade_write_actions` | Float64 | File write operations performed by Cascade agent |
| `most_used_model` | String | Most used AI model that day, e.g. `claude-3.5-sonnet` |
| `client_version` | String | Windsurf IDE version |
| `subscription_included_reqs` | Float64 | Requests covered by subscription |
| `usage_based_reqs` | Float64 | Requests on usage-based billing |

Note: `windsurf_daily_usage` has no `unique` primary key column — the natural key is `(email, date)`.

---

### `windsurf_events` — Individual AI invocation events

| Field | Type | Description |
|-------|------|-------------|
| `user_email` | String | User email — identity key |
| `event_id` | String | Unique event identifier — primary key |
| `timestamp` | DateTime64(3) | Event timestamp |
| `kind` | String | Event type: `completion`, `supercomplete`, `cascade_chat`, `cascade_agent`, etc. |
| `model` | String | AI model used, e.g. `claude-3.5-sonnet`, `gpt-4o` |
| `is_chargeable` | Bool | Whether event incurs billing |
| `request_cost` | Float64 | Request cost in credits |
| `is_token_based_call` | Bool | Billed by tokens vs per-request |
| `input_tokens` | Float64 | Tokens in the prompt (nullable) |
| `output_tokens` | Float64 | Tokens in the model response (nullable) |
| `cache_read_tokens` | Float64 | Tokens served from prompt cache (nullable) |
| `total_cents` | Float64 | Total cost in cents (nullable) |

Token fields are nullable — not all events have token-level detail. Unlike Cursor, these are included inline rather than in a separate table.

---

### `windsurf_collection_runs` — Connector execution log

| Field | Type | Description |
|-------|------|-------------|
| `run_id` | String | Unique run identifier |
| `started_at` | DateTime64(3) | Run start time |
| `completed_at` | DateTime64(3) | Run end time |
| `status` | String | `running` / `completed` / `failed` |
| `daily_usage_records_collected` | Float64 | Rows collected for `windsurf_daily_usage` |
| `events_collected` | Float64 | Rows collected for `windsurf_events` |
| `api_calls` | Float64 | API calls made |
| `errors` | Float64 | Errors encountered |
| `settings` | String | Collection configuration (team, lookback period) |

Monitoring table — not an analytics source.

---

## Identity Resolution

`email` in `windsurf_daily_usage` and `user_email` in `windsurf_events` are the primary identity keys — resolved to canonical `person_id` via Identity Manager in Silver step 2.

`user_id` (Windsurf/Codeium platform user ID) is Windsurf-internal and not used for cross-system resolution.

---

## Silver / Gold Mappings

| Bronze table | Silver target | Status |
|-------------|--------------|--------|
| `windsurf_daily_usage` | `class_ai_dev_usage` | Planned — stream not yet defined |
| `windsurf_events` | `class_ai_dev_usage` | Planned — event-level detail |

**Gold**: Same as Cursor — AI dev tool Gold metrics derived from `class_ai_dev_usage` once the unified stream is defined.

---

## Open Questions

### OQ-WS-1: Windsurf vs Cursor feature mapping for Silver unification

Windsurf's Cascade (unified chat + agent) maps to multiple Cursor features (Chat, Composer, Agent). When building a unified `class_ai_dev_usage`:

- Does `cascade_chat_requests` map to Cursor's `chatRequests + composerRequests`?
- Does `cascade_agent_requests` map to Cursor's `agentRequests`?
- How are `supercomplete_shown` / `supercomplete_accepted` mapped to Cursor's tab completion metrics?

### OQ-WS-2: Token fields nullable conditions

`windsurf_events` inline token fields are nullable. As with Cursor, the conditions are unclear:

- Are NULL token fields for per-request billed events, or for specific model types?
- Should Silver treat NULL token fields as zero for cost aggregation or exclude those rows?
