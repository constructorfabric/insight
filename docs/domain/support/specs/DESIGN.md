# Technical Design — Support Domain

Version 0.2 — November 2026
Based on: #886, #2297

Target bronze → silver design for customer support data, with Zendesk as the
first source. It describes where the domain should end up, not the shipped
connector; [section 10](#10-gap-to-the-shipped-connector) lists the
difference. Gold and UI are out of scope.

<!-- toc -->

- [1. What support is](#1-what-support-is)
- [2. Principles](#2-principles)
- [3. Zendesk connector](#3-zendesk-connector)
- [4. Custom fields](#4-custom-fields)
- [5. Silver contract](#5-silver-contract)
- [6. Merged tickets](#6-merged-tickets)
- [7. What is reused from task tracking](#7-what-is-reused-from-task-tracking)
- [8. Invariants](#8-invariants)
- [9. Sources that shape the contract](#9-sources-that-shape-the-contract)
- [10. Gap to the shipped connector](#10-gap-to-the-shipped-connector)
- [11. Deferred: support queues in a tracker](#11-deferred-support-queues-in-a-tracker)
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
`class_task_*`.

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
- **What the vendor does not keep, we keep.** Dictionaries and memberships
  have no history in Zendesk; silver snapshots them on every sync.

## 3. Zendesk connector

**API:** REST v2 at `https://{subdomain}.zendesk.com/api/v2/`. Basic auth with
an admin email and API token — the incremental exports, SLA policies and
satisfaction data are admin-only. Agents resolve to people by email.

**Why many tables:** a ticket has many audits, each audit many events; SLA,
ratings and metric events are separate objects with their own lifecycles, and
custom fields, statuses and schedules are needed to decode them.

| Bronze table | Endpoint | Mode | Purpose |
|---|---|---|---|
| `zendesk_tickets` | `incremental/tickets/cursor.json?include=metric_sets&support_type_scope=all` | incremental, cursor | Ticket snapshot, whole, including tickets handled by AI agents |
| `zendesk_ticket_audits` | `tickets/{id}/audits` | substream over changed tickets | Full history from `Create`: fields, comments, survey events |
| `zendesk_ticket_events` | `incremental/ticket_events?include=comment_events&support_type_scope=all` | incremental, time | Bulk history for increments — see [OQ-1](#oq-1-history-bulk-export-or-per-ticket-audits) |
| `zendesk_ticket_metric_events` | `incremental/ticket_metric_events` | incremental | SLA lifecycle: apply, activate, pause, fulfill, breach |
| `zendesk_satisfaction_ratings` | `satisfaction_ratings` | incremental, time | Legacy CSAT |
| `zendesk_survey_responses` | `guide/survey_responses` | incremental by `created_at_start` | Survey-based CSAT |
| `zendesk_users` | `incremental/users/cursor.json` | incremental, cursor | Agents and end users, role, suspension |
| `zendesk_organizations` | `incremental/organizations` | incremental, time | Customer accounts |
| `zendesk_groups`, `zendesk_group_memberships` | list | full refresh | Teams and who is in them |
| `zendesk_ticket_fields`, `zendesk_ticket_forms`, `zendesk_custom_statuses`, `zendesk_brands` | list | full refresh | Dictionaries to decode tickets and audits |
| `zendesk_sla_policies`, `zendesk_schedules` | list | full refresh | SLA targets and business hours |
| `zendesk_deleted_tickets` | `deleted_tickets` | full refresh | Deletion census |
| `zendesk_kb_articles` | `help_center/incremental/articles` | incremental, time | Knowledge base (Guide) |

**Throughput.** Per the vendor documentation:

| Endpoint family | Page | Requests per minute | Records per minute |
|---|---|---|---|
| `incremental/*` (tickets, ticket events, organizations) | up to 1,000 | 10 per account, shared | ≈ 10,000 |
| `incremental/ticket_metric_events` | up to 100 | same shared 10 | ≈ 1,000 |
| `incremental/users/cursor.json` | up to 1,000 | 20 (60 with High Volume), its own budget | ≈ 20,000 |
| `tickets/{id}/audits` | one ticket | the account's general limit | one ticket each |

Streams that share the incremental budget run on staggered schedules, tickets
first. The audit substream is bounded by `incremental_dependency` on the
ticket parent.

**Pagination is per endpoint, never mixed.** Cursor exports stop on
`end_of_stream` / `has_more = false`; time-based exports pass `end_time` as the
next `start_time` until `end_of_stream`, and deduplicate on the record id,
because a page boundary repeats records of the same second.

**Both CSAT generations are collected.** An account runs one at a time, but
may switch, so its history holds both. Survey responses link to a ticket
through `subject_zrns` (`zen:ticket:{id}`) and through the
`SurveyResponseSubmitted` audit event, which also carries the assignee and
group at submission. Survey CSAT needs Support Professional or Suite Growth
and above; on a lower plan the stream is empty, not an error.

**Deletions.** A deleted ticket stays in the export with status `deleted` and
its audits answer 404. The 404 is recorded as an availability event, never a
failed sync.

## 4. Custom fields

**Shape on the ticket.** `custom_fields` is an array of `{id, value}`; the
names and options live in `zendesk_ticket_fields`.

| Field type | Cardinality | Value |
|---|---|---|
| `text`, `textarea`, `regexp`, `integer`, `decimal`, `date`, `checkbox` | single | scalar |
| `tagger` (drop-down) | single | option tag |
| `multiselect` | multi | array of option tags |
| `lookup` | single | id of the referenced record |
| `multi_lookup` | multi | array of ids |

**Cardinality never changes for a field.** The vendor makes `type` writable
only on creation, so a field id keeps its type and cardinality for life; a
different type is a new field with a new id. `field_cardinality` is read from
the type and never re-derived per event.

**What does change.** Titles, and the option set of drop-downs: renaming an
option keeps its tag, deleting an option removes its value from tickets. The
vendor keeps no history of field definitions (only `updated_at`), so silver
snapshots `zendesk_ticket_fields` every sync with the `snapshot` /
`fields_history` macros already used for user profiles, and decodes each event
with the definition in force at its time.

**In history.** A custom field change is an audit `Change` with
`field_name = <field id>` and the new and previous value. The vendor
documents `tags` values as arrays; the design assumes every multi-value field
carries the full set before and after, not a delta
([OQ-3](#oq-3-multi-value-fields-in-audits)). Drop-down, multi-select and
checkbox values are mirrored into `tags`, so one edit can produce two `Change`
events; silver counts the field change and treats the matching `tags` change
as its echo. An option deletion rewrites tickets without an audit; it
surfaces as a `snapshot_diff`.

**State from history.** Because every event carries the whole value, the
current value of a field is its latest event — a fold, not delta arithmetic.
Added and removed elements of a multi-value field, when a metric needs them,
are set differences between consecutive values. `Create` lists the values a
ticket started with, so no initial state is derived backwards as Jira needs.

## 5. Silver contract

Facts and dimensions only; per-day aggregates and thresholds belong to gold.

| Class | Grain | Built from |
|---|---|---|
| `class_support_ticket_field_history` | ticket × field × event | audits / ticket events, deletions, snapshot diffs |
| `class_support_tickets` | ticket, current state | fold of history, checked against the snapshot |
| `class_support_interactions` | one message | comment events |
| `class_support_sla_events` | ticket × SLA metric × instance × event | ticket metric events |
| `class_support_sla_policies` | policy × metric × priority | SLA policies |
| `class_support_satisfaction` | one rating | legacy ratings and survey responses |
| `class_support_agents` | agent | users with an agent role |
| `class_support_groups`, `class_support_group_memberships` | group; agent × group interval | groups, membership snapshots |
| `class_support_customers` | requester or organization | end users, organizations |
| `class_support_field_definitions` | field × version | ticket field snapshots |
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
| `field_cardinality` | String | `single` or `multi`, from the field type |
| `value` | String (JSON) | New value; the whole set for multi-value fields |
| `previous_value` | String (JSON) | Value before the event |
| `event_at` | DateTime | Audit time |
| `event_order` | Int | Total order within a ticket: time, then audit id, then event id |
| `actor_id` | String | Audit author |
| `actor_kind` | String | `agent`, `ai_agent`, `end_user`, `system` (trigger, automation, API) |
| `channel` | String | `via.channel` of the audit |

### Tickets

| Field | Type | Description |
|---|---|---|
| `ticket_id` | String | Native ticket id |
| `support_type` | String | `agent` or `ai_agent` |
| `status_raw`, `status` | String | Vendor status; canonical `new`, `open`, `pending`, `on_hold`, `solved`, `closed` |
| `priority`, `type`, `channel` | String | Canonical values |
| `brand_id`, `form_id` | String | Context for drivers |
| `requester_id`, `organization_id` | String | External; never resolved to a person |
| `assignee_id`, `group_id` | String | Owner now |
| `created_at`, `solved_at`, `closed_at` | DateTime | Lifecycle |
| `availability` | String | `present`, `merged`, `deleted`, `archived` |
| `merged_into_id` | String | Target of a merge, when it can be recovered |
| `custom_attrs` | String (JSON) | Custom field id → decoded value |

### Interactions

The common denominator for tickets, chats and calls; first reply, reply
counts and one-touch resolution are computed from it.

| Field | Type | Description |
|---|---|---|
| `interaction_id` | String | Comment event id |
| `ticket_id` | String | Native ticket id |
| `author_id` | String | Writer |
| `author_kind` | String | `agent`, `ai_agent`, `end_user`, `system` |
| `is_public` | Int (bool) | Reply to the customer (1) or internal note (0) |
| `channel` | String | `email`, `web`, `chat`, `voice`, `api`, … |
| `occurred_at` | DateTime | Time |

### SLA events and policies

| Field | Type | Description |
|---|---|---|
| `ticket_id` | String | Native ticket id |
| `metric` | String | Canonical: `first_reply`, `next_reply`, `periodic_update`, `pausable_update`, `requester_wait`, `agent_work`, `resolution`, `group_ownership` |
| `instance_id` | Int | Repeat number for metrics that fire more than once per ticket |
| `event_type` | String | `apply`, `activate`, `pause`, `fulfill`, `breach`, `measure` |
| `is_cancelled` | Int (bool) | The vendor's `deleted` flag: an event later withdrawn |
| `target_minutes` | Int | Target in force |
| `business_hours` | Int (bool) | Whether the target counts business hours |
| `policy_id` | String | Applied policy |
| `occurred_at` | DateTime | Time |

`class_support_sla_policies` holds `policy_id`, `title`, `position`,
`metric`, `priority`, `target_minutes`, `business_hours`.

### Satisfaction

| Field | Type | Description |
|---|---|---|
| `rating_id` | String | Native id of the rating or survey response |
| `survey_kind` | String | `legacy` or `survey` |
| `ticket_id` | String | Rated ticket |
| `score` | String | Canonical `good`, `neutral`, `bad`, `offered` |
| `rating` | numeric | Raw rating when the scale is numeric |
| `scale_max` | Int | Top of the scale from the survey question; NULL for good/bad |
| `has_comment` | Int (bool) | Customer left text |
| `reason` | String | Vendor reason code |
| `assignee_id`, `group_id` | String | Owner when the rating was given |
| `rated_at` | DateTime | Time |

### Dimensions

- `class_support_agents`: `agent_id`, `email`, `role`, `is_active`. Resolved
  to a person through identity resolution.
- `class_support_groups` / `class_support_group_memberships`: memberships as
  intervals. Zendesk has only the current list, so the interval is built from
  per-sync snapshots (`snapshot` / `fields_history` macros) plus an absence
  check, as Jira's availability history does: `valid_from` is the
  membership's `created_at`, `valid_to` the first sync that no longer sees it.
- `class_support_customers`: `customer_id`, `kind` (`user`, `organization`),
  `organization_id`. Never resolved to a person.
- `class_support_field_definitions`: `field_id`, `type`, `title`, options,
  `valid_from` / `valid_to`.
- `class_support_schedules`: `schedule_id`, `time_zone`, weekly intervals,
  holidays — what business-hours durations are computed against.
- `class_support_kb_articles`: `article_id`, `event_type` (`created`,
  `published`, `updated`, `archived`), `author_id`, `occurred_at`.

### Operator settings

Settings Zendesk does not hold — the minimum number of ratings before CSAT is
shown for an agent or group, for example — are tenant configuration. They ship
the way task-tracking value maps do: a TSV bundle in a ConfigMap, loaded into
a `config.*` table by the umbrella hook Job, with a product default when no row
exists. Gold reads them; silver does not.

## 6. Merged tickets

**What Zendesk does.** A merge is final. Every source ticket is closed and
tagged `closed_by_merge` (some vendor articles spell it `closed_by_merged`),
and gets a comment; the target gets a comment and the sources' attachments
and users. Field values freeze on the source. Comments are not moved, and no
field links source to target — only the comment text names it.

**What silver does.** Both tickets stay as they are, with their own history.

- The source's closing audit becomes an `availability` event `merged`; the
  ticket is not counted as solved, and its resolution time is not measured.
- Interactions on the source stay on the source: the work was done there.
- `merged_into_id` is filled when the merge audit names the target; otherwise
  it is NULL, never guessed from free text.

## 7. What is reused from task tracking

- **The history shape and its tooling**: per entity × field × event rows,
  `event_kind`, `field_cardinality`, a single `event_order`, and
  `snapshot_diff` reconciliation.
- **The `snapshot` / `fields_history` macros** for what the vendor keeps no
  history of: field definitions and group memberships.
- **The ConfigMap → `config.*` mechanism** for value maps (statuses,
  priorities, custom field values) and operator settings.

The task classes themselves are not reused: their gold would read tickets as
work items, and they have no place for requester, channel, public vs internal
replies, SLA or CSAT.

## 8. Invariants

- An agent's activity never includes `end_user` or `system` events;
  `ai_agent` activity is kept and reported apart from people.
- `requester_id` and `organization_id` never join to people.
- A merged or deleted ticket is never counted as solved.
- A reopen is a `status` change out of `solved` into an open category.
- A `tags` change that mirrors a custom field change in the same audit is not
  a second update.
- A custom field event is decoded with the field definition in force at the
  event's time.

## 9. Sources that shape the contract

The contract is fixed only after it fits sources with different models, not
only Zendesk look-alikes.

| Source | What it tests |
|---|---|
| Jira Service Management | Tracker-based queues, request types, SLA goals and cycles |
| Intercom | Conversation instead of ticket; bots and AI agents as authors |
| ServiceNow | ITSM: incidents and requests, `sys_audit` history, `task_sla` |
| Salesforce Service Cloud | Case field history, entitlements and milestones as SLA |
| Freshdesk | Near-isomorphic to Zendesk — a cheap check against vendor lock-in |
| HubSpot Service Hub | Tickets inside a CRM already planned for sales |
| Aircall, Zendesk Talk | Voice interactions: duration, missed calls |

Minimum set before freezing the contract: Zendesk, Jira Service Management,
Intercom, and ServiceNow or Salesforce.

## 10. Gap to the shipped connector

| Area | Shipped | Target |
|---|---|---|
| Ticket payload | selected columns; custom fields dropped; AI-agent tickets excluded | whole object, all support types |
| Audit events | classified into four counters; values dropped | full field history with previous values |
| SLA | none | policies + metric events |
| CSAT | legacy ratings only | legacy ratings + survey responses |
| Users | agents only, full refresh | all users, incremental export |
| Groups, organizations, fields, statuses, schedules | none | dictionary streams with history kept in silver |
| Knowledge base | none | Guide articles |
| Silver | `class_support_activity` (person × day) + two dims | fact and dimension classes above; aggregates move to gold |
| Gold | none | out of scope here |

## 11. Deferred: support queues in a tracker

A team may run support as a project in Jira or YouTrack instead of a helpdesk.
No such case is planned now. When one appears, the intended route is a
projection of `class_task_*` rows into `class_support_*` for projects the
tenant marks as support, through operator configuration, not a second
connector. Open at that point: who marks a project, how it relates to the
taxonomy constructor (#2281), and what stands in for CSAT and SLA where the
tracker has neither.

## Open Questions

### OQ-1: History, bulk export or per-ticket audits

Per-ticket audits cost one request per changed ticket but give the whole
history from `Create`. `incremental/ticket_events` returns up to 1,000 events
a page across all tickets, but shares the 10-a-minute budget and starts at
`start_time`, so tickets older than the window have no `Create`.

| Option | Backfill | Increments | Cost |
|---|---|---|---|
| A. Audits only | audits | audits | one request per changed ticket, every sync |
| B. Audits once, then bulk | audits for every ticket once | `ticket_events` | bulk pages; one audit request per ticket ever |
| C. Bulk only | `ticket_events` from `start_date`, initial state derived backwards | `ticket_events` | cheapest; Jira-style synthetic initial |
| D. Bulk as change detector | audits | audits only for tickets the bulk export names | as A, minus the ticket export as parent |

Leaning to B. Blocking question: the layout of a `Change` child event in
`ticket_events` (whether the field name is a key or `field_name`) and whether
it carries the same events as audits — to compare on a test account.

### OQ-2: How a merge audit names its target

Whether the source's closing audit carries a structured reference to the
target (in `via` or an event), or only the comment text, decides whether
`merged_into_id` can be filled. To check on a test account.

### OQ-3: Multi-value fields in audits

The vendor reference shows arrays for `tags` but does not say whether a
`multiselect` or `multi_lookup` change carries the full set or only the
difference, nor whether its `tags` echo lands in the same audit. Both decide
how the fold reads multi-value fields — to check on a test account.
