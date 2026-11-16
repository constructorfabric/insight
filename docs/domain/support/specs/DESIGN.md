# Technical Design — Support Domain

Version 0.1 — November 2026
Based on: #886, #2297

Target bronze → silver design for customer support data, with Zendesk as the
first source. It describes where the domain should end up, not the shipped
connector; [section 8](#8-gap-to-the-shipped-connector) lists the difference.
Gold and UI are out of scope.

<!-- toc -->

- [1. What support is](#1-what-support-is)
- [2. Principles](#2-principles)
- [3. Zendesk connector](#3-zendesk-connector)
- [4. Silver contract](#4-silver-contract)
- [5. What is reused from task tracking](#5-what-is-reused-from-task-tracking)
- [6. Invariants](#6-invariants)
- [7. Sources that shape the contract](#7-sources-that-shape-the-contract)
- [8. Gap to the shipped connector](#8-gap-to-the-shipped-connector)
- [Open Questions](#open-questions)

<!-- tocstop -->

## 1. What support is

A helpdesk looks like a tracker — tickets, statuses, assignees, comments — but
answers a different question.

| | Task tracking | Support |
|---|---|---|
| Who opens the item | the team itself | an external customer (email, form, chat, phone) |
| Question it answers | what was done, how long it took | how fast and how well the customer was answered |
| Core measures | throughput, cycle time, estimates | first reply, resolution time, SLA, CSAT, reopens |
| Comments | internal discussion | public reply to the customer vs internal note |
| Participants | all employees | the requester is external and never a person in Insight |

Support is therefore its own domain (`class_support_*`), not rows in
`class_task_*`. A tracker project that a tenant runs as a support queue reaches
this domain as a projection of task classes ([section 5](#5-what-is-reused-from-task-tracking)).

## 2. Principles

- **Bronze is raw.** The connector lands every object whole (`metadata`
  holds the API response). Classification, canonical values and durations are
  silver's job, so a new metric never needs a resync.
- **History is event-sourced.** State at any instant is folded from events;
  the current snapshot is an observation that checks the fold, not the truth.
- **Activity is attributed to the actor, quality to the owner.** A reply
  counts for whoever wrote it; CSAT and SLA count for the assignee or group
  at the moment the rating or breach happened.
- **Every duration exists twice**: business hours and calendar hours.
- **Keys are `{tenant}-{source}-{native id}`.** Zendesk ids are unique only
  inside one subdomain.

## 3. Zendesk connector

**API:** REST v2 at `https://{subdomain}.zendesk.com/api/v2/`. Basic auth with
an admin email and API token — the incremental exports, SLA policies and
satisfaction ratings are admin-only. Users are `id`-keyed; agents resolve to
people by email.

**Why many tables:** a ticket has many audits, each audit many events; SLA,
ratings and metric events are separate objects with their own lifecycles, and
custom fields, statuses and schedules are needed to decode them.

| Bronze table | Endpoint | Mode | Purpose |
|---|---|---|---|
| `zendesk_tickets` | `incremental/tickets/cursor.json?include=metric_sets` | incremental, cursor | Ticket snapshot, whole: `custom_fields`, `tags`, `via`, brand, form, custom status |
| `zendesk_ticket_audits` | `tickets/{id}/audits` | substream over changed tickets | Full history from `Create`, with authors and comments |
| `zendesk_ticket_metric_events` | `incremental/ticket_metric_events` | incremental, time | SLA lifecycle: apply, activate, pause, fulfill, breach |
| `zendesk_satisfaction_ratings` | `satisfaction_ratings` | incremental, time | CSAT |
| `zendesk_users` | `incremental/users/cursor.json` | incremental, cursor | Agents and end users, role, suspension |
| `zendesk_organizations` | `incremental/organizations` | incremental, time | Customer accounts |
| `zendesk_groups`, `zendesk_group_memberships` | list | full refresh | Teams and who is in them |
| `zendesk_ticket_fields`, `zendesk_ticket_forms`, `zendesk_custom_statuses`, `zendesk_brands` | list | full refresh | Dictionaries to decode tickets and audits |
| `zendesk_sla_policies`, `zendesk_schedules` | list | full refresh | SLA targets and business hours |
| `zendesk_deleted_tickets` | `deleted_tickets` | full refresh | Deletion census |
| `zendesk_kb_articles` | `help_center/incremental/articles` | incremental, time | Knowledge base (Guide) |

**Pagination is per endpoint, never mixed.** Cursor exports stop on
`end_of_stream` / `has_more = false`; time-based exports follow `next_page`
until `end_of_stream` and deduplicate on the record id, because a page
boundary repeats records of the same second.

**Rate budget.** All `incremental/*` calls share one per-account budget of 10
requests a minute. Schedules are staggered so tickets come first; the audit
substream draws on the general limit instead and is bounded by
`incremental_dependency` on the ticket parent.

**Deletions.** A deleted ticket stays in the export with status `deleted` and
its audits answer 404. The 404 is recorded as an availability event, never a
failed sync.

## 4. Silver contract

Facts and dimensions only; per-day aggregates belong to gold.

| Class | Grain | Built from |
|---|---|---|
| `class_support_ticket_field_history` | ticket × field × event | audits (`Create`, `Change`), deletions |
| `class_support_tickets` | ticket, current state | fold of history, checked against the snapshot |
| `class_support_interactions` | one message | audit `Comment` events |
| `class_support_sla_events` | ticket × SLA metric × instance × event | ticket metric events |
| `class_support_sla_policies` | policy × metric × priority | SLA policies |
| `class_support_satisfaction` | one rating | satisfaction ratings |
| `class_support_agents` | agent | users with an agent role |
| `class_support_groups`, `class_support_group_memberships` | group; agent × group | groups |
| `class_support_customers` | requester or organization | users with the end-user role, organizations |
| `class_support_schedules` | schedule × interval | business-hours schedules |
| `class_support_kb_articles` | article × event | Guide articles |

Every class also carries `unique_key`, `tenant_id`, `insight_source_id`,
`data_source`, `collected_at`, `_version`; they are omitted below.

### Ticket field history

Same shape as `class_task_field_history`.

| Field | Type | Description |
|---|---|---|
| `ticket_id` | String | Native ticket id |
| `event_id` | String | Audit id; shared by every field one audit changed |
| `event_kind` | String | `initial` (from `Create`), `change`, `availability`, `snapshot_diff` |
| `field_id` | String | `status`, `priority`, `group_id`, `assignee_id`, `tags`, or a custom field id |
| `value` | String (JSON) | New value; the whole set for `tags` and multi-select fields |
| `previous_value` | String (JSON) | Value before the event |
| `event_at` | DateTime | Audit time |
| `event_order` | Int | Total order within a ticket: time, then audit id, then event id |
| `actor_id` | String | Audit author |
| `actor_kind` | String | `agent`, `end_user`, `system` (trigger, automation, API) |
| `channel` | String | `via.channel` of the audit |

### Tickets

| Field | Type | Description |
|---|---|---|
| `ticket_id` | String | Native ticket id |
| `status_raw`, `status` | String | Vendor status; canonical `new`, `open`, `pending`, `on_hold`, `solved`, `closed` |
| `priority`, `type`, `channel` | String | Canonical values |
| `brand_id`, `form_id` | String | Context for drivers |
| `requester_id`, `organization_id` | String | External; never resolved to a person |
| `assignee_id`, `group_id` | String | Owner now |
| `created_at`, `solved_at`, `closed_at` | DateTime | Lifecycle |
| `merged_into_id` | String | Set when the ticket was closed by a merge |
| `availability` | String | `present`, `deleted`, `archived` |
| `custom_attrs` | String (JSON) | Custom field id → decoded value |

### Interactions

The common denominator for tickets, chats and calls; first reply, reply
counts and one-touch resolution are computed from it.

| Field | Type | Description |
|---|---|---|
| `interaction_id` | String | Comment event id |
| `ticket_id` | String | Native ticket id |
| `author_id` | String | Writer |
| `author_kind` | String | `agent`, `end_user`, `system` |
| `is_public` | Int (bool) | Reply to the customer (1) or internal note (0) |
| `channel` | String | `email`, `web`, `chat`, `voice`, `api`, … |
| `occurred_at` | DateTime | Time |

### SLA events and policies

| Field | Type | Description |
|---|---|---|
| `ticket_id` | String | Native ticket id |
| `metric` | String | Canonical: `first_reply`, `next_reply`, `periodic_update`, `requester_wait`, `agent_work` |
| `instance_id` | Int | Repeat number for metrics that fire more than once per ticket |
| `event_type` | String | `apply`, `activate`, `pause`, `fulfill`, `breach` |
| `is_cancelled` | Int (bool) | A breach the vendor later withdrew |
| `target_minutes` | Int | Target in force |
| `business_hours` | Int (bool) | Whether the target counts business hours |
| `policy_id` | String | Applied policy |
| `occurred_at` | DateTime | Time |

`class_support_sla_policies` holds `policy_id`, `title`, `position`,
`metric`, `priority`, `target_minutes`, `business_hours`.

### Satisfaction

| Field | Type | Description |
|---|---|---|
| `rating_id` | String | Native id |
| `ticket_id` | String | Rated ticket |
| `score` | String | Canonical `good`, `bad`, `offered` |
| `score_raw` | numeric | Vendor score when the scale is numeric |
| `scale_max` | Int | Top of the scale; NULL for good/bad |
| `has_comment` | Int (bool) | Customer left text |
| `reason` | String | Vendor reason code |
| `assignee_id`, `group_id` | String | Owner when the rating was given |
| `rated_at` | DateTime | Time |

### Dimensions

- `class_support_agents`: `agent_id`, `email`, `role`, `is_active`. Resolved
  to a person through identity resolution.
- `class_support_groups` / `class_support_group_memberships`: team and
  membership, with `valid_from` / `valid_to` once membership history is known.
- `class_support_customers`: `customer_id`, `kind` (`user`, `organization`),
  `organization_id`. Never resolved to a person.
- `class_support_schedules`: `schedule_id`, `time_zone`, weekly intervals,
  holidays — what business-hours durations are computed against.
- `class_support_kb_articles`: `article_id`, `event_type` (`created`,
  `published`, `updated`, `archived`), `author_id`, `occurred_at`.

## 5. What is reused from task tracking

- **The history shape and its tooling**: per entity × field × event rows,
  `event_kind`, a single `event_order`, and `snapshot_diff` reconciliation.
  Zendesk maps cleanly — `Create` is the initial state, so no synthetic
  initial is derived backwards as Jira needs.
- **Operator value maps** (`config.field_value_map` and defaults): vendor
  statuses, priorities and custom field values to canonical values, under a
  support field namespace.
- **Tracker queues as a support source**: issues from a project the tenant
  maps as support are projected from `class_task_*` into `class_support_*`.
  No second connector is needed for them.

The task classes themselves are not reused: their gold would read tickets as
work items, and they have no place for requester, channel, public vs internal
replies, SLA or CSAT.

## 6. Invariants

- An agent's activity never includes `end_user` or `system` events.
- `requester_id` and `organization_id` never join to people.
- A merged or deleted ticket is never counted as solved.
- A reopen is a `status` change out of `solved` into an open category.
- `tags` and multi-select fields store whole sets; additions and removals are
  set differences between consecutive events.
- A custom field change is unreadable without `zendesk_ticket_fields`; both
  land in the same sync.

## 7. Sources that shape the contract

The contract is fixed only after it fits sources with different models, not
only Zendesk look-alikes.

| Source | What it tests |
|---|---|
| Jira Service Management | Tracker-based queues, request types, SLA goals and cycles; projection from task classes |
| Intercom | Conversation instead of ticket; bots and AI agents as authors |
| ServiceNow | ITSM: incidents and requests, `sys_audit` history, `task_sla` |
| Salesforce Service Cloud | Case field history, entitlements and milestones as SLA |
| Freshdesk | Near-isomorphic to Zendesk — a cheap check against vendor lock-in |
| HubSpot Service Hub | Tickets inside a CRM already planned for sales |
| Aircall, Zendesk Talk | Voice interactions: duration, missed calls |

Minimum set before freezing the contract: Zendesk, Jira Service Management,
Intercom, and ServiceNow or Salesforce.

## 8. Gap to the shipped connector

| Area | Shipped | Target |
|---|---|---|
| Ticket payload | selected columns; custom fields dropped | whole object |
| Audit events | classified into four counters; values dropped | full field history with previous values |
| SLA | none | policies + metric events |
| Users | agents only, full refresh | all users, incremental export |
| Groups, organizations, fields, statuses, schedules | none | dictionary streams |
| Knowledge base | none | Guide articles |
| Silver | `class_support_activity` (person × day) + two dims | fact and dimension classes above; aggregates move to gold |
| Gold | none | out of scope here |

## Open Questions

### OQ-1: New survey-based CSAT

Accounts on the newer CSAT get an empty legacy ratings list. Whether its
responses are reachable through a public API needs checking on a test account.

### OQ-2: Bulk ticket events instead of per-ticket audits

`incremental/ticket_events` returns events for all tickets in pages, far
cheaper than one request per ticket, but shares the 10-a-minute budget, starts
at `start_time` (tickets older than the window lack `Create`) and, per the
documentation examples, carries field names as keys rather than in
`field_name`. Options: audits for backfill and bulk events for increments, or
audits only. Decide after comparing both payloads on a test account.

### OQ-3: AI-agent tickets

The ticket export defaults to `support_type_scope=agent` and omits tickets
handled only by AI agents. Whether they belong in the domain, and as which
`actor_kind`, is open.

### OQ-4: Group membership history

Zendesk exposes current memberships only. Folding them into intervals needs
a snapshot per sync; whether that precision is worth it depends on the
team-level metrics gold will build.
