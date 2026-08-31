# Technical Design — Projects V2 Boards

Collecting GitHub Projects V2 board data, and projecting a board's status column
into the task class as its own field. Phase 2 of
[GitHub Issues as a task tracker](../feature-github-issues/DESIGN.md), whose
§11.1 records the API capabilities this design builds on; nothing here re-opens
those findings.

<!-- toc -->

- [1. Scope](#1-scope)
- [2. Streams](#2-streams)
  - [2.1 The board enumerator](#21-the-board-enumerator)
  - [2.2 Field catalogue](#22-field-catalogue)
  - [2.3 Cards](#23-cards)
  - [2.4 Timeline additions](#24-timeline-additions)
- [3. Where the history of a board lives](#3-where-the-history-of-a-board-lives)
- [4. The incremental filter and its silent failure](#4-the-incremental-filter-and-its-silent-failure)
- [5. Identity: what a board status is called](#5-identity-what-a-board-status-is-called)
- [6. Board status is not the issue's state](#6-board-status-is-not-the-issues-state)
- [7. Binding coverage is not enforced yet](#7-binding-coverage-is-not-enforced-yet)
- [8. Silver](#8-silver)
- [9. Rollout](#9-rollout)
- [10. Deferred](#10-deferred)

<!-- tocstop -->

## 1. Scope

**In.** Board field definitions with their select options; card snapshots with
their current field values; board status history per board, retroactively
recoverable from issue timelines; board membership history. A board's status
column reaches `class_task_field_history` as its own field and becomes bindable.

**Out.** History of non-status board fields — no API exposes it. Webhooks. Any
mechanism that combines a board's status with the issue's own lifecycle.

## 2. Streams

### 2.1 The board enumerator

`projects_all_parent` in `definitions` returns `{id, number}` per board and
nothing else. The existing `projects_v2` stream carries the board header and
would have been re-requested once per child; the two board substreams partition
over the slim enumerator instead. Closed boards are kept: a closed board holds
history and never changes again, so skipping it loses the record without saving
a sync.

### 2.2 Field catalogue

`project_fields`, one row per (board, field, collection day). Every member type
of `ProjectV2FieldConfiguration` is spelled out — the union exposes no interface
carrying `id`/`name` in common, the same shape `issue_fields` already uses.
Select options are hoisted to `options_json`; a multi-select names them
`multiSelectOptions` rather than `options`, and both land in the same column.

`is_mirror` states whether a field is board data or a projection of an issue
property. It is computed from an **allow-list** of authored types (`TEXT`,
`SINGLE_SELECT`, `MULTI_SELECT`, `NUMBER`, `DATE`, `ITERATION`) so a mirror type
GitHub adds later defaults to excluded rather than to competing with the issue's
own field for the same fact.

### 2.3 Cards

`project_items`, one row per (card, day the card changed), carrying the card's
current field values as `field_values_json`. Draft cards are filtered out: they
have no issue behind them and no identity outside the board.

### 2.4 Timeline additions

`issue_timeline_events` gains `project_id`, `project_number` and
`was_automated`, and collects `AddedToProjectV2Event` and
`RemovedFromProjectV2Event`. The board discriminator is what makes a status
event resolvable at all: every board defines its own status field, and an issue
on several boards interleaves all their status events in this one timeline.

## 3. Where the history of a board lives

GitHub keeps no history of a board field, an option rename, or a non-status
card value. `ProjectV2Item` and each field value carry `updatedAt`, which says
when a value was last touched and never what it was before. A succession of
observations is therefore the only possible record.

Both bronze relations are keyed on the ENTITY — a board field, a card — so the
`ReplacingMergeTree` collapses each re-collection onto the current state, and
the record of change is built above them by the shared SCD2 macros:
`github__project_fields_snapshot` and `github__project_items_snapshot` append a
version only when a tracked column actually moves, and
`github__project_fields_history` turns the field catalogue's versions into a
per-attribute change log.

An earlier revision of this design put the collection day into `unique_key`
instead. That defeats the collapse: bronze stops holding the present and
accumulates one row per entity per day whether anything changed or not, which
is what `union_by_tag` warns against in as many words — *never encode the
version into `unique_key`* (ADR-0001, ADR-0004). The macros already existed and
do the job better, because they write nothing on a day when nothing changed.

## 4. The incremental filter and its silent failure

`ProjectV2.items` accepts a `query` argument that filters server-side on the
card's update date, so no full board re-fetch is needed. Two properties of that
filter drive the design:

- **Granularity is a day.** A full ISO timestamp returns zero rows. The cursor's
  `datetime_format` is `%Y-%m-%d`, which is also what renders into the query
  string, so a timestamp cannot reach it.
- **A rejected qualifier is indistinguishable from no changes.** It returns zero
  rows with no GraphQL error: green sync, empty stream. `assert_boards_yield_cards`
  guards the outcome — a source holding board field definitions and board status
  events but no cards at all is the shape a rejected filter leaves behind, and
  none of the three conditions holds for an organization with genuinely empty
  boards.

`lookback_window` is one day, so a day is re-read every sync. That is deliberate
rather than tolerated: the row key carries the day the card changed, so the
re-read rewrites the same row.

A client-side cut on top of the server filter is NOT used. The CDK applies
`is_client_side_incremental` in the record selector, before the transformations
that rename `updatedAt` to `updated_at` — the cursor field it looks for does not
exist yet, and every record is dropped silently. State observation runs after
the transformations, which is why the cursor field is the bronze column name.

## 5. Identity: what a board status is called

A status event states the board and two column **names**. It names no field
object, and there is no option identifier anywhere in it — while a card snapshot
carries `optionId` and no history. So the two sides of the same fact arrive
keyed differently, and neither key is usable on its own:

- Option identifiers are inherited from the board template, so boards created
  from it share identifiers and then rename the columns independently. The same
  identifier can read as one column name on one board and an unrelated one on
  another.
- Column names are only unique within a board.

The board is the one thing every board status fact carries, so the board is the
identity:

| | |
|---|---|
| `field_id` | `project_status:<project node id>` |
| `value_id` | `<project node id>:<lower-cased column name>` |
| `value_display` | the column name as the board states it |

The value key has to carry the board as well, not just the field key:
`class_task_statuses.status_id` is unique per source and gold joins
`field_history.value_ids[1]` to it, so a bare column name would collapse two
boards' same-named columns into one dimension row. Lower-cased because a board
states its own casing and two spellings of one column are one column.

A rename produces a new value key and needs a new mapping row. That is what the
configuration tables' validity axis is for, and the catalogue's snapshot history
is what still resolves the old name for events that carry it.

## 6. Board status is not the issue's state

An issue is open or closed. A board column is a separate field, one per board.
Nothing merges them: they reach silver as different `field_id` values, they are
bound separately, and which of them feeds a measure is the operator's binding.

This is not merely a convention — merging is currently a **defect**, because
nothing chooses between two fields bound to one role. `task_field_roles_current`
emits one row per field, every consumer joins it by role and takes what comes
back, and `config.task_field_roles.precedence` is read by nothing. Two fields
bound to `status` therefore interleave their timelines into one ordered event
array and produce spans belonging to neither. `assert_one_field_per_role`
refuses that configuration until precedence is honoured.

## 7. Binding coverage is not enforced yet

Boards multiply the field set: one status field per board, so enforcing binding
coverage means one authored row per board — and the configuration tables are
written by hand, with no authoring surface. `task_board_bindings_enforced`
(default false) therefore excludes board fields from
`assert_every_field_is_bound_or_ignored` and `assert_bound_values_are_mapped`.

It relaxes the reporting, never the invariant. An unbound or unmapped board
value still produces no dimension row and still reaches no category — it goes
unreported instead of failing the build. A board an operator has bound works
exactly as any other field; a board nobody has touched is inert.

## 8. Silver

- `github__task_field_history` gains board status changelog rows through the
  same event pipeline as every other field. Board status is excluded from
  initial-value synthesis: an initial row is dated at the issue's creation, but
  a card can join a board long after, so synthesising one would fabricate a
  span the issue never had. Nothing is lost — adding a card emits its own status
  event whose previous value is empty.
- `github__task_statuses` admits both families, keyed on `value_id` alone; a
  board value already carries its board, so they cannot collide.
- `github__task_field_metadata` declares one bindable board status per board
  that actually defines a select column, with `project_key` recording the board.
  A board with no select field can never emit a status event, so declaring one
  for it would invite a binding that resolves nothing.
- Membership events reach bronze and stop there. The class contract has no
  membership field, and inventing one is not needed by any measure yet.

## 9. Rollout

New streams have no state; their first sync is complete by definition. The two
board streams create their own bronze relations on first sync, and the bronze
reconciler adds the three new `issue_timeline_events` columns.

Those three columns are populated only for events collected after the change.
Issue timelines are retained indefinitely, so clearing the
`issue_timeline_events` cursor and re-walking recovers the whole board status
history — the same rollout step phase 1 documents in its §9, with the same
unverified item: confirm whether the Airbyte version in use truncates the
destination stream on a state clear. Card field values cannot be backfilled.

## 10. Deferred

- **Honouring `precedence`**, which is what would let an issue's own lifecycle
  and a board column coexist as status sources instead of being mutually
  exclusive.
- **An authoring surface for bindings**, without which enforcing board coverage
  is not reasonable. A generator that emits reviewable rows from the collected
  catalogue is the cheap version.
- **Board views and automation definitions.** `ProjectV2.views` carries an
  operator-facing layout; workflow definitions likely need elevated permission.
- **Membership in silver**, if a measure ever wants "which boards was this on".
- **Iteration field values** are collected but nothing reads them.
- **Per-board-field card history.** `github__project_items_snapshot` versions
  the whole `field_values_json` blob, so a change is visible but not attributed
  to a single board field. Splitting it belongs with the first consumer.
