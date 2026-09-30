# YouTrack task Silver

The adapter consumes the Bronze connector and the shared `event_order` contract.
It needs no additional runtime: `tag:youtrack` builds staging and identity
observations; `tag:youtrack+` reaches the shared task classes and Gold.

## Cardinality and evidence

`fieldType.id` and the issue custom-field `$type` describe the field when it was
observed. The activities API does not document a historical field-type version.
In particular, an array with one member does not prove a single-valued field.

`youtrack__field_observations` materializes the versioned metadata observations
already retained by the Bronze connector. `youtrack__field_type_history` exposes successive
observations. If two observations disagree, the transition occurred at an
unknown point between them; neither endpoint is claimed as its exact time.
This is observation history, not a complete historical schema registry.

A custom-field changelog row therefore takes the cardinality of the field type
observed last at or before the activity (`[*]` is `multi`, any other type
`single`); an activity older than every observation takes the first observed
type, and a field never observed stays `unknown`. A later observation thus
never reinterprets an activity it does not precede. Snapshot rows use their own
issue-field `$type` where it names the cardinality (the `Multi*`/`Single*`
families, simple, text and period fields), including for null values, and the
same observed-type rule otherwise. A retired field keeps the cardinality its
field last had. Arrays are never truncated.

Multi-valued and `unknown` fields use remove/add set algebra, which works for
both complete before/after sets and element deltas. The first operation on each
value establishes its initial membership; snapshot members untouched by the
available history supply the remaining baseline. A `single` field is replaced,
not merged: its state before the first available change is what that change
removed, and each change sets it to what the change added — an id-merge would
keep the old value whenever the replaced text differs from it by a byte.
Missing source changes cannot be recovered by either rule.

Only the creation marker is synthesized at issue creation. Field values without
creation-time evidence are observations at collection time, not historical
initial values. A snapshot disagreement is `snapshot_diff`; a field absent from
a subsequent complete issue snapshot is `retired_field`. Identical observations
are suppressed so repeated syncs cannot move a close time forward. Comment and
work-item lifecycle events are one per observed state — a comment's
`(deleted, updated)`, a work item's `updated` — keyed and dated by that state's
first observation, so re-reading an unchanged record adds nothing.

## Ordering and rebuilds

Activity IDs remain opaque. Same-instant before/after dependencies feed
`task_instant_order`; IDs give a deterministic fallback for unconstrained or
contradictory events. That fallback is not evidence of causal order.
`task_event_order` packs the event timestamp, kind band and rank. Ranks are
checked for overflow. Initial markers precede the first recorded activity even
when it predates the issue creation timestamp.

The initial implementation rebuilds derived journals and links as tables. This
makes late events, catalogue changes and retries deterministic without an
append watermark that can miss an older event. It is a full recomputation,
not Jira's per-issue incremental optimization. Metadata and issue observations
are append-only RMT relations keyed by immutable identity and observation time.
Observation models opt out of full refresh: a derived-history rebuild must not
erase evidence Bronze no longer retains. Changing an observation schema requires
an explicit migration.

Shared task classes replace YouTrack keys that disappeared from a rebuilt
producer, and admit its complete projection independently of another source's
watermark. Other sources retain their existing incremental behavior.

## Bindings and units

Task identity and every entity relationship use source-native immutable IDs,
scoped by `insight_source_id`. `id_readable` and `target_readable` are labels for
display and user-entered lookup only. A project move may change both labels but
must not change task, comment, worklog, link or history keys. Project activity
values use the nested `IssueKey.project.id`; the enclosing IssueKey ID is not a
project identity.

Native global custom-field IDs are used consistently across the issue snapshot,
activity and catalogue. `CustomFilterField.customField.id` is requested explicitly;
older activities may use their native filter-field ID. Missing snapshot field
identities and activities without parent snapshots have explicit coverage tests.

Use `config.task_field_roles` for field roles, `config.task_value_map` for status
categories, and `config.field_value_map` / `field_value_defaults` for issue kinds.
Only the fixed `summary` property has a built-in role. Field names do not infer
roles. The same global field cannot have different project-specific bindings in
the current config contract.

Period values retain minutes. Bind them with `value_unit='minutes'` and
`unit_multiplier=60`; work item durations are converted directly to seconds.
A scalar Gold role cannot represent multiple simultaneous values. Silver keeps
all of them; the coverage test reports the incompatible binding, and Gold never
attributes a multi-assignee issue to an arbitrary first member.

## Deletions and links

An explicit comment `deleted` flag reaches comments and lifecycle history.
Work-item deletion is unknown without authoritative evidence (`is_deleted=NULL`).
A missing census row is not interpreted as deletion or access loss: this Bronze
contract does not supply a completed-run boundary that proves an absence.
Census issues missing a full issue record are reported by a coverage test,
scoped to the collection window: an issue last updated before
`youtrack_start_date` is in the census but deliberately never snapshotted.

Link intervals are bounded by complete issue observations and have
`evidence='observation'`, `valid_from_known=0`. Empty link sets close intervals;
removal and re-addition yield separate intervals. Trimmed sets and missing link
properties cannot establish an absence. Link identity uses immutable target IDs,
not readable keys. Exact activity-based link intervals are not inferred from a
snapshot. Sprint dimensions retain native dates; archived does not imply a
completion date or lifecycle state.

## Validation

The disposable ClickHouse suite is in `dbt/tests/youtrack/transform`. It runs the
actual dbt models, including the shared Silver union and Gold role consumer.
Additional dbt tests check identities, coverage, value arrays and event ordering.

API references:
- https://www.jetbrains.com/help/youtrack/devportal/api-entity-CustomFieldActivityItem.html
- https://www.jetbrains.com/help/youtrack/devportal/api-entity-CustomFilterField.html
- https://www.jetbrains.com/help/youtrack/devportal/api-entity-FieldType.html
