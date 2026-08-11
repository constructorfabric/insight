# CRM Connector Specification (Multi-Source)

> Version 1.0 — March 2026
> Based on: HubSpot (Source 16) and Salesforce (Source 17)

Defines the Silver layer for CRM connectors. The Silver layer has two steps: Step 1 unifies raw Bronze data from source-specific tables (`hubspot_*`, `salesforce_*`) into a common schema; Step 2 enriches with `person_id` via Identity Resolution.

**Primary analytics focus**: internal salespeople (employees) — their deal ownership, activity volume, and workload. External contacts and accounts are reference data only.

**Dual analytics purpose**: CRM connectors serve two distinct analytics use cases that must both be supported:

1. **Pipeline analytics** — deals, stages, pipeline value, win rate, forecast. Answers: *What is in the pipeline? How is it progressing?*
2. **Sales activity analytics** — outreach activity per sales rep: calls made, emails sent, meetings booked, tasks completed. Answers: *How active is each rep? Are they doing the work that leads to deals?*

Sales activity analytics is the primary signal for **Sales rep productivity measurement** — the equivalent of commit count for engineers. A sales rep's pipeline may lag by weeks or months, but activity volume is an immediate, high-frequency signal that reflects current effort. `crm_activities` is the central table for this use case.

<!-- toc -->

