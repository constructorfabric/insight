# Technical Design — GitHub Issues as a Task Tracker

Routing GitHub Issues through the existing silver task class contract so the Task
Delivery metrics work for a second tracker, without Projects v2 boards and
without a second enrich binary.

<!-- toc -->

- [1. Scope](#1-scope)
  - [1.1 Phase Boundary](#11-phase-boundary)
  - [1.2 Why Boards Are Out of Scope](#12-why-boards-are-out-of-scope)
  - [1.3 Measures Produced and Withheld](#13-measures-produced-and-withheld)
- [2. Connector Changes](#2-connector-changes)
  - [2.1 Stop Discarding Native Field Values](#21-stop-discarding-native-field-values)
  - [2.2 Request Identifiers in the Timeline Query](#22-request-identifiers-in-the-timeline-query)
  - [2.3 Organization Issue Fields Stream](#23-organization-issue-fields-stream)
  - [2.4 Organization Issue Types Stream](#24-organization-issue-types-stream)
  - [2.5 Descriptor and Bronze Schema](#25-descriptor-and-bronze-schema)
- [3. Bronze into Silver](#3-bronze-into-silver)
  - [3.1 Model Map](#31-model-map)
  - [3.2 Field Encoding](#32-field-encoding)
  - [3.3 Deriving Initial State](#33-deriving-initial-state)
  - [3.4 Row Shape per Issue](#34-row-shape-per-issue)
  - [3.5 Assignee](#35-assignee)
  - [3.6 Users and Identity](#36-users-and-identity)
- [4. Configuration Schema](#4-configuration-schema)
  - [4.1 Tables](#41-tables)
  - [4.2 Design Choices](#42-design-choices)
  - [4.3 Units and Where the Math Lives](#43-units-and-where-the-math-lives)
  - [4.4 Reading a Mapping](#44-reading-a-mapping)
  - [4.5 Feeding the Class Dimensions](#45-feeding-the-class-dimensions)
  - [4.6 Ownership](#46-ownership)
- [5. Making Gold Source-Neutral](#5-making-gold-source-neutral)
- [6. The Field Metadata Catalogue](#6-the-field-metadata-catalogue)
- [7. Supporting Work](#7-supporting-work)
- [8. Work Order](#8-work-order)
- [9. Rollout on an Existing Installation](#9-rollout-on-an-existing-installation)
- [10. Decisions Taken](#10-decisions-taken)
- [11. Deferred to Phase 2](#11-deferred-to-phase-2)
  - [11.1 Projects v2 Boards](#111-projects-v2-boards)
  - [11.2 Multi-Value Folding](#112-multi-value-folding)
  - [11.3 Timeline Coverage](#113-timeline-coverage)
  - [11.4 Native Issue Fields](#114-native-issue-fields)
  - [11.5 Configuration Gaps](#115-configuration-gaps)
  - [11.6 Pipeline Work](#116-pipeline-work)
  - [11.7 Cross-Source Semantics](#117-cross-source-semantics)

<!-- tocstop -->

## 1. Scope

### 1.1 Phase Boundary

Phase 1 derives the task lifecycle from the issue itself: `state`, assignee,
issue type, and the organization's native issue fields. Projects v2 boards are
excluded entirely.

| | |
|---|---|
| **In** | Closure and reopen history, assignee attribution, issue type, due date, effort estimate |
| **Out** | Board membership, board Status, iterations, any per-project field |
| **New parts** | Two connector streams, one hoisted bronze column, six staging models, two configuration tables |
| **Not needed** | A Rust enrich binary, a board discriminator, option-identifier mapping, a project-side cursor |

### 1.2 Why Boards Are Out of Scope

Closure is the signal every Task Delivery measure hangs on, and the issue's own
`ClosedEvent` and `ReopenedEvent` carry it for every issue — not only those
somebody placed on a board. Where an automation links a board column to the
issue state, the close and the board transition land within seconds of each
other, so reading the issue rather than the card loses nothing.

Board coverage is also a subset of the tracker by construction: issues nobody
added to a project have no board history at all. A measure computed only over
boarded issues is a measure over a self-selected population.

### 1.3 Measures Produced and Withheld

`state` is binary, so there is no `in_progress` lifecycle category. These
measures are not produced for GitHub: `dev_time_hours`, `pickup_days`,
`flow_dev_seconds`, `flow_lead_seconds`, `in_progress_seconds`,
`worklog_seconds`.

Two more are withheld for a different reason. `estimation_error_pct` and
`estimation_samples` compare an estimate against time actually spent, and GitHub
has no spent-time field at all — gold's estimation branch already requires
`time_spent_seconds IS NOT NULL`, so it produces nothing rather than something
wrong.

> **Do not map `open` to `in_progress`.** Gold would then compute
> `dev_time_hours` as calendar time from creation to close — a plausible-looking
> duplicate of `resolution_days`. Map `open` to `new`; `dev_seconds` stays zero
> and gold's existing `if(dev_seconds > 0, …)` guard omits the measure instead
> of inventing it.

Produced: `tasks_closed`, `bugs_fixed`, `closed_non_bug`, `resolution_days`,
`close_events`, `reopened_within_14d`, `due_date_on_time`, `due_date_with_due`,
`slip_days_total`, `late_count`, and `stale_in_progress` subject to the caveat
in [section 5](#5-making-gold-source-neutral).

## 2. Connector Changes

### 2.1 Stop Discarding Native Field Values

The REST issues payload carries `issue_field_values`, and the manifest's
`RemoveFields` deletes it without hoisting anything out of it. This is the only
source of the *current* value of a field set at creation and never changed,
which is the majority of them.

Hoist it into a bronze column as a JSON string, following the convention already
used for `label_names` and `assignee_ids`, then drop the nested original.

The payload's shape matters, because it does not agree with the timeline's. Each
entry states `issue_field_id` (a numeric identifier), `issue_field_name`,
`data_type` and `value` — and a `node_id` that names the *value*, not the field.
The timeline names the same field by its field node id. The two never meet
directly; [section 2.3](#23-organization-issue-fields-stream) carries the bridge.

Hoist `type.node_id` from the same payload while there. The type is currently
taken by display name alone, and a rename orphans whatever was bound to it.

### 2.2 Request Identifiers in the Timeline Query

`IssueFieldChangedEvent.issueField` is queried for `name` only. Add `id` to every
inline fragment so history keys on the stable vendor identifier rather than a
renameable label. Apply the same to `IssueTypeChangedEvent`: request the type's
`id` alongside its name.

While in that query, add `previousOptions` and `newOptions`. A multi-select field
change carries its values there and nowhere else; with only the two scalar
fields requested, such a change reaches bronze as an event with an empty before
and after. No multi-select issue field exists in the organization today, which
makes this cheap insurance rather than a fix.

### 2.3 Organization Issue Fields Stream

GraphQL, partitioned by organization exactly as `projects_v2` is, and paginated
the same way — a catalogue larger than one page must not truncate silently, since
the whole point of the stream is that every identifier history can name is
present.

```graphql
organization(login: $org) {
  issueFields(first: 100, after: $cursor) {
    pageInfo { hasNextPage endCursor }
    nodes {
      __typename
      ... on IssueFieldSingleSelect { id name dataType options { id name } }
      ... on IssueFieldMultiSelect  { id name dataType options { id name } }
      ... on IssueFieldNumber       { id name dataType }
      ... on IssueFieldDate         { id name dataType }
      ... on IssueFieldText         { id name dataType }
    }
  }
}
```

`fullDatabaseId` is the bridge, and it is the reason this stream is not
optional. It equals the `issue_field_id` the REST issue payload states, while
`id` equals what the timeline states — so a snapshot value and a change event
resolve to one catalogue row without ever matching on a display name. Verified
against a live organization: the numeric identifier a REST payload reports for a
field is exactly the `fullDatabaseId` the catalogue reports for it.

This is also the catalogue an operator reads when authoring role bindings, and
the input to the silver field metadata class.

### 2.4 Organization Issue Types Stream

`organization.issueTypes` returning `id`, `name`, `description`. Bronze stores
the issue type as a bare display name with no identifier today, so a rename
silently orphans every mapping. The catalogue supplies a stable key.

### 2.5 Descriptor and Bronze Schema

New streams need their bronze DDL added to the connectors-ddl snapshot and the
connector descriptor version bumped. The descriptor bump lands post-merge, so a
green pull request does not prove the deployed descriptor moved — verify after
merge.

Projects v2 streams and the `PROJECT_V2_ITEM_STATUS_CHANGED_EVENT` timeline item
stay exactly as they are. They feed `class_git_item_events` today and are phase
2's raw material.

## 3. Bronze into Silver

### 3.1 Model Map

```text
bronze_github.issues ─────────────┐
bronze_github.issue_timeline_events ─┼─▶ github__task_field_history ──▶ silver.class_task_field_history
bronze_github.issue_fields ────────┐ │
bronze_github.issue_types ─────────┤ └─▶ github__task_field_metadata ─▶ silver.class_task_field_metadata
config.task_value_map ─────────────┼───▶ github__task_statuses ───────▶ silver.class_task_statuses
                                   └───▶ github__task_issuetypes ─────▶ silver.class_task_issuetypes
staging.github__account_emails ────────▶ github__task_users ──────────▶ silver.class_task_users
```

Every staging model carries a `silver:class_task_*` tag and `union_by_tag` picks
it up. The silver models themselves need no edit and Jira is untouched.

### 3.2 Field Encoding

GitHub publishes no field catalogue for the properties that live on the issue
itself, so those take the vendor's own API names as their `field_id`. Fields the
organization defines take their real GraphQL node identifiers.

| `field_id` | Source | `value_ids[1]` | Cardinality |
|---|---|---|---|
| `created` | sentinel, not a field | — | — |
| `state` | `ClosedEvent`, `ReopenedEvent`, snapshot `state` and `state_reason` | `open`, `closed:completed`, `closed:not_planned`, `closed:duplicate` | single |
| `assignees` | `AssignedEvent`, `UnassignedEvent`, snapshot `assignee_ids` | numeric account id as text, empty when unassigned | single, see 3.5 |
| `type` | `IssueTypeChangedEvent`, snapshot `type.node_id` | issue type node id | single |
| `IFD_…` | `IssueFieldChangedEvent`, hoisted field values | ISO date | single |
| `IFN_…` | same | number as text | single |
| `labels` | `LabeledEvent`, `UnlabeledEvent` | label name | multi |

One value needs normalising. GraphQL states a closure reason as an upper-case
enum (`COMPLETED`) while the REST snapshot states it lower-case (`completed`) —
the same lifecycle value in two spellings. Left alone, the initial row and the
change row for one issue disagree, and an operator has to bind six values where
there are three. The staging model lower-cases both sides.

`field_id` for the native fields is the field's node id, resolved for snapshot
values through the catalogue's `fullDatabaseId`. The identifier a value carries
depends on which side it came from; the catalogue is what makes that invisible
downstream.

The composite `state:state_reason` value is deliberate. It makes "closed as not
planned" a distinct, enumerable value that an operator maps to a lifecycle
category in configuration, rather than a policy decision frozen into SQL.

### 3.3 Deriving Initial State

A timeline records changes, so any field set at creation and never touched is
absent from it. Every synthetic-initial row comes from one rule, uniform across
all fields:

```text
initial_value =
    if the field has events  -> argMin(prev_value, event_at)
    otherwise                -> the snapshot value
```

No reverse-fold, no running state, no Rust binary. This works because GitHub's
field-change events carry the complete previous value rather than a delta, and
because `state` is trivially `open` at creation.

### 3.4 Row Shape per Issue

1. One creation marker: `field_id='created'`, `event_kind='synthetic_initial'`,
   `_seq=0`, `event_at` set to the issue's creation time, author set to its
   author. Gold reads this as `created_at`.
2. One `synthetic_initial` row per role-bearing field present at creation,
   `_seq` running 1..N in `field_id` order.
3. One `changelog` row per timeline event, `_seq=0`, ordered by `event_at`.

`unique_key` follows the project convention
`{insight_source_id}-{data_source}-{id_readable}-{field_id}-{event_id}`, with
`event_id = initial:{issue_id}` for synthetic rows per ADR-005. `id_readable` is
`owner/repo#number`.

Build the history model as a full table first. Initial rows depend on an issue's
whole event set, so an incremental model must reprocess entire issues rather
than new events — worth doing, but not before the model is proven correct. When
it becomes incremental, key `delete+insert` on the issue, not on `unique_key`.

### 3.5 Assignee

Gold reads `value_ids[1]` and wants one owner, while GitHub allows several. The
phase 1 rule:

- each `AssignedEvent` emits a `set` row carrying that account id;
- each `UnassignedEvent` emits a `set` row carrying an empty value;
- an issue whose snapshot has an assignee but no assignment event gets a
  `synthetic_initial` row carrying the first snapshot assignee.

The value is the numeric account identifier as text, not the login. The identity
bridge already keys on that identifier, and a login can be changed by the person
who holds it.

Gold's latest-wins pivot then yields the most recently assigned person, and an
empty value falls out of the users join, correctly leaving an unassigned issue
unattributed.

Known limitation: removing one of two assignees reports the issue as unassigned
until someone is assigned again. Folding the real assignee set is phase 2 work.

### 3.6 Users and Identity

`github__task_users` maps account to e-mail from
`staging.github__account_emails`, emitting `user_id` as the numeric account
identifier — which is what that model keys on. Gold joins this with an **inner**
join and an `email LIKE '%@%'` filter, so an issue whose assignee has no known
e-mail disappears from every measure.

Measure that coverage before shipping. It is the single largest determinant of
how much of the tracker reaches a dashboard.

## 4. Configuration Schema

Two operator-authored tables, one row per decision. They answer what the vendor
cannot: which field plays which role, and what a value means.

### 4.1 Tables

```sql
CREATE TABLE config.task_field_roles
(
    tenant_id          String,
    insight_source_id  String,
    data_source        LowCardinality(String),
    field_id           String,
    valid_from         DateTime64(3),
    recorded_at        DateTime64(3),
    unique_key         String DEFAULT concat(tenant_id, ':', insight_source_id, ':',
                                             data_source, ':', field_id, ':',
                                             toString(valid_from), ':', toString(recorded_at)),
    role               LowCardinality(String),
    precedence         UInt8,
    value_unit         LowCardinality(String),
    unit_multiplier    Float64,
    is_deleted         UInt8,
    note               String,
    recorded_by        String,
    _version           DateTime64(3) DEFAULT now64(3)
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key;
```

```sql
CREATE TABLE config.task_value_map
(
    tenant_id          String,
    insight_source_id  String,
    data_source        LowCardinality(String),
    field_id           String,
    value_id           String,
    valid_from         DateTime64(3),
    recorded_at        DateTime64(3),
    unique_key         String DEFAULT concat(tenant_id, ':', insight_source_id, ':',
                                             data_source, ':', field_id, ':', value_id, ':',
                                             toString(valid_from), ':', toString(recorded_at)),
    canonical_value    LowCardinality(String),
    value_display      String,
    is_deleted         UInt8,
    note               String,
    recorded_by        String,
    _version           DateTime64(3) DEFAULT now64(3)
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key;
```

`unique_key` carries a `DEFAULT` expression so a hand-authored `INSERT` lists only
the business columns. Verify on a local stand that the ClickHouse version in use
accepts a defaulted column in `ORDER BY`; if it does not, compute the key in the
insert statement instead.

### 4.2 Design Choices

- **ReplacingMergeTree keyed on the full decision.** Because `recorded_at` is
  part of `unique_key`, deduplication only ever collapses the *same* decision
  written twice — an accidental re-run of a seed, a retried statement. Every
  distinct decision, including a correction of an earlier one, is a distinct key
  and survives. That preserves the journal while making inserts idempotent, and
  it keeps these relations on the same engine and sorting convention as every
  other table in the warehouse rather than making them a special case.
- **No `valid_to`.** A period ends where the next begins. Gaps and overlaps
  become unrepresentable and no operator has to remember to close anything.
- **Two time axes.** `valid_from` says which events a mapping applies to — the
  process genuinely changed on a date. `recorded_at` says when the decision was
  made — the mapping was wrong and history is being corrected. One axis cannot
  express both, and conflating them means either rewriting the past when you
  should not, or failing to when you should.
- **`role` is a low-cardinality string, not an enum.** Adding a role to an enum
  is a schema migration on an operator-facing table, for a value gold matches by
  content anyway. A low-cardinality string stores identically and adds none of
  that friction. The cost is that the database no longer constrains the domain,
  which makes the third coverage test in [section 7](#7-supporting-work)
  load-bearing rather than a formality.
- **Unit is declared, not just scaled.** See [4.3](#43-units-and-where-the-math-lives).

### 4.3 Units and Where the Math Lives

A scalar multiplier converts one numeric unit into another. It cannot parse a
date format, split `3h 20m` into seconds, or turn a t-shirt size into a number —
and, most importantly, it cannot make story points comparable to time, because
those units are not commensurable at all.

Rather than grow an expression language in a configuration column, the work
splits by who can possibly know the answer:

| Concern | Belongs in | Why |
|---|---|---|
| Parsing, extraction, type shaping — reading a number out of JSON, normalising a date format, decomposing a duration string | The per-source staging projection | It differs by vendor, not by customer. It is code: reviewed, typed, and covered by tests. This is already where each source's reconciliation lives. |
| Value-to-value lookups — a select option that stands for a number or a category | `config.task_value_map` | Already a value-keyed mapping table; a t-shirt size mapping to a number is the same shape as a column mapping to a lifecycle category. |
| Instance-specific scalars — how many seconds a given organization counts in a man-day | `unit_multiplier` | Genuinely a local convention that neither the vendor nor the code can know. |
| Whether two quantities are comparable at all | `value_unit` | A declaration, not a computation. |

`value_unit` names the unit the field's values are expressed in — `seconds`,
`man_days`, `points`, `date`, `none`. Gold converts to the role's canonical unit
only when the declared unit is convertible, and emits nothing when it is not.
This is the whole point of declaring the unit rather than only the factor: an
estimate in story points against time spent must produce no estimation measure,
not a plausible-looking ratio.

If a source ever needs arithmetic beyond this — a computed field, a conditional
derivation — it belongs in that source's staging model, where it can be read and
tested, not in a string evaluated at runtime.

### 4.4 Reading a Mapping

```sql
CREATE VIEW config.task_value_map_current AS
SELECT
    tenant_id, insight_source_id, data_source, field_id, value_id,
    canonical_value, valid_from,
    ifNull(
        leadInFrame(valid_from) OVER (
            PARTITION BY tenant_id, insight_source_id, data_source, field_id, value_id
            ORDER BY valid_from
            ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING),
        toDateTime64('2999-01-01 00:00:00', 3)
    ) AS valid_to
FROM config.task_value_map
WHERE is_deleted = 0
ORDER BY valid_from, recorded_at DESC
LIMIT 1 BY tenant_id, insight_source_id, data_source, field_id, value_id, valid_from;
```

### 4.5 Feeding the Class Dimensions

Configuration does not bypass the existing contract, it feeds it.
`github__task_statuses` reads the value map for the field bound to `status` and
emits `(status_id, status_category)`; `github__task_issuetypes` does the same for
`issue_kind`. Gold keeps joining the status and issue-type classes as it does
now.

Jira keeps deriving its categories from `statusCategory` and needs no
configuration. Where an authored row exists for a Jira field it wins —
`coalesce(authored, derived)` — which gives an override channel for a status
category an administrator assigned badly.

**Phase 1 resolves the mapping as of the build, not as of the event.** The
configuration relations stay bitemporal and the journal is written in full, but
the projections read the row in force now and apply it to all history — the
correction semantics, which is the common case.

Carrying validity into gold would mean giving `class_task_statuses` and
`class_task_issuetypes` a `valid_from` / `valid_to` grain, and `union_by_tag`
selects every column of every branch: the columns would have to appear on the
Jira projections too, the existing incremental silver relations would need a
schema change or a rebuild, and four join sites in gold would need a range
predicate. That is a real cost, and phase 1 has nothing to spend it on — the
whole GitHub status vocabulary is five values whose meaning does not migrate.

Deferred to [section 11.5](#115-configuration-gaps). It becomes worth paying for
when boards arrive and a board column's meaning genuinely changes on a date.

### 4.6 Ownership

The `config` relations must not be dbt models — dbt would recreate and wipe
them. They are created by migration alongside the connectors-ddl snapshot and
declared to dbt as sources, the same arrangement as the Rust-owned
`staging.jira__task_field_history`.

## 5. Making Gold Source-Neutral

Gold matches vendor field identifiers as string literals — eleven of them across
two models. Those literals are Jira's, which couples gold to one vendor in
contradiction of its own contract, where `field_id` is documented as
vendor-specific.

| Model | Today | After |
|---|---|---|
| `task_issue_state` | `field_id = 'status'` and five siblings, plus an `IN (…)` filter | `role = 'status'`, via a join to the role bindings |
| `task_status_spans` | `field_id = 'status'` | `role = 'status'` |
| `task_issue_state` | estimate and spent read as raw seconds | converted per `value_unit` and `unit_multiplier`, or left null when the unit is not convertible |
| `task_metric_evidence` | dimensions carry issue type only | dimensions also carry `data_source` |

The role binding arrives through a small ephemeral model — one row per
`(insight_source_id, field_id)` resolved at build time, carrying `role`,
`precedence` and `unit_multiplier`. It is small enough to join freely.

> **Blocking.** `task_metric_evidence` hard-codes `source_key='task'` and carries
> no source dimension. Without the `data_source` dimension, GitHub issues merge
> into the same per-person Task Delivery numbers as Jira with no way to separate
> them: a person working in both trackers gets a blended figure, and an issue
> mirrored between trackers is counted twice. Ship the dimension before the
> first GitHub row reaches gold.

`stale_in_progress` needs a decision rather than a change. Its SQL condition is
"not done and untouched for a fortnight", which for a source without an
`in_progress` category means "open and untouched" — a defensible measure under a
misleading name. Either rename it or suppress it for sources lacking the
category; the source dimension at least keeps the two populations separable.

## 6. The Field Metadata Catalogue

The silver field metadata class is used, and GitHub should contribute to it even
though nothing consumes GitHub's rows on day one.

**What it is.** A vendor-derived catalogue: which fields exist in this instance,
their cardinality, whether their values carry opaque identifiers or string
literals. It describes *what is there*. It deliberately does not say *what
anything means* — that is the configuration tables' job, and the distinction is
why the two are separate.

**Who reads it.** Today only `jira-enrich`, which builds a field map to classify
cardinality and value type per event. Its query filters `data_source = 'jira'`,
so GitHub rows are invisible to it and no change is needed there.

**Why populate it anyway.**

- It is the list an operator works from when authoring role bindings: real
  identifiers of this instance rather than invented ones.
- The coverage tests need `is_multi` and `has_id` to validate that emitted
  history matches declared cardinality; the existing dbt tests under the task
  suite already assert exactly that.
- Leaving one source's catalogue empty makes the class contract a half-truth and
  invites the next source to skip it too.

**What GitHub contributes.**

| Row group | `field_id` | `field_type` | `is_multi` | `has_id` |
|---|---|---|---|---|
| Issue properties, constant per vendor | `state` | `state` | 0 | 0 |
| | `assignees` | `user` | 1 | 0 |
| | `type` | `issuetype` | 0 | 1 |
| | `labels` | `string` | 1 | 0 |
| Organization issue fields, from the new stream | `IFSS_…`, `IFN_…`, `IFD_…` | the GraphQL `dataType` | 1 for multi-select | 1 for select types |

`project_key` stays null throughout phase 1 — organization issue fields are not
project-scoped. The column is where phase 2's board fields will record which
board they belong to, which is why it already exists.

## 7. Supporting Work

**Split the git item events model.** It currently mixes issue and pull-request
lifecycle events, has no gold consumer, and is absent from the git silver
README. Once issue events flow into the task class it becomes a duplicate
representation of the same facts. Drop the issue branch, keep pull requests, and
the `item_type` discriminator falls away with it.

**Coverage tests that fail the build.** An unmapped value silently becomes
`undefined`, and gold treats anything not `done` as open — so a missing mapping
produces issues that never close rather than an error. Four tests close that
hole:

1. Every observed `(field_id, value_id)` whose field is bound to `status` or
   `issuetype` has a mapping valid at its `event_at`.
2. Every field carrying events, in a source that has any binding, is bound to a
   role or explicitly marked `ignored`.
3. Canonical values fall inside the domain their role permits.
4. `valid_from` is monotonic within a key and not set in the future.

This is the same rule the project already applies to required environment
configuration: no silent defaults.

**Metric tests** go in `src/ingestion/tests/e2e/metrics/` as `*.test.yaml`
against the declarative rig, one file per measure family, mirroring the existing
`tasks_*` suite that covers the Jira path. Seed `bronze_github.issues`,
`issue_timeline_events`, `issue_fields` and `issue_types` plus the roster the
identity chain needs, run the batch metrics endpoint, and assert the measures
listed in [section 1.3](#13-measures-produced-and-withheld).

Two properties of the rig make this cheaper than it looks. The seeder keys on
`<schema>.<table>` and does not care which schema that is, so the configuration
rows a GitHub fixture needs are seeded the same way the bronze rows are — no rig
change, provided the migration has created the relations. And because phase 1
adds no enrich binary, the GitHub path runs bronze to gold entirely inside the
rig: a fixture exercises the real staging models rather than starting from a
hand-written silver row.

Files to add, each asserting one thing the design claims:

| File | Proves |
|---|---|
| `github_tasks_closed.test.yaml` | closure from `state` alone, the type breakdown, the source dimension, re-sync deduplication, and the withheld measures staying absent rather than degenerating into lead time |
| `github_tasks_bugs.test.yaml` | `issue_kind` from the authored value map, and a type the map does not mention staying its own group instead of defaulting into the non-bug side |
| `github_tasks_lifecycle.test.yaml` | resolution counted from the synthesised creation marker rather than the first recorded event, and reopen rate folded out of the state history |
| `github_tasks_due_dates.test.yaml` | a due date read through a native issue field bound to the `duedate` role, across the numeric-to-node-id bridge, with an undated issue guarding the denominator |

An unmapped value failing the build belongs to the dbt coverage test rather than
here: this rig asserts API responses, and a fixture carrying an unmapped value
would fail the build instead of producing an assertion. See
[section 11.5](#115-configuration-gaps).

Duplicated bronze rows belong in the fixtures: they are what proves the
read-time deduplication holds. The last two files are the ones worth writing
first — they assert absence and failure, which is where a pipeline lies most
convincingly.

**Documentation.** Every new staging model gets a `schema.yml` entry with column
descriptions and the accepted values its enums allow, and the connector README
gains the two new streams.

## 8. Work Order

1. **Source dimension in evidence.** Independent of everything else and blocking
   everything else. Merge it alone.
2. **Connector.** Hoist the field values, add identifiers to the timeline query,
   add the two organization streams, bronze DDL, descriptor bump.
3. **Configuration tables and migration.** Empty tables plus the interval views;
   seed Jira's identity bindings so nothing changes for it.
4. **Users and metadata first.** These two models alone reveal identity coverage
   and give the operator the catalogue to author against. Stop and read the
   numbers before building history.
5. **Field history and dimensions.** The three remaining staging models,
   full-table materialization.
6. **Gold.** Role resolution replacing the literals, unit multiplier, validity
   predicates on the dimension joins.
7. **Metric fixtures** in `src/ingestion/tests/e2e/metrics/`, coverage tests, then
   the git model split.

Step 4 is a genuine checkpoint. If assignee e-mails do not resolve, the inner
join in `task_issue_state` discards those issues and everything built after it
measures a fraction of the tracker.

## 9. Rollout on an Existing Installation

A deployment that already collects GitHub needs a re-sync, because two of the
changes add data that an incremental cursor will never go back for.

| Stream | Action | Why |
|---|---|---|
| `issues` | Clear state, full re-sync | The hoisted `issue_field_values` column is populated only by a fetch. The cursor is a datetime cursor on `updated_at`, so an issue nobody touches again is never re-read — and those are precisely the issues whose field values were set at creation and never changed, which is what the column exists to capture. |
| `issue_timeline_events` | Clear state, full re-walk | Historical events carry field and type names but no identifiers. Keying history on names instead would mean two resolution paths and a silent failure whenever something is renamed. |
| `issue_fields`, `issue_types` | Nothing | New streams have no state; the first sync is complete by definition. |
| `projects_v2` and the rest | Nothing | Unchanged. |

### Order of operations

```text
merge
  -> descriptor bump lands (post-merge, not in the pull request)
  -> reconcile picks up the new manifest and the two new streams
  -> only now: clear state for issues and issue_timeline_events
  -> run the sync
  -> run the transform
```

Clearing state before the new descriptor is live re-syncs against the *old*
manifest: the full cost of the walk, none of the new data.

### What is already safe

Both bronze relations are `ReplacingMergeTree(_airbyte_extracted_at)` ordered by
`unique_key`, so a re-sync is idempotent — a re-fetched row carries a newer
extraction timestamp and wins. Nothing duplicates.

The reconciler anchors a connection's `sourceCatalogId` to the `catalogId`
returned by the latest `discover_schema`, so adding two streams does not leave
the connection stuck behind a schema-change prompt.

### Two traps

- **Clearing state through the API.** A `not_set` state value faults the
  service; an empty `streamState` object is what actually works. The repository
  already wraps the endpoint in the reconcile library.
- **The incomplete window.** Confirm whether the Airbyte version in use
  truncates the destination stream on clear. If it does, bronze is incomplete
  between the truncation and the end of the sync, and a transform running in
  that window builds silver and gold from partial data. Either guarantee the
  sync completes before the transform, or detach the transform for the run.

## 10. Decisions Taken

1. **Closure that is not delivery follows Jira.** Every `closed:*` value maps to
   `done`, including `not_planned` and `duplicate` — the same rule Jira applies
   when a "won't do" resolution still closes an issue. Consistency across
   trackers beats precision within one, and the composite value encoding means a
   deployment that disagrees changes a configuration row rather than a model.
   The consequence to state plainly: `tasks_closed` counts closures, not
   deliveries. Splitting them later means promoting `state_reason` to a
   dimension, not reworking the pipeline.
2. **Seconds per man-day is not needed yet.** The multiplier only ever feeds the
   estimation measures, and those require a spent-time value that GitHub does
   not have — `time_estimate_seconds` is consumed in exactly one place in gold,
   gated on `time_spent_seconds IS NOT NULL`. Phase 1 therefore records the
   estimate with `value_unit = 'man_days'` and leaves the multiplier unset. The
   number becomes a real decision the day a spent-equivalent or a
   points-independent measure arrives; until then, declaring the unit is enough.
3. **Multi-assignee keeps the simple rule.** Latest assignment wins,
   unassignment clears. The rule only misreports where an issue holds more than
   one assignee and one of them is removed, which is a small tail of the
   population — but see the note below about describing that tail.
4. **The git model is split in the same pull request.** Issue events are not left
   duplicated across two silver relations.
5. **Mappings apply as of the build, not as of the event.** Both time axes are
   recorded, but only the correction axis has an effect in phase 1 — see
   [section 4.5](#45-feeding-the-class-dimensions) for what the alternative
   would have cost and why nothing in phase 1 needs it.

> **Do not quantify decision 3 in the pull request description.** Repository and
> collaboration content must not carry counts, percentages or distributions
> observed in a deployed environment. Describe the condition and its blast
> radius instead: "an issue that holds several assignees and loses one is
> reported unassigned until someone is assigned again; issues with a single
> assignee, which is the ordinary case, are unaffected." Measure the prevalence
> to decide, then leave the measurement out of the write-up.

## 11. Deferred to Phase 2

Everything below was established while designing phase 1 and deliberately left
out of it. Capability claims were verified against the GraphQL schema by
introspection; they are recorded here so phase 2 does not repeat the work.

### 11.1 Projects v2 Boards

Boards are the only source of an `in_progress` lifecycle category, which is what
unlocks `dev_time_hours`, `pickup_days`, the flow measures and a truthful
`stale_in_progress`.

**Status is per-project, not a shared field.** Every project defines its own
`Status`, and two projects' same-named fields are different fields with
different identifiers. `ProjectV2FieldType` has no `STATUS` member — Status is an
ordinary `SINGLE_SELECT`, indistinguishable by type from any other select field
a board defines. Which field is the status therefore cannot be derived from the
API and must be configured per instance.

**Single-select option identifiers collide across projects.** They are inherited
from the default board template, so projects created from that template share
option identifiers and then rename them independently — the same identifier can
read as one column name in one project and a completely different one in
another. Any mapping to a lifecycle category must key on the pair
`(field_id, option_id)`, never the option identifier alone. A project created
outside the template receives fresh identifiers.

**Built-in project fields mirror issue properties.** `TITLE`, `ASSIGNEES`,
`LABELS`, `MILESTONE`, `REPOSITORY`, `LINKED_PULL_REQUESTS`, `REVIEWERS`,
`PARENT_ISSUE`, `SUB_ISSUES_PROGRESS`, `CREATED`, `UPDATED`, `CLOSED` are
projections of the issue, not board data. Filter them out by `dataType` when
reading field values, or they become a second competing source of truth for
assignee and labels. `ProjectV2SingleSelectField` also carries `isIssueField`
and `issueField`, so a project field can be backed by a native issue field.

**A board discriminator exists and is not requested.**
`ProjectV2ItemStatusChangedEvent` exposes `project` and `wasAutomated`. Adding
`project` to the timeline query is a one-line change and resolves the ambiguity
of an issue sitting on several boards, whose status events currently interleave
in one stream with no way to tell them apart.

**Board membership history is available.** `AddedToProjectV2Event` and
`RemovedFromProjectV2Event` both expose `project`, so which boards an issue was
on, and when, is fully reconstructible rather than only observable as a current
snapshot.

**Incremental collection is possible and needs its own cursor.**
`ProjectV2.items` accepts a `query` argument that filters server-side on the
card's update date: `updated:>=YYYY-MM-DD`, `updated:>DATE`, `updated:DATE..*`
and `updated:>@today-Nd` all work. Three constraints apply:

- Granularity is a day. A full ISO timestamp returns zero rows. Use a date
  cursor with one day of overlap and filter precisely on `ProjectV2Item.updatedAt`
  locally.
- An unparseable qualifier returns zero rows with **no** GraphQL error. A typo,
  or an accidental timestamp, reads as "no changes" — a green sync with no rows.
  Guard with a canary query or by building the filter string from a date only.
- `ProjectV2ItemOrderField` has a single member, `POSITION`. There is no
  server-side ordering by date, but with a working filter there is no need for
  one.

The unit of iteration is the project rather than the issue, which makes a
board-side sweep cheaper than the existing per-issue timeline walk.

**Moving a card does not bump the issue's `updated_at`.** The timeline stream is
a substream of an issue window cursored on issue `updated_at`, so board
movements on otherwise-untouched issues are not picked up. This is why board
collection needs the item cursor above rather than riding on the existing one.

**Non-status board fields have no history in any API.** No timeline event type
exists for them. `ProjectV2Item` and each field value carry `createdAt`,
`updatedAt` and `creator`, which say when a value was last touched but never
what it was before. History can only be accumulated forward by diffing
successive snapshots. Status history, by contrast, is retroactively backfillable
because issue timelines are retained indefinitely.

**Webhooks are the only true change feed.** `projects_v2_item` events carry the
field and the before/after values. They require a receiver, which the pull-based
ingestion does not have, and they are not retroactive.

**Precedence.** When an issue is on several boards, the role binding's
`precedence` column decides which board's status wins. The column exists in the
phase 1 schema for this purpose and is unused until boards arrive.

### 11.2 Multi-Value Folding

Eight issue attributes are multi-valued and arrive as add/remove deltas:
assignees, labels, sub-issues, blocked-by, blocking, Projects v2 membership,
connected items, and the set of attached issue-field definitions. Reconstructing
the state of any of them requires folding the deltas.

Only assignee matters to gold today, and phase 1 sidesteps the fold with the
latest-wins rule in [section 3.5](#35-assignee). Phase 2 should fold the real
set, both to fix the case where an issue holds several assignees and loses one —
reported unassigned until somebody is assigned again — and to make `value_ids`
truthful for multi-valued fields, which the class contract declares to be folded
state rather than a bare delta.

The fold also unblocks a question phase 1 cannot answer: when several people are
assigned, whether the metric should attribute the issue to one of them or divide
it between them. The latest-wins rule picks one by accident of ordering rather
than by policy.

A further seven attributes arrive as deltas but are single-valued, so the delta
is the state and no fold is needed: parent issue, issue type, milestone,
open/closed, locked, pinned, duplicate marking.

### 11.3 Timeline Coverage

The issue timeline union has 51 member types; the connector requests nine.
Nothing in the uncollected remainder is needed for Task Delivery, but several
carry structure that other metric groups might want: sub-issue relationships,
blocked-by and blocking edges, connected pull requests, milestone assignment,
title renames, and the issue-field attachment events.

Two attribution signals are also uncollected: `wasAutomated` on the board status
event, and the `intent` object on `IssueFieldChangedEvent`, which carries an
intent identifier, a confidence and a rationale. Together they distinguish a
change made by automation from one made by a person.

### 11.4 Native Issue Fields

Phase 1 requests `previousOptions` and `newOptions` on field-change events but
has no consumer for them, because the organization defines no multi-select issue
field. When one appears, the folding rules in
[section 11.2](#112-multi-value-folding) apply, with one simplification: unlike
labels and assignees, a native field change carries the complete option set
before and after, so no fold is required — the event is already the state.

Two organization fields exist that map to no gold role: a priority single-select
and a numeric factor field. They are carried in history and available whenever a
measure wants them.

### 11.5 Configuration Gaps

**Issue-kind lists are global.** `task_bug_type_names` and
`task_non_bug_type_names` are dbt variables scoped to the whole deployment, with
no tenant or source key. Two trackers with different type vocabularies collide,
and an unlisted type falls to `unknown`, which is excluded from both `bugs_fixed`
and `closed_non_bug` while still counting toward `tasks_closed` — the bug share
moves with no signal. Migrating these lists into the value-mapping table keyed on
the source resolves it; the current lists become the per-vendor default seed.

**Three configuration guarantees are implemented but unproven.**

The coverage test that refuses an unmapped value runs in the rig, but every
fixture maps everything it seeds, so the test passes without ever firing.
Proving it fires needs a negative harness the rig does not have: a fixture
carrying an unmapped value fails the build rather than producing an assertion,
which is the desired behaviour and the reason it cannot be asserted in the same
run. Until that exists, the guard is reasoned about rather than demonstrated.

The authored override beating a vendor-derived category — the channel for a
status an administrator filed under a category that does not match how the team
treats it — is implemented in the `coalesce(authored, derived)` rule and
exercised by nothing.

Unit conversion is unreachable: `unit_multiplier` only ever feeds the estimation
measures, and those need a spent-time value GitHub does not have. The first
source that supplies one, or the first measure that wants an estimate on its
own, makes it testable.

**Nothing resolves as of the event yet.** Phase 1 reads both the role bindings
and the value mappings as of the build, so a mapping written today applies to all
history. Making resolution temporal means widening `class_task_statuses` and
`class_task_issuetypes` to a `valid_from` / `valid_to` grain — which, because
`union_by_tag` unions every column, also touches the Jira projections and the
existing incremental relations — and adding a range predicate at four join sites
in gold. The interval view over the configuration tables is already the shape
that feeds it. This matters the first time a status column's meaning genuinely
changes on a date, or a field is repurposed, neither of which phase 1 can
express.

**Jira custom fields.** Any role beyond the six system fields means a
`customfield_NNNNN` identifier, which is instance-specific by definition and
needs the same binding machinery GitHub uses.

**Authored override for Jira status categories.** The `coalesce(authored,
derived)` rule in [section 4.5](#45-feeding-the-class-dimensions) is implemented
in phase 1 but exercised by nothing. It becomes useful the first time a
workflow status is filed under a category that does not match how the team
treats it.

### 11.6 Pipeline Work

**Incremental history.** Phase 1 materializes the field history model as a full
table for correctness. Making it incremental requires selecting issues touched in
the window and reprocessing them whole, with `delete+insert` keyed on the issue
rather than on `unique_key`.

**Status backfill.** Once board collection exists, historical board status can be
recovered by resetting the timeline stream cursor and re-walking every issue.
Field values on cards cannot be backfilled.

**Rename or suppress the stale measure.** `stale_in_progress` measures open and
untouched issues for any source lacking an `in_progress` category. Either the
name or the population should change.

**Split the git item events model**, if it was not done in phase 1.

### 11.7 Cross-Source Semantics

**Mirrored issues double-count.** The `data_source` dimension added in phase 1
lets a consumer separate trackers, but it does not deduplicate. An issue tracked
in both Jira and GitHub contributes twice to a person's totals. Detecting the
link and choosing an authoritative side is unsolved.

**Worklog has no GitHub equivalent.** `worklog_seconds`, `in_progress_seconds`
and the estimation measures remain Jira-only. Any cross-source aggregate that
mixes them is comparing populations, not people, which is an argument for
surfacing the source dimension in the product rather than only in the data.
