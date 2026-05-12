# Cursor Connector Specification

> Version 1.0 — March 2026
> Based on: `docs/CONNECTORS_REFERENCE.md` Source 8 (Cursor)

Standalone specification for the Cursor (AI Dev Tool) connector. Expands Source 8 in the main Connector Reference with full table schemas, identity mapping, Silver/Gold pipeline notes, and open questions.

<!-- toc -->

- [Overview](#overview)
- [Bronze Tables](#bronze-tables)
  - [`cursor_daily_usage` — Daily aggregated usage per user](#cursordailyusage-daily-aggregated-usage-per-user)
  - [`cursor_events` — Individual AI invocation events](#cursorevents-individual-ai-invocation-events)
  - [`cursor_events_token_usage` — Token consumption per event (1:1 with cursor_events)](#cursoreventstokenusage-token-consumption-per-event-11-with-cursorevents)
  - [`cursor_collection_runs` — Connector execution log](#cursorcollectionruns-connector-execution-log)
- [Identity Resolution](#identity-resolution)
- [Silver / Gold Mappings](#silver-gold-mappings)
- [Open Questions](#open-questions)
  - [OQ-CUR-1: Silver stream for AI dev tool usage](#oq-cur-1-silver-stream-for-ai-dev-tool-usage)
  - [OQ-CUR-2: `cursor_events_token_usage` — when is token detail absent?](#oq-cur-2-cursoreventstokenusage-when-is-token-detail-absent)

<!-- /toc -->

---

## Overview

**API**: Cursor Admin API (enterprise/team dashboard)

**Category**: AI Dev Tool

**Authentication**: API key (Cursor team account)

**Identity**: `email` in `cursor_daily_usage` and `userEmail` in `cursor_events` — resolved to canonical `person_id` via Identity Manager.

**Field naming**: camelCase — Cursor API uses JavaScript-style camelCase; preserved as-is at Bronze level.

**Why three tables**: `cursor_daily_usage` and `cursor_events` represent different granularities — daily aggregates vs individual AI invocations. `cursor_events_token_usage` is 1:1 with `cursor_events` but kept separate to allow NULL-free storage when token detail is absent (not all events have token-level breakdown).

---

## Bronze Tables

### `cursor_daily_usage` — Daily aggregated usage per user

| Field | Type | Description |
|-------|------|-------------|
| `unique` | String | Primary key |
| `day` | String | Day label |
| `date` | Int64 | Unix timestamp in milliseconds |
| `email` | String | User email — identity key |
| `userId` | String | Cursor platform user ID |
| `isActive` | Bool | Whether user had any activity this day |
| `chatRequests` | Float64 | AI chat interactions |
| `cmdkUsages` | Float64 | Cmd+K (inline edit) usages |
| `composerRequests` | Float64 | Composer feature requests |
| `agentRequests` | Float64 | Agent mode requests |
| `bugbotUsages` | Float64 | Bug bot usages |
| `totalTabsShown` | Float64 | Tab completion suggestions shown |
| `totalTabsAccepted` | Float64 | Tab completions accepted |
| `totalAccepts` | Float64 | All AI suggestions accepted |
| `totalApplies` | Float64 | Code applications (apply to file) |
| `totalRejects` | Float64 | Suggestions rejected |
| `totalLinesAdded` | Float64 | Total lines of code added |
| `totalLinesDeleted` | Float64 | Total lines deleted |
| `acceptedLinesAdded` | Float64 | Lines added from accepted AI suggestions |
| `acceptedLinesDeleted` | Float64 | Lines deleted from accepted AI suggestions |
| `mostUsedModel` | String | Most used AI model that day, e.g. `claude-3.5-sonnet` |
| `tabMostUsedExtension` | String | File extension with most tab completions |
| `applyMostUsedExtension` | String | File extension with most applies |
| `clientVersion` | String | Cursor IDE version |
| `subscriptionIncludedReqs` | Float64 | Requests covered by subscription |
| `usageBasedReqs` | Float64 | Requests on usage-based billing |
| `apiKeyReqs` | Float64 | Requests using API key |

---

### `cursor_events` — Individual AI invocation events

| Field | Type | Description |
|-------|------|-------------|
| `unique` | String | Primary key |
| `userEmail` | String | User email — identity key |
| `timestamp` | DateTime64(3) | Event timestamp |
| `kind` | String | Event type: `chat`, `completion`, `agent`, `cmd-k`, etc. |
| `model` | String | AI model used, e.g. `gpt-4o`, `claude-3.5-sonnet` |
| `maxMode` | Bool | Whether max mode was enabled |
| `isChargeable` | Bool | Whether event incurs billing |
| `requestsCosts` | Float64 | Request cost in credits |
| `cursorTokenFee` | Float64 | Cursor platform fee |
| `isTokenBasedCall` | Bool | Billed by tokens vs per-request |
| `isHeadless` | Bool | Triggered without UI (automated) |

---

### `cursor_events_token_usage` — Token consumption per event (1:1 with cursor_events)

| Field | Type | Description |
|-------|------|-------------|
| `event_unique` | String | Parent event reference — joins to `cursor_events.unique` |
| `inputTokens` | Float64 | Tokens in the prompt |
| `outputTokens` | Float64 | Tokens in the model response |
| `cacheReadTokens` | Float64 | Tokens served from prompt cache |
| `cacheWriteTokens` | Float64 | Tokens written to cache |
| `totalCents` | Float64 | Total cost in cents |
| `discountPercentOff` | Float64 | Discount applied |

All fields nullable — not all events have token-level detail. This table exists as a separate entity to avoid NULLs in the main events table.

---

### `cursor_collection_runs` — Connector execution log

| Field | Type | Description |
|-------|------|-------------|
| `run_id` | String | Unique run identifier |
| `started_at` | DateTime64(3) | Run start time |
| `completed_at` | DateTime64(3) | Run end time |
| `status` | String | `running` / `completed` / `failed` |
| `daily_usage_records_collected` | Float64 | Rows collected for `cursor_daily_usage` |
| `events_collected` | Float64 | Rows collected for `cursor_events` |
| `token_records_collected` | Float64 | Rows collected for `cursor_events_token_usage` |
| `api_calls` | Float64 | API calls made |
| `errors` | Float64 | Errors encountered |
| `settings` | String | Collection configuration (team, lookback period) |

Monitoring table — not an analytics source.

---

## Identity Resolution

`email` in `cursor_daily_usage` and `userEmail` in `cursor_events` are the primary identity keys — resolved to canonical `person_id` via Identity Manager in Silver step 2.

`userId` (Cursor platform user ID) is Cursor-internal and not used for cross-system resolution.

---

## Silver / Gold Mappings

| Bronze table | Silver target | Status |
|-------------|--------------|--------|
| `cursor_daily_usage` | `class_ai_dev_usage` | Planned — stream not yet defined |
| `cursor_events` | `class_ai_dev_usage` | Planned — event-level detail |
| `cursor_events_token_usage` | *(detail table)* | Available — feeds cost analytics |

**Gold**: AI dev tool Gold metrics (acceptance rate, lines added via AI, model distribution, cost per user) will be derived from `class_ai_dev_usage` once the unified stream is defined. Cursor, Windsurf, and GitHub Copilot would all feed this stream.

---

## Open Questions

### OQ-CUR-1: Silver stream for AI dev tool usage

No `class_ai_dev_usage` (or equivalent) Silver stream is currently defined in `CONNECTORS_REFERENCE.md`. Cursor, Windsurf, and GitHub Copilot represent the same category but with different granularities:

- Cursor: per-event detail + daily aggregates + separate token table
- Windsurf: per-event (with inline token fields) + daily aggregates
- GitHub Copilot: org-level only (no per-user events)

- Should `class_ai_dev_usage` be a daily aggregate table (harmonised across all three sources)?
- Or should it be event-level for Cursor + Windsurf, with Copilot contributing only aggregate rows?

### OQ-CUR-2: `cursor_events_token_usage` — when is token detail absent?

`cursor_events_token_usage` has all-nullable fields for events without token detail. The conditions under which token data is absent are unclear:

- Is it per-request billing (flat fee, no token count)?
- Is it a Cursor API limitation for certain model types?
- Understanding this is important for Silver cost aggregation — NULL rows must not be treated as zero-cost events.
