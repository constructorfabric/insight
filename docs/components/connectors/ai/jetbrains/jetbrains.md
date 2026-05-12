# JetBrains AI Assistant Connector Specification

> Version 1.0 — March 2026
> Based on: JetBrains AI Enterprise admin API (organization-level usage analytics)

Standalone specification for the JetBrains AI Assistant (AI Dev Tool) connector. Documents the enterprise data collection approach, Bronze table schemas, identity mapping, Silver/Gold pipeline notes, and open questions.

<!-- toc -->

- [Overview](#overview)
- [Data Collection Approach](#data-collection-approach)
  - [Approach Comparison](#approach-comparison)
  - [Chosen Approach: JetBrains AI Enterprise Admin API](#chosen-approach-jetbrains-ai-enterprise-admin-api)
- [Bronze Tables](#bronze-tables)
  - [`jetbrains_activity` — Daily aggregated AI usage per user](#jetbrainsactivity-daily-aggregated-ai-usage-per-user)
  - [`jetbrains_users` — User directory](#jetbrainsusers-user-directory)
  - [`jetbrains_collection_runs` — Connector execution log](#jetbrainscollectionruns-connector-execution-log)
- [Identity Resolution](#identity-resolution)
- [Silver / Gold Mappings](#silver-gold-mappings)
- [Open Questions](#open-questions)
  - [OQ-JB-1: JetBrains AI Assistant vs bare IDE — centralized API availability](#oq-jb-1-jetbrains-ai-assistant-vs-bare-ide-centralized-api-availability)
  - [OQ-JB-2: JetBrains AI Enterprise admin API — endpoint stability and scope](#oq-jb-2-jetbrains-ai-enterprise-admin-api-endpoint-stability-and-scope)
  - [OQ-JB-3: Silver feature mapping against Cursor / Windsurf metrics](#oq-jb-3-silver-feature-mapping-against-cursor-windsurf-metrics)

<!-- /toc -->

---

## Overview

**API**: JetBrains AI Enterprise admin API (organization usage analytics)

**data_source**: `insight_jetbrains`

**Category**: AI Dev Tool

**Authentication**: JetBrains AI Enterprise organization admin credentials (API key or OAuth token issued via the JetBrains AI Enterprise admin panel). Standard license server and Toolbox App do not provide usage analytics APIs and are out of scope.

**Identity**: `email` in `jetbrains_users` and `jetbrains_activity` — resolved to canonical `person_id` via Identity Manager.

**Field naming**: snake_case — consistent with other AI dev connectors in this category.

**Why two entity tables**: `jetbrains_activity` captures daily per-user AI usage metrics (completions, chat interactions, active days, models used). `jetbrains_users` is the organization user directory — separated because user attributes change independently of daily activity and must be joined at query time, not denormalized into every activity row.

**Key structural similarity to GitHub Copilot**: JetBrains AI Enterprise exposes daily aggregates per user (similar to Cursor/Windsurf) but does not provide individual invocation event logs at the same granularity as Cursor's `cursor_events`. The primary analytics unit is the daily aggregate row.

**Key difference from Cursor/Windsurf**: JetBrains AI Assistant is a plugin that runs inside any JetBrains IDE (IntelliJ IDEA, PyCharm, GoLand, WebStorm, etc.). The underlying IDE is tracked via `ide_product_code` — this enables adoption analysis by product family without requiring separate connectors per IDE.

---

## Data Collection Approach

### Approach Comparison

Three data collection approaches exist for JetBrains tooling. Only one is viable for enterprise-grade centralized analytics.

| Approach | Data Available | Requires Agent? | Centralized API? | Viable for Insight? |
|----------|---------------|-----------------|-----------------|---------------------|
| **JetBrains AI Enterprise admin API** | AI completions shown/accepted, chat messages, active days, models used, per-user daily aggregates | No | Yes | **Yes — chosen** |
| **ActivityTracker plugin (open-source)** | IDE time tracking, file edit events, keystrokes | Yes — local file per machine | No | No — no central API |
| **JetBrains Toolbox App REST API (local)** | Installed IDEs, last-used timestamps | Yes — local daemon per machine | No | No — no central API |

### Chosen Approach: JetBrains AI Enterprise Admin API

The JetBrains AI Enterprise admin panel provides an organization-level REST API for usage analytics. This is the only approach that supports:

- Centralized data collection without agents on developer machines
- Per-user daily usage metrics consistent with `cursor_daily_usage` and `windsurf_daily_usage`
- User directory with email-based identity resolution

**Prerequisite**: The organization must be subscribed to JetBrains AI Enterprise (not individual or team plans). Without this subscription, no centralized usage API is available and centralized collection is not possible.

---

## Bronze Tables

### `jetbrains_activity` — Daily aggregated AI usage per user

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | String | JetBrains account user ID — primary key fragment |
| `email` | String | User email — identity key |
| `date` | Date | Activity date — primary key fragment (composite PK: `user_id` + `date`) |
| `is_active` | Bool | Whether user had any AI activity this day |
| `completions_shown` | Float64 | AI inline completion suggestions shown |
| `completions_accepted` | Float64 | Inline completion suggestions accepted |
| `chat_messages` | Float64 | AI chat messages sent (JetBrains AI chat panel) |
| `chat_sessions` | Float64 | Distinct chat sessions started |
| `active_days_in_period` | Float64 | Rolling active days (may be provided as a period aggregate) |
| `most_used_model` | String | Most used AI model that day, e.g. `gpt-4o`, `claude-3.5-sonnet` |
| `models_used` | String | JSON array of distinct model identifiers used that day |
| `ide_product_code` | String | JetBrains IDE product code, e.g. `IU` (IntelliJ IDEA Ultimate), `PY` (PyCharm), `GO` (GoLand), `WS` (WebStorm) |
| `ide_version` | String | IDE build version, e.g. `243.22562.218` |
| `plugin_version` | String | JetBrains AI Assistant plugin version |
| `metadata` | String | Full API response (String (JSON)) |

Natural key is `(user_id, date)`. No surrogate primary key — consistent with `windsurf_daily_usage` pattern where natural key is used.

---

### `jetbrains_users` — User directory

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | String | JetBrains account user ID — primary key |
| `email` | String | User email — identity key |
| `display_name` | String | User display name |
| `username` | String | JetBrains account username (login) |
| `role` | String | Organization role: `member` / `admin` |
| `status` | String | Account status: `active` / `inactive` / `pending` |
| `created_at` | DateTime64(3) | When the user was added to the organization |
| `last_seen_at` | DateTime64(3) | Last recorded activity timestamp (any JetBrains service) |
| `ingestion_at` | DateTime64(3) | When this row was collected — cursor for incremental sync |
| `metadata` | String | Full API response (String (JSON)) |

---

### `jetbrains_collection_runs` — Connector execution log

| Field | Type | Description |
|-------|------|-------------|
| `run_id` | String | Unique run identifier |
| `started_at` | DateTime64(3) | Run start time |
| `completed_at` | DateTime64(3) | Run end time |
| `status` | String | `running` / `completed` / `failed` |
| `activity_records_collected` | Float64 | Rows collected for `jetbrains_activity` |
| `users_collected` | Float64 | Rows collected for `jetbrains_users` |
| `api_calls` | Float64 | API calls made |
| `errors` | Float64 | Errors encountered |
| `settings` | String | Collection configuration (org ID, lookback period) (String (JSON)) |

Monitoring table — not an analytics source.

---

## Identity Resolution

`email` in both `jetbrains_activity` and `jetbrains_users` is the primary identity key — resolved to canonical `person_id` via Identity Manager in Silver step 2.

`user_id` (JetBrains account ID) is JetBrains-internal. It is preserved for deduplication within the Bronze layer but is not used as the primary key for cross-system identity resolution.

`username` in `jetbrains_users` is a secondary identifier and is not used for cross-system resolution.

---

## Silver / Gold Mappings

| Bronze table | Silver target | Status |
|-------------|--------------|--------|
| `jetbrains_activity` | `class_ai_dev_usage` | Planned — daily aggregate rows, feeds unified AI dev stream |
| `jetbrains_users` | *(identity enrichment only)* | Available — email feeds Identity Manager; no separate Silver target |

**Silver field mapping notes**: `completions_shown` → unified `suggestions_shown`; `completions_accepted` → unified `suggestions_accepted`; `chat_messages` → unified `chat_requests`. `ide_product_code` and `ide_version` are JetBrains-specific attributes — carried as extended fields in `class_ai_dev_usage` or dropped to Gold metadata, pending unified schema decision.

**Gold**: AI dev tool Gold metrics (acceptance rate, chat engagement, active users, model distribution) derived from `class_ai_dev_usage` alongside Cursor, Windsurf, and GitHub Copilot. `ide_product_code` enables a JetBrains-specific breakdown by IDE family (e.g., IntelliJ vs PyCharm adoption of AI Assistant).

---

## Open Questions

### OQ-JB-1: JetBrains AI Assistant vs bare IDE — centralized API availability

The JetBrains AI Enterprise admin API is only available when the organization uses JetBrains AI Enterprise. Organizations using:

- JetBrains individual or team IDE licenses (without AI Enterprise): no centralized usage API exists.
- JetBrains AI Assistant free tier: no org-level admin panel or API.

Without AI Enterprise, the only usage signals available are local (ActivityTracker plugin files, Toolbox App local API) — neither supports centralized collection without a device agent. This connector is therefore only deployable for organizations with a JetBrains AI Enterprise subscription.

- Should the connector spec document a fallback path (e.g., ActivityTracker log ingestion via an optional agent)?
- Or should it strictly require AI Enterprise and document the prerequisite as a hard constraint?

### OQ-JB-2: JetBrains AI Enterprise admin API — endpoint stability and scope

The JetBrains AI Enterprise admin API is not yet publicly documented at the same level of detail as Cursor or Windsurf APIs. The field list in `jetbrains_activity` is based on publicly available information and the structure of comparable admin APIs.

- Are `chat_sessions`, `models_used`, and `plugin_version` actually exposed by the API, or must they be inferred from other fields?
- Does the API provide per-invocation event logs (similar to `cursor_events`) or only daily aggregates?
- What is the API's lookback window — can historical data be backfilled beyond 30 days?

These fields must be validated against the actual API contract before Bronze schema is considered final.

### OQ-JB-3: Silver feature mapping against Cursor / Windsurf metrics

JetBrains AI Assistant's feature surface (inline completions + chat) maps more directly to Windsurf than to Cursor, but `models_used` (JSON array per day) differs from Cursor's `mostUsedModel` (single value). When building `class_ai_dev_usage`:

- How should the `models_used` array be normalized for Silver? Explode to one row per model per day, or keep as aggregate JSON?
- Should `ide_product_code` (JetBrains-specific) be carried into `class_ai_dev_usage` as an optional IDE-dimension column, or stored only in Bronze?
- Cursor and Windsurf expose per-request billing fields (`subscription_included_reqs`, `usage_based_reqs`). JetBrains AI Enterprise may not — how are NULL billing fields handled in the unified stream?
