# Zendesk Connector Specification

> Version 2.0 — June 2026
> Previous: v1.1 May 2026
> Based on: `docs/connectors/support/README.md` (Support domain schema)
> Decisions: INSIGHT-459 Phase 1 scope locked; see Resolved Questions.
> Phase 2 (Ticket Audits `support_ticket_events`) + Silver (`class_support_activity`) + Gold (`support_bullet_rows`) are now SHIPPED (INSIGHT-459). `support_collection_runs` de-scoped (declarative manifests cannot emit a connector-generated stream — monitoring via Airbyte platform job stats).

Standalone specification for the Zendesk (Support / Helpdesk) connector.

<!-- toc -->

- [Overview](#overview)
- [Phase Scope](#phase-scope)
- [Bronze Tables](#bronze-tables)
  - [`support_tickets` — Ticket metadata and current state](#support_tickets--ticket-metadata-and-current-state)
  - [`support_agents` — Agent directory](#support_agents--agent-directory)
  - [`zendesk_satisfaction_ratings` — CSAT ratings (separate stream)](#zendesk_satisfaction_ratings--csat-ratings-separate-stream)
  - [`support_collection_runs` — Connector execution log](#support_collection_runs--connector-execution-log)
  - [`support_ticket_events` — Ticket Audit log (SHIPPED)](#support_ticket_events--ticket-audit-log-shipped)
  - [Phase 2: `zendesk_ticket_ext` — Custom ticket fields](#phase-2-zendesk_ticket_ext--custom-ticket-fields)
- [Identity Resolution](#identity-resolution)
- [Silver / Gold Mappings](#silver--gold-mappings)
- [Resolved Questions](#resolved-questions)
- [Open Questions](#open-questions)
  - [OQ-ZD-4: `support_ticket_events` incremental audit collection strategy (Phase 2)](#oq-zd-4-support_ticket_events-incremental-audit-collection-strategy-phase-2)
  - [OQ-ZD-5: Business-hours-only satisfaction_score on support_tickets (Phase 2)](#oq-zd-5-business-hours-only-satisfaction_score-on-support_tickets-phase-2)

<!-- /toc -->

---

## Overview

**API**: Zendesk REST API v2 (`https://{subdomain}.zendesk.com/api/v2/`)

**Category**: Support / Helpdesk

**Authentication**:
- **API token** (preferred for service accounts): HTTP Basic Auth with `{email}/token:{api_token}` encoded as Base64. Token created under Admin → Apps & Integrations → Zendesk API.
- **OAuth 2.0**: Authorization Code flow — requires a Zendesk OAuth client. Scopes: `tickets:read`, `users:read`, `satisfaction_ratings:read`.

**Identity**: `support_agents.email` — resolved to canonical `person_id` via Identity Manager. Zendesk `user.id` (numeric) is Zendesk-internal; `email` is the cross-system key.

**`data_source`**: `"insight_zendesk"` — used as the source discriminator in all unified Bronze tables.

**`insight_source_id`**: set to the Zendesk subdomain slug, e.g. `zendesk-acme`. Required to disambiguate multiple Zendesk tenants in the same Bronze store.

**Design principle**: `support_tickets` stores the current ticket state. `zendesk_satisfaction_ratings` captures CSAT ratings as an append-only separate stream. `support_ticket_events` (per-ticket audit log from `/api/v2/tickets/{id}/audits`) is SHIPPED — collected per-ticket via the Ticket Audits API behind a slim key-only parent stream (`support_ticket_ids`) so the substream cache stays small. This pattern mirrors the task-tracking domain (`task_tracker_issues` + `task_tracker_history`).

**Incremental export**: Zendesk provides `GET /api/v2/incremental/tickets.json?start_time={unix_ts}` for efficient bulk export. Use this endpoint for scheduled collection runs — the cursor advances only when the full page is consumed. Full ticket audits are fetched individually via `/api/v2/tickets/{id}/audits` (no bulk audit export); this is SHIPPED — collected per-ticket via the Ticket Audits API behind a slim key-only parent stream (`support_ticket_ids`) so the substream cache stays small.

---

## Phase Scope

| Stream / Table | Phase | API Source | Sync Mode | Notes |
|----------------|-------|-----------|-----------|-------|
| `support_tickets` | **Phase 1** | `GET /api/v2/incremental/tickets.json` | Incremental (`updated_at`) | Core snapshot; includes both business- and calendar-hours timing |
| `support_ticket_ids` | **Phase 1** | `GET /api/v2/incremental/tickets.json` | Incremental (`updated_at`) | Slim key-only parent for the audit substream (`ticket_id` + `updated_at` only, no `metadata`); keeps the CDK parent-record cache tiny |
| `support_agents` | **Phase 1** | `GET /api/v2/users?role[]=agent&role[]=admin` | Full refresh | Identity anchor |
| `zendesk_satisfaction_ratings` | **Phase 1** | `GET /api/v2/satisfaction_ratings` | Incremental (`updated_at`) | Separate stream; not backfilled onto `support_tickets` |
| `support_ticket_events` | **Phase 1 (shipped)** | `GET /api/v2/tickets/{id}/audits` | Append-only per ticket | Per-ticket audits behind the slim `support_ticket_ids` parent; one row per audit with raw `events[]` JSON; explosion + classification in Silver |
| `support_collection_runs` | **DE-SCOPED — platform job stats** | Connector-generated | — | Not implemented; a declarative manifest cannot emit a connector-generated stream — monitoring is via Airbyte platform job stats / sync logs |
| `zendesk_ticket_ext` | **Phase 2** | `ticket.custom_fields[]` | Incremental (with tickets) | Custom field key-value pairs for workspace-specific fields |

**Phase 1 analytics**: ticket volume trends, agent roster and identity resolution, CSAT score distribution, basic assignee/group breakdowns, and audit-derived activity (updates, public/private comments, solved-ticket counts) attributed to the acting agent.

**Phase 2 analytics**: custom attribute segmentation via `zendesk_ticket_ext`.

---

## Bronze Tables

### `support_tickets` — Ticket metadata and current state

Maps to the unified `support_tickets` table defined in `docs/connectors/support/README.md`. Current state snapshot, updated on each collection run.

**API**: `GET /api/v2/incremental/tickets.json?start_time={unix_ts}` (incremental). Side-load `metric_sets` via `?include=metric_sets` to retrieve timing fields in the same response without extra calls.

| Field | Type | Description |
|-------|------|-------------|
| `insight_source_id` | String | Connector instance identifier, e.g. `zendesk-acme` |
| `ticket_id` | String | Zendesk ticket `id` (numeric, stored as string) |
| `subject` | String | Ticket `subject`; NULL if blank |
| `status` | String | `status` field — values: `new` / `open` / `pending` / `hold` / `solved` / `closed` — mapped directly |
| `priority` | String | `priority` field — `low` / `normal` / `high` / `urgent`; NULL if not set |
| `ticket_type` | String | `type` field — `question` / `incident` / `problem` / `task`; NULL if not set |
| `assignee_id` | String | `assignee_id` — numeric Zendesk user ID (agent); NULL if unassigned — joins to `support_agents.agent_id` |
| `group_id` | String | `group_id` — numeric group ID; NULL if unassigned |
| `requester_id` | String | `requester_id` — numeric Zendesk user ID (customer); **not** resolved to `person_id` |
| `organization_id` | String | `organization_id` — numeric org ID; NULL if requester has no organisation |
| `created_at` | DateTime64(3) | `created_at` (ISO 8601 string → DateTime64) |
| `updated_at` | DateTime64(3) | `updated_at` — cursor for incremental sync |
| `solved_at` | DateTime64(3) | `metric_set.solved_at`; NULL if not yet solved |
| `first_reply_time_seconds` | Int64 | `metric_set.reply_time_in_minutes.business` × 60; NULL if no reply yet. **Business hours** — aligned with SLA Policy evaluation |
| `first_reply_time_calendar_seconds` | Int64 | `metric_set.reply_time_in_minutes.calendar` × 60; NULL if no reply yet. **Calendar (wall-clock) hours** — enables cross-source comparison with JSM |
| `full_resolution_time_seconds` | Int64 | `metric_set.full_resolution_time_in_minutes.business` × 60; NULL if unresolved. **Business hours** |
| `full_resolution_time_calendar_seconds` | Int64 | `metric_set.full_resolution_time_in_minutes.calendar` × 60; NULL if unresolved. **Calendar hours** |
| `satisfaction_score` | String | `NULL` in Phase 1 — ratings live in `zendesk_satisfaction_ratings`; backfill to this field is Phase 2 |
| `tags` | String | `tags` array joined as comma-separated string |
| `metadata` | String | Full API response as JSON |
| `data_source` | String | `"insight_zendesk"` |
| `_version` | UInt64 | Collection timestamp in milliseconds — deduplication version |

**Indexes**:
- `idx_support_ticket_lookup`: `(insight_source_id, ticket_id, data_source)`
- `idx_support_ticket_assignee`: `(assignee_id, data_source)`
- `idx_support_ticket_updated`: `(updated_at)`
- `idx_support_ticket_status`: `(status, data_source)`

**Note on timing fields**: both business-hours (`first_reply_time_seconds`, `full_resolution_time_seconds`) and calendar-hours (`first_reply_time_calendar_seconds`, `full_resolution_time_calendar_seconds`) variants are stored. Business-hours values align with Zendesk SLA Policy evaluation. Calendar-hours values enable cross-source consistency with JSM (which derives timing from the event log without business-hours filtering). See Resolved Questions RQ-ZD-3.

**Note on `satisfaction_score`**: Phase 1 always stores NULL here. Satisfaction ratings are collected in the separate `zendesk_satisfaction_ratings` stream. Phase 2 will backfill this field from the ratings stream at Silver processing time.

---

### `support_agents` — Agent directory

Identity anchor for support analytics. Maps to `person_id` via Identity Manager.

**API**: `GET /api/v2/users?role[]=agent&role[]=admin` — returns both agent-tier users. Paginate with `page[after]` cursor (cursor-based pagination). Group-name enrichment (via `GET /api/v2/groups`) is deferred to Phase 2.

| Field | Type | Description |
|-------|------|-------------|
| `insight_source_id` | String | Connector instance identifier |
| `agent_id` | String | Zendesk `user.id` (numeric, stored as string) |
| `email` | String | `user.email` — primary identity key → `person_id` |
| `display_name` | String | `user.name` |
| `role` | String | `user.role` — `agent` / `admin` / `light_agent` |
| `group_id` | String | `user.default_group_id` — numeric primary group ID; NULL if not set |
| `group_name` | String | **Phase 2** — NULL in Phase 1. Display name of the group at `default_group_id`; resolved via `GET /api/v2/groups` once group-name resolution is implemented |
| `is_active` | Int64 | `user.active` — 1 if active; 0 if suspended (`user.suspended = true`) |
| `collected_at` | DateTime64(3) | Collection timestamp |
| `data_source` | String | `"insight_zendesk"` |
| `_version` | UInt64 | Collection timestamp in milliseconds |

**Indexes**:
- `idx_support_agent_lookup`: `(insight_source_id, agent_id, data_source)`
- `idx_support_agent_email`: `(email)`

**Note on `role`**: Zendesk has three agent-tier roles — `agent` (standard), `admin` (full access), `light_agent` (read-only with comment access). All three are returned by the combined role query. Fetch admins separately with `?role=admin` if the `role[]` array param is not supported by the account's plan tier.

**Note on `group_name`**: `default_group_id` references a Zendesk Group. Group-name resolution is **deferred to Phase 2** — Phase 1 stores NULL. When implemented, group names are fetched via `GET /api/v2/groups` at startup and joined at collection time to populate `group_name`.

---

### `zendesk_satisfaction_ratings` — CSAT ratings (separate stream)

Dedicated stream for customer satisfaction ratings. Stored separately rather than backfilled onto `support_tickets` — this preserves full rating history (initial score, updates, and the requester's comment) and avoids mutable overwrites on the ticket snapshot table.

**API**: `GET /api/v2/satisfaction_ratings?sort_by=updated_at&sort_order=asc&start_time={unix_ts}` — incremental by `updated_at`. Returns one record per rating event (initial rating, score change, comment added).

| Field | Type | Description |
|-------|------|-------------|
| `insight_source_id` | String | Connector instance identifier, e.g. `zendesk-acme` |
| `rating_id` | String | Zendesk `satisfaction_rating.id` (numeric, stored as string) |
| `ticket_id` | String | Parent ticket ID — joins to `support_tickets.ticket_id` |
| `requester_id` | String | `requester_id` — the customer who submitted the rating; **not** resolved to `person_id` |
| `assignee_id` | String | `assignee_id` at time of rating — joins to `support_agents.agent_id`; NULL if not set |
| `group_id` | String | `group_id` at time of rating; NULL if not set |
| `score` | String | `"good"` / `"bad"` / `"offered"` (offered but not yet answered); NULL if rating was withdrawn |
| `comment` | String | Free-text comment from the requester; NULL if none provided |
| `reason` | String | `reason_code` or `reason` label from Zendesk (e.g. `"Awesome support"`); NULL if not provided |
| `created_at` | DateTime64(3) | When the rating was first offered |
| `updated_at` | DateTime64(3) | Last update — cursor for incremental sync |
| `data_source` | String | `"insight_zendesk"` |
| `_version` | UInt64 | Collection timestamp in milliseconds |

**Indexes**:
- `idx_zendesk_rating_ticket`: `(insight_source_id, ticket_id, data_source)`
- `idx_zendesk_rating_updated`: `(updated_at)`
- `idx_zendesk_rating_assignee`: `(assignee_id, data_source)`

**Note on `score = "offered"`**: Zendesk emits a rating record as soon as it is offered to the requester, before a response is received. This record has `score = "offered"`. When the requester responds, the existing record is updated to `good` or `bad`. The incremental cursor captures both the creation and the update.

---

### `support_collection_runs` — Connector execution log

> **Status**: DE-SCOPED — not implemented. A declarative manifest cannot emit a connector-generated stream; run monitoring is via Airbyte platform job stats / sync logs. The schema below is retained for reference only.

| Field | Type | Description |
|-------|------|-------------|
| `run_id` | String | Unique run identifier (UUID) |
| `started_at` | DateTime64(3) | Run start timestamp |
| `completed_at` | DateTime64(3) | Run end timestamp; NULL while running |
| `status` | String | `running` / `completed` / `failed` |
| `tickets_collected` | Int64 | Rows upserted into `support_tickets` |
| `ratings_collected` | Int64 | Rows upserted into `zendesk_satisfaction_ratings` |
| `agents_collected` | Int64 | Rows upserted into `support_agents` |
| `api_calls` | Int64 | Total API calls made during the run |
| `errors` | Int64 | Number of errors encountered |
| `settings` | String | Collection configuration as JSON: `subdomain`, `incremental_cursor`, `lookback_days` |
| `data_source` | String | `"insight_zendesk"` |
| `_version` | UInt64 | Collection timestamp in milliseconds |

Monitoring table — not an analytics source.

---

### `support_ticket_events` — Ticket Audit log (SHIPPED)

> **Status**: SHIPPED. Collected per-ticket via the Ticket Audits API behind the slim `support_ticket_ids` parent stream.

Every audit on every ticket is collected from `GET /api/v2/tickets/{id}/audits` via a `SubstreamPartitionRouter` over the slim `support_ticket_ids` parent (`incremental_dependency: true`). This is the source of truth for audit-derived activity (updates, comments, solved-ticket counts).

**Bronze shape**: lands **one row per AUDIT** with the raw `events[]` array stored as a JSON string. The per-event explosion and classification happen in **Silver**, not Bronze.

| Field | Type | Description |
|-------|------|-------------|
| `tenant_id` | String | Tenant identifier |
| `source_id` | String | Connector instance identifier |
| `unique_key` | String | Deduplication key = audit id |
| `collected_at` | DateTime64(3) | Collection timestamp |
| `data_source` | String | `"insight_zendesk"` |
| `audit_id` | String | Zendesk `audit.id` (numeric, stored as string) |
| `ticket_id` | String | Zendesk ticket `id` — joins to `support_tickets.ticket_id` |
| `author_id` | String | `audit.author_id` — Zendesk user ID; NULL for system events — joins to `support_agents.agent_id` |
| `created_at` | DateTime64(3) | `audit.created_at` — when this audit was recorded |
| `events` | String | The audit's `events[]` array, stored as a JSON string |

**Note**: Bronze stores the raw audit + events JSON; the per-event explosion and actor-classification happen in Silver (`zendesk__support_event`): Comment+public→`public_comment`, Comment+private→`private_comment`, Change status=solved→`solved`, other Change→`update`. Attribution is on the ACTOR (`audit.author_id`→agent); end-user/system authors drop.

**Superseded — the old normalized event_type mapping below is no longer the Bronze shape.** The Silver classification taxonomy is `public_comment` / `private_comment` / `solved` / `update`.

| Zendesk audit event type | Superseded `event_type` | Notes |
|--------------------------|---------------------|-------|
| `ChangeEvent` with `field_name = "status"` | `status_change` | `value_from` / `value_to` = raw status strings |
| `ChangeEvent` with `field_name = "assignee_id"` | `assignment` | `value_from` / `value_to` = numeric agent IDs as strings |
| `ChangeEvent` (all other fields) | `field_change` | `field_name` preserved from the event |
| `CommentEvent` (public) | `comment` | `is_public = 1`; `comment_body` from `body` stripped of HTML |
| `CommentEvent` (private) | `comment` | `is_public = 0` |
| `SatisfactionRatingEvent` | `satisfaction_update` | `value_to` = `good` / `bad` / `offered`; `field_name = "satisfaction"` |
| `NotificationEvent`, `CcEvent`, etc. | `field_change` | Captured for completeness |

---

### Phase 2: `zendesk_ticket_ext` — Custom ticket fields

> **Status**: deferred to Phase 2. Schema locked in this spec; implementation pending.

Zendesk tickets support custom fields configured per account via `GET /api/v2/ticket_fields`. Each custom field value appears in `ticket.custom_fields[]` array in the ticket response. Non-standard fields not in the core `support_tickets` schema are written here.

| Field | Type | Description |
|-------|------|-------------|
| `insight_source_id` | String | Connector instance identifier, e.g. `zendesk-acme` |
| `ticket_id` | String | Parent ticket ID — joins to `support_tickets.ticket_id` |
| `field_id` | String | Zendesk custom field ID (numeric, stored as string) |
| `field_title` | String | Custom field display title (from `GET /api/v2/ticket_fields`) |
| `field_value` | String | Field value as string |
| `value_type` | String | Type hint: `string` / `number` / `enumeration` / `date` / `json` |
| `collected_at` | DateTime64(3) | Collection timestamp |

**Discovery**: `GET /api/v2/ticket_fields` returns all custom field definitions for the account. The connector fetches field metadata at startup and maps `field_id` to `field_title` when writing rows.

---

## Identity Resolution

**Identity anchor**: `support_agents` — internal agents who respond to tickets.

**Resolution process**:
1. Extract `email` from `support_agents`
2. Normalize (lowercase, trim)
3. Map to canonical `person_id` via Identity Manager in Silver step 2
4. Propagate `person_id` to `support_ticket_events` (Phase 2) via `author_id` → `support_agents.agent_id` join

**Resolution chain**:
```
support_ticket_events.author_id              (Phase 2)
  → support_agents.agent_id
  → support_agents.email
  → person_id

zendesk_satisfaction_ratings.assignee_id     (Phase 1)
  → support_agents.agent_id
  → support_agents.email
  → person_id
```

**`requester_id` in `support_tickets` and `zendesk_satisfaction_ratings`**: external customers — **not** resolved to `person_id`. Used for volume analytics and routing only.

**`insight_source_id` is required in all joins** — numeric Zendesk IDs (ticket IDs, user IDs) are scoped to one subdomain; they collide across different Zendesk tenants.

---

## Silver / Gold Mappings

### Phase 1

| Bronze table | Silver target | Notes |
|-------------|--------------|-------|
| `support_agents` | Identity Manager (`email` → `person_id`) | Used for identity resolution |
| `support_tickets` | Reference — ticket context | Volume, status, priority breakdowns; ticket counts per group/assignee |
| `zendesk_satisfaction_ratings` | `class_support_activity` (CSAT fields only) | Ratings attributed to agent via `assignee_id` → `person_id`; `satisfaction_score` = fraction `good / (good + bad)` per agent per period |

### Audit-derived activity (`support_ticket_events`, shipped)

| Bronze table | Silver target | Notes |
|-------------|--------------|-------|
| `support_ticket_events` | `zendesk__support_event` → `class_support_activity` | Audit `events[]` exploded (ARRAY JOIN) into one row per event, classified by ACTOR (`audit.author_id` → agent INNER JOIN; end-user/system authors drop) |
| `support_tickets` | `silver.dim_support_ticket` — ticket context | Ticket context dimension; NOT used for attribution |
| `zendesk_satisfaction_ratings` | `class_support_activity` (CSAT fields) | CSAT is assignee-attributed (the one exception to actor-attribution) |

**Attribution principle**: activity is attributed to the ACTOR (who did it), never the assignee — **except CSAT**, which is assignee-attributed because Zendesk binds the rating to the assignee.

**`class_support_activity`** is a person×date rollup (`zendesk__support_activity` staging → shared `silver.class_support_activity`). Shipped fields:

| Field | Derived from |
|-------|-------------|
| `updates` | Count of `update` events (other Change) by actor per date |
| `public_comments` | Count of `public_comment` events (Comment + public) by actor per date |
| `private_comments` | Count of `private_comment` events (Comment + private) by actor per date |
| `solved` | DISTINCT tickets solved per actor per day (`uniqExactIf` over `source_ticket_id`) — NOT solve-event count |
| `csat_good` | CSAT `good` ratings, **assignee-attributed** (good/bad only; `offered` excluded) |
| `csat_total` | CSAT `good` + `bad` ratings, **assignee-attributed** |
| `kb_articles_created` | Honest-NULL — no Guide/Help-Center stream exists |

Event classification (in `zendesk__support_event`): Comment+public→`public_comment`; Comment+private→`private_comment`; Change `field_name=status` value=`solved`→`solved`; any other Change→`update`. `metric_date` = `toDate(audit.created_at)` in UTC.

**Gold metrics (shipped)**: CH views in migration `20260611000000_support-bullet-rows.sql`:
- `support_bullet_rows` (person×date×metric_key) → `support_person_period` → `support_company_stats`.
- 7 metric keys: `support_active`, `support_updates`, `support_public_comments`, `support_private_comments`, `support_solved`, `support_csat_good`, `support_csat_total`. (`support_kb` intentionally not emitted — ComingSoon via catalog.)
- `support_active` = 1 only when actor activity that day (`updates + public_comments + private_comments + solved > 0`); a CSAT-only day does NOT count as active. Rolls up via max (person) / sum→count-of-active-members (company).
- `support_solved` = distinct tickets (label "Solved tickets").
- CSAT % = Σgood / Σtotal, computed in the analytics `query_ref`.

**Downstream (shipped)**: analytics Support metric sets (Team/IC) + `metric_catalog` + thresholds; a frontend "Support" dashboard section. Also: ticket-volume / CSAT-distribution analytics from `support_tickets` + `zendesk_satisfaction_ratings`.

---

## Resolved Questions

### RQ-ZD-1: Phase 1 stream count — 3 streams for MVP

**Decision**: Phase 1 collects `support_tickets`, `support_agents`, and `zendesk_satisfaction_ratings`. `support_ticket_events` (audit log) and `zendesk_ticket_ext` (custom fields) are deferred to Phase 2.

**Rationale**: The audit log endpoint (`GET /api/v2/tickets/{id}/audits`) requires one call per ticket — prohibitively expensive for large accounts on first run. Phase 1 analytics (volume, CSAT, roster) do not require per-event data. The spec locks Phase 2 schemas to ensure no breaking schema changes when the streams are added.

**Update (v2.0):** Phase 2 `support_ticket_events` is now shipped — the audit stream + Silver activity rollup (`class_support_activity`) + Gold (`support_bullet_rows`) are delivered. `zendesk_ticket_ext` remains deferred.

### RQ-ZD-2: `satisfaction_score` — stored as a separate stream

**Decision**: Satisfaction ratings are stored in `zendesk_satisfaction_ratings` as an incremental stream, not backfilled onto `support_tickets.satisfaction_score` during collection. `support_tickets.satisfaction_score` is NULL in Phase 1.

**Rationale**: A separate stream preserves the full rating history (score changes, requester comments, reason codes) that a single-field backfill would lose. The Silver layer can compute `satisfaction_score` per-ticket by joining `zendesk_satisfaction_ratings` on `ticket_id` and taking the latest non-offered score.

### RQ-ZD-3: Business-hours AND calendar-hours timing — both stored in Bronze

**Decision**: `support_tickets` contains four timing fields:
- `first_reply_time_seconds` — business hours (`metric_set.reply_time_in_minutes.business × 60`)
- `first_reply_time_calendar_seconds` — calendar hours (`metric_set.reply_time_in_minutes.calendar × 60`)
- `full_resolution_time_seconds` — business hours
- `full_resolution_time_calendar_seconds` — calendar hours

**Rationale**: SLA Policies in Zendesk are defined in business hours; business-hours values are the correct denominator for Zendesk SLA compliance. However, cross-source comparison with JSM (which derives timings from the event log, inherently calendar-hours) requires a consistent baseline. Storing both values in Bronze costs two extra Int64 fields and avoids a re-collection when the Silver layer needs the other variant.

---

## Open Questions

### OQ-ZD-4: `support_ticket_events` incremental audit collection strategy (Phase 2)

Fetching audits requires one API call per ticket (`GET /api/v2/tickets/{id}/audits`). For large accounts with millions of tickets, fetching all audits on the first run is expensive. Zendesk does not provide a bulk audit export endpoint.

**Question**: Should the initial collection only fetch audits for tickets updated within the lookback window (e.g. last 90 days), accepting that older tickets have no event history in Bronze? Or should the connector offer a configurable full-history backfill mode (rate-limited, resumable)?

**Current plan**: The audit stream fans out over the slim `support_ticket_ids` parent with `incremental_dependency: true`, so each run only (re)fetches audits for tickets whose `updated_at` advanced since the last cursor — bounded by the incremental window plus `lookback_window: P1D`. There is no `backfill_mode` config key; first-run history is governed by the parent stream's `start_date` / cursor, not a separate backfill mode.

### OQ-ZD-5: Business-hours-only satisfaction_score on support_tickets (Phase 2)

When Phase 2 backfills `support_tickets.satisfaction_score` from `zendesk_satisfaction_ratings`, should the Silver job use the latest rating or the last non-`offered` rating?

**Question**: A ticket may have multiple rating events (`offered` → `good` → changed to `bad`). Should `satisfaction_score` store the most recent non-`offered` value, or the most recent value (including potential `null` when a rating is withdrawn)?

**Current plan**: use the most recent non-`offered` and non-`null` score. If no answered rating exists, `satisfaction_score` remains NULL.