- [Overview](#overview)
- [Silver Tables — Step 1: Unified Schema (pre-Identity Resolution)](#silver-tables--step-1-unified-schema-pre-identity-resolution)
  - [`crm_users` — Internal salesperson directory](#crm_users--internal-salesperson-directory)
  - [`crm_deals` — Deal / opportunity pipeline](#crm_deals--deal--opportunity-pipeline)
  - [`crm_activities` — Calls, meetings, tasks](#crm_activities--calls-meetings-tasks)
  - [`crm_contacts` — External contact reference](#crm_contacts--external-contact-reference)
  - [`crm_accounts` — Company / account reference](#crm_accounts--company--account-reference)
  - [`crm_collection_runs` — Connector execution log](#crm_collection_runs--connector-execution-log)
- [Source Mapping](#source-mapping)
  - [HubSpot](#hubspot)
- [Identity Resolution](#identity-resolution)
- [Silver Step 2 → Gold](#silver-step-2--gold)
- [Open Questions](#open-questions)
  - [OQ-CRM-1: `is_won` / `is_closed` derivation for HubSpot](#oq-crm-1-is_won--is_closed-derivation-for-hubspot)
  - [OQ-CRM-2: Activity duration normalisation](#oq-crm-2-activity-duration-normalisation)
  - [OQ-CRM-3: Stage normalisation across sources](#oq-crm-3-stage-normalisation-across-sources)

<!-- /toc -->

---

## Overview

**Category**: CRM

**Supported Sources**:
- HubSpot (`data_source = "insight_hubspot"`)
- Salesforce (`data_source = "insight_salesforce"`)

**Authentication**:
- HubSpot: Private App token
- Salesforce: OAuth 2.0 (Connected App)

**Identity**: `crm_users.email` — internal salespeople resolved to canonical `person_id` via Identity Manager. `crm_contacts.email` is for external customers and is **not** resolved to `person_id`.

**Why multi-source design**: Organizations may use HubSpot and Salesforce simultaneously (e.g. different business units) or migrate between them. This unified schema enables:
- Single query across all CRM sources for sales performance analytics
- Consistent identity resolution: `owner_id` → `crm_users.email` → `person_id`
- Simplified Silver/Gold transformation regardless of CRM vendor

**Terminology mapping**:

| Concept | HubSpot | Salesforce | Unified |
|---------|---------|------------|---------|
| Internal user | Owner | User | `crm_users` |
| Deal | Deal | Opportunity | `crm_deals` |
| Activity | Engagement (call/meeting/task/email) | Task + Event | `crm_activities` |
| External person | Contact | Contact | `crm_contacts` |
| Company | Company | Account | `crm_accounts` |

---

## Silver Tables — Step 1: Unified Schema (pre-Identity Resolution)

> **Silver Step 1**: Data from source-specific Bronze tables ([hubspot](hubspot/hubspot.md)) is normalized and written here. No `person_id` yet — Identity Resolution runs in Step 2.

### `crm_users` — Internal salesperson directory

Identity anchor for all CRM analytics. Maps to `person_id` via Identity Manager.

| Field | Type | Constraints | Description |
|-------|------|-------------|-------------|
| `user_id` | String | REQUIRED | Source-specific user ID (`owner.id` in HubSpot, `User.Id` 18-char in Salesforce) |
| `email` | String | REQUIRED | Email — primary identity key → `person_id` |
| `first_name` | String | NULLABLE | First name |
| `last_name` | String | NULLABLE | Last name |
| `title` | String | NULLABLE | Job title |
| `department` | String | NULLABLE | Department (Salesforce only; NULL for HubSpot) |
| `is_active` | Int64 | REQUIRED | 1 if active, 0 if deactivated / archived |
| `metadata` | String | REQUIRED | Full API response as JSON |
| `collected_at` | DateTime64(3) | REQUIRED | Collection timestamp |
| `data_source` | String | DEFAULT '' | Source discriminator |
| `_version` | UInt64 | REQUIRED | Deduplication version (millisecond timestamp) |

**Indexes**:
- `idx_crm_user_lookup`: `(user_id, data_source)`
- `idx_crm_user_email`: `(email)`

---

### `crm_deals` — Deal / opportunity pipeline

| Field | Type | Constraints | Description |
|-------|------|-------------|-------------|
| `deal_id` | String | REQUIRED | Source-specific deal ID |
| `name` | String | REQUIRED | Deal / opportunity name |
| `pipeline` | String | NULLABLE | Pipeline name (HubSpot) or forecast category (Salesforce) |
| `stage` | String | REQUIRED | Current stage — raw source value (portal-specific for HubSpot, standard for Salesforce) |
| `amount` | Float64 | NULLABLE | Deal amount |
| `close_date` | Date | NULLABLE | Expected or actual close date |
| `owner_id` | String | REQUIRED | Salesperson ID — joins to `crm_users.user_id` |
| `account_id` | String | NULLABLE | Associated company/account ID — joins to `crm_accounts.account_id` |
| `is_closed` | Int64 | NULLABLE | 1 if deal is closed — derived from stage for HubSpot, native field for Salesforce |
| `is_won` | Int64 | NULLABLE | 1 if deal was won — derived from stage for HubSpot, native field for Salesforce |
| `lead_source` | String | NULLABLE | Lead origin (Salesforce only; NULL for HubSpot) |
| `probability` | Float64 | NULLABLE | Win probability 0–100 (Salesforce only; NULL for HubSpot) |
| `metadata` | String | REQUIRED | Full API response as JSON |
| `custom_str_attrs` | Map(String, String) | DEFAULT {} | Workspace-specific string custom fields promoted from `hubspot_deal_ext` / `salesforce_opportunity_ext` per Custom Attributes Configuration |
| `custom_num_attrs` | Map(String, Float64) | DEFAULT {} | Workspace-specific numeric custom fields promoted from `hubspot_deal_ext` / `salesforce_opportunity_ext` per Custom Attributes Configuration |
| `created_at` | DateTime64(3) | REQUIRED | Deal creation |
| `updated_at` | DateTime64(3) | REQUIRED | Last update — cursor for incremental sync |
| `data_source` | String | DEFAULT '' | Source discriminator |
| `_version` | UInt64 | REQUIRED | Deduplication version |

**Indexes**:
- `idx_crm_deal_lookup`: `(deal_id, data_source)`
- `idx_crm_deal_owner`: `(owner_id, data_source)`
- `idx_crm_deal_updated`: `(updated_at)`

**Note on `is_won` / `is_closed` for HubSpot**: these fields are not returned by the Deals API. Derivation requires fetching pipeline stage settings via `GET /crm/v3/pipelines/deals` and comparing `stage` against stages marked as `closedWon` or `closedLost`. See OQ-CRM-1.

---

### `crm_activities` — Calls, meetings, tasks

| Field | Type | Constraints | Description |
|-------|------|-------------|-------------|
| `activity_id` | String | REQUIRED | Source-specific activity ID |
| `activity_type` | String | REQUIRED | Normalised type: `call` / `meeting` / `task` / `email` / `note` / `event` |
| `owner_id` | String | REQUIRED | Salesperson who performed the activity — joins to `crm_users.user_id` |
| `contact_id` | String | NULLABLE | Associated external contact (nullable) |
| `deal_id` | String | NULLABLE | Associated deal (nullable) |
| `account_id` | String | NULLABLE | Associated company/account (nullable) |
| `timestamp` | DateTime64(3) | REQUIRED | When the activity occurred |
| `duration_seconds` | Int64 | NULLABLE | Duration in seconds — normalised from milliseconds (HubSpot) or minutes (Salesforce Events) |
| `outcome` | String | NULLABLE | Human-readable outcome or status (call disposition label, meeting outcome, task status) |
| `metadata` | String | REQUIRED | Full API response as JSON |
| `created_at` | DateTime64(3) | REQUIRED | Record creation |
| `data_source` | String | DEFAULT '' | Source discriminator |
| `_version` | UInt64 | REQUIRED | Deduplication version |

**Indexes**:
- `idx_crm_activity_lookup`: `(activity_id, data_source)`
- `idx_crm_activity_owner`: `(owner_id, data_source)`
- `idx_crm_activity_timestamp`: `(timestamp)`

**Duration normalisation**:
- HubSpot calls: `hs_call_duration` (milliseconds) ÷ 1000 → seconds
- HubSpot meetings: `(hs_meeting_end_time − hs_meeting_start_time)` ÷ 1000 → seconds
- Salesforce Events: `DurationInMinutes` × 60 → seconds
- Salesforce Tasks (calls): `CallDurationInSeconds` → seconds (no conversion needed)

**`outcome` normalisation**:
- HubSpot calls: resolve `hs_call_disposition` GUID via `GET /crm/v3/objects/call-dispositions` → human label
- HubSpot meetings: `hs_meeting_outcome` → `SCHEDULED` / `COMPLETED` / `NO_SHOW` / `CANCELLED`
- Salesforce Tasks: `Status` field → `Not Started` / `In Progress` / `Completed` / `Deferred`

---

### `crm_contacts` — External contact reference

Reference data only — not resolved to `person_id`. Used to enrich deal and activity context.

| Field | Type | Constraints | Description |
|-------|------|-------------|-------------|
| `contact_id` | String | REQUIRED | Source-specific contact ID |
| `email` | String | NULLABLE | Contact email (external customer) |
| `first_name` | String | NULLABLE | First name |
| `last_name` | String | NULLABLE | Last name |
| `owner_id` | String | NULLABLE | Assigned salesperson — joins to `crm_users.user_id` |
| `account_id` | String | NULLABLE | Associated company — joins to `crm_accounts.account_id` |
| `lifecycle_stage` | String | NULLABLE | HubSpot lifecycle stage; NULL for Salesforce (`lead_source` used instead) |
| `metadata` | String | REQUIRED | Full API response as JSON |
| `custom_str_attrs` | Map(String, String) | DEFAULT {} | Workspace-specific string custom fields promoted from `hubspot_contact_ext` / `salesforce_contact_ext` per Custom Attributes Configuration |
| `custom_num_attrs` | Map(String, Float64) | DEFAULT {} | Workspace-specific numeric custom fields (e.g. scores, tiers) |
| `created_at` | DateTime64(3) | REQUIRED | Record creation |
| `updated_at` | DateTime64(3) | REQUIRED | Last update |
| `data_source` | String | DEFAULT '' | Source discriminator |
| `_version` | UInt64 | REQUIRED | Deduplication version |

**Indexes**:
- `idx_crm_contact_lookup`: `(contact_id, data_source)`

---

### `crm_accounts` — Company / account reference

Reference data only. Used to group deals and activities by company.

| Field | Type | Constraints | Description |
|-------|------|-------------|-------------|
| `account_id` | String | REQUIRED | Source-specific company/account ID |
| `name` | String | REQUIRED | Company name |
| `domain` | String | NULLABLE | Website domain (HubSpot `domain`; Salesforce `Website`) |
| `industry` | String | NULLABLE | Industry classification |
| `owner_id` | String | NULLABLE | Account owner — joins to `crm_users.user_id` |
| `parent_account_id` | String | NULLABLE | Parent company for hierarchies (Salesforce `ParentId`; NULL for HubSpot) |
| `metadata` | String | REQUIRED | Full API response as JSON |
| `created_at` | DateTime64(3) | REQUIRED | Record creation |
| `updated_at` | DateTime64(3) | REQUIRED | Last update |
| `data_source` | String | DEFAULT '' | Source discriminator |
| `_version` | UInt64 | REQUIRED | Deduplication version |

**Indexes**:
- `idx_crm_account_lookup`: `(account_id, data_source)`

---

### `crm_collection_runs` — Connector execution log

| Field | Type | Constraints | Description |
|-------|------|-------------|-------------|
| `run_id` | String | REQUIRED | Unique run identifier (UUID) |
| `started_at` | DateTime64(3) | REQUIRED | Run start timestamp |
| `completed_at` | DateTime64(3) | NULLABLE | Run end timestamp |
| `status` | String | REQUIRED | `running` / `completed` / `failed` |
| `users_collected` | Int64 | NULLABLE | Rows collected for `crm_users` |
| `deals_collected` | Int64 | NULLABLE | Rows collected for `crm_deals` |
| `activities_collected` | Int64 | NULLABLE | Rows collected for `crm_activities` |
| `contacts_collected` | Int64 | NULLABLE | Rows collected for `crm_contacts` |
| `accounts_collected` | Int64 | NULLABLE | Rows collected for `crm_accounts` |
| `api_calls` | Int64 | NULLABLE | Total API / SOQL calls made |
| `errors` | Int64 | NULLABLE | Number of errors encountered |
| `settings` | String | NULLABLE | Collection configuration as JSON |
| `data_source` | String | DEFAULT '' | Source discriminator |
| `_version` | UInt64 | REQUIRED | Deduplication version |

---

## Source Mapping

> Per-source Bronze schemas (raw connector output) are defined in [hubspot](hubspot/hubspot.md). The tables below describe how those Bronze records are normalized into Silver Step 1 unified tables.

### HubSpot

| Unified table | HubSpot source | Key mapping notes |
|---------------|---------------|-------------------|
| `crm_users` | `GET /crm/v3/owners` | `id` → `user_id`; `archived` → `is_active = 0` |
| `crm_deals` | `GET /crm/v3/objects/deals` | `hubspot_owner_id` → `owner_id`; `dealstage` → `stage`; `is_won`/`is_closed` derived from pipeline settings |
| `crm_activities` | `GET /crm/v3/objects/calls` + `/meetings` + `/tasks` + `/emails` + `/notes` | `hs_timestamp` → `timestamp`; `hs_call_duration` (ms) ÷ 1000 → `duration_seconds`; disposition GUID resolved to label → `outcome` |
| `crm_contacts` | `GET /crm/v3/objects/contacts` | `hubspot_owner_id` → `owner_id`; company association via Associations API |
| `crm_accounts` | `GET /crm/v3/objects/companies` | `domain` → `domain`; no parent hierarchy in HubSpot |

---

## Identity Resolution

**Identity anchor**: `crm_users` — internal salespeople only.

**Resolution process**:
1. Extract `email` from `crm_users`
2. Normalize (lowercase, trim)
3. Map to canonical `person_id` via Identity Manager in Silver step 2
4. Propagate `person_id` to `crm_deals` and `crm_activities` via `owner_id` join

**`crm_contacts.email`** — external customers, **not** resolved to `person_id`. Treated as CRM reference data.

**Cross-source matching**: same salesperson may exist in both HubSpot and Salesforce if the organization uses both. Email-based resolution ensures they map to a single `person_id`.

---

## Silver Step 2 → Gold

Silver Step 1 (`crm_*`) feeds into Silver Step 2 (`class_*`) after Identity Resolution adds `person_id`.

| Silver Step 1 table | Silver Step 2 target | Notes |
|---------------------|----------------------|-------|
| `crm_users` | Identity Manager (`email` → `person_id`) | Used for identity resolution |
| `crm_deals` | `class_crm_deals` | Planned — unified deal pipeline stream |
| `crm_activities` | `class_crm_activities` | Planned — unified activity stream |
| `crm_contacts` | *(reference only)* | No Silver Step 2 target — used to enrich deal/activity context |
| `crm_accounts` | *(reference only)* | No Silver Step 2 target — used for grouping by company |

**Planned Silver Step 2 streams**:
- `class_crm_deals`: deduplicated deals with resolved `person_id`, normalised `is_won`/`is_closed`, stage category
- `class_crm_activities`: unified activities with resolved `person_id`, normalised `duration_seconds`, human-readable `outcome`

**Gold metrics**:

*Pipeline analytics* (from `crm_deals`):
- Per salesperson: deal count, total deal value, win rate, average deal cycle time
- Pipeline health: open deals by stage, weighted pipeline value

*Sales activity analytics* (from `crm_activities`):
- Activity volume per rep per week: calls made, emails sent, meetings booked, tasks completed
- Activity mix: ratio of calls vs emails vs meetings per rep (prospecting pattern analysis)
- Outcome rate: completed activities vs total activities (effectiveness signal)
- Workload: activity distribution and balance across team members

> **Note on `crm_activities` as the productivity signal**: Sales activity metrics are the primary productivity measure for sales roles — analogous to commit count for engineers. Unlike deal metrics (which lag by weeks or months), activity counts are a real-time signal of daily effort. The `crm_activities` table maps from HubSpot Engagements API (`/crm/v3/objects/calls`, `/meetings`, `/tasks`, `/emails`) and Salesforce Activity objects (`Task` + `Event`). Both sources are already covered in the Bronze schema above.

---

## Open Questions

### OQ-CRM-1: `is_won` / `is_closed` derivation for HubSpot

HubSpot does not expose `is_won` / `is_closed` as deal properties. Derivation requires:
1. Fetching pipeline settings: `GET /crm/v3/pipelines/deals`
2. For each stage: checking `metadata.isClosed` and `metadata.probability` (100 = won, 0 = lost)

**Question**: Should derivation happen at Bronze collection time (store computed values) or in Silver transformation (store raw `stage` only in Bronze)?

**Current approach**: Store raw `stage` in Bronze, derive `is_won`/`is_closed` in Silver using pipeline settings cached separately.

---

### OQ-CRM-2: Activity duration normalisation

Duration units differ across sources and activity types:
- HubSpot calls: milliseconds
- HubSpot meetings: start/end timestamps (calculate difference)
- Salesforce Events: minutes
- Salesforce Tasks (calls): seconds

**Question**: Should normalisation to seconds happen at Bronze collection time or in Silver?

**Current approach**: Normalise to seconds at collection time, store in `duration_seconds`. Raw value preserved in `metadata`.

---

### OQ-CRM-3: Stage normalisation across sources

HubSpot deal stages are portal-specific internal names (e.g. `appointmentscheduled`, `closedwon`). Salesforce uses more standardised names (`Prospecting`, `Closed Won`). There is no universal stage taxonomy.

**Question**: Should `class_crm_deals` normalise stages to a common set (e.g. `open` / `won` / `lost`) or preserve source-specific values?

**Current approach**: Preserve raw stage in Bronze. In Silver, derive binary `is_won`/`is_closed` — sufficient for most analytics. Full stage funnel analysis remains source-specific.
