# task_tracker dbt tests

Singular SQL tests on the task-tracking classes and their configuration. Each
file returns rows **that represent a violation** — a test passes when zero rows
are returned.

Every test here belongs to one of the two catalogs described in
[`../README.md`](../README.md); nothing in this directory is untagged, because an
untagged test runs on no installation:

- `data_quality` — reads silver or `config` only, so it runs in the scheduled
  install-wide pass whatever trackers a tenant has.
- `connector_quality` + the connector slug — needs that connector's bronze or
  staging, or checks a rule only that connector's rows follow, so it runs in the
  connector's own `connector-checks` step.

Run one catalog against a warehouse:

```bash
dbt test --selector data_quality --profiles-dir .
dbt test --select tag:connector_quality,tag:jira --profiles-dir .
dbt test --select tag:connector_quality,tag:github --profiles-dir .
```

## What's covered

| Test | Catalog | What it catches |
|------|---------|-----------------|
| `assert_value_arrays_same_length` | data_quality | `value_ids` and `value_displays` out of sync |
| `assert_no_duplicate_items_in_array` | data_quality | `value_ids` contains the same item twice |
| `assert_single_value_arrays_max_one` | data_quality | A single-value field with several elements |
| `assert_multi_value_no_comma_strings` | data_quality | A comma-joined list stored as one multi-value element |
| `assert_delta_action_matches_cardinality` | data_quality | `delta_action` incompatible with `field_cardinality` |
| `assert_event_kind_matches_event_id` | data_quality | `event_kind` that disagrees with the `event_id` prefix |
| `assert_event_order_unique_per_field` | data_quality | Two rows of one (issue, field) share `event_order`, leaving that field's order undefined |
| `assert_event_order_carries_event_time` | data_quality | A row other than `synthetic_initial` whose `event_order` does not lead with its `event_at` millisecond |
| `assert_initial_state_precedes_first_event` | data_quality | An issue's `synthetic_initial` rows order after one of its changelog rows |
| `assert_no_duplicate_silver_rows` | data_quality | More than one row per (issue, field, event) after `FINAL` |
| `assert_value_id_type_consistent` | data_quality | One field classified with different `value_id_type`s |
| `assert_text_refs_resolve` | data_quality | A `text_ref` value or a current body with no row in `class_task_field_text`, or a display that is not a prefix of its body |
| `assert_link_intervals_are_sane` | data_quality | A link interval that ends before it begins, or one closed at the epoch — the value an unmatched ASOF join yields instead of NULL |
| `assert_issue_types_unique_per_source` | data_quality | Two `class_task_issuetypes` rows for one (source, type id), which multiplies issues in gold joins |
| `assert_one_field_per_role` | data_quality | Two vendor fields bound to one metric role |
| `assert_every_field_is_bound_or_ignored` | data_quality | A field with events in a bound source that no binding names |
| `assert_bound_values_are_mapped` | data_quality | A status or issue type value observed in history with no mapping |
| `assert_canonical_values_match_their_role` | data_quality | A status mapping outside `new`, `in_progress`, `done`, `undefined` |
| `assert_field_values_are_canonical` | data_quality | A `field_value_map` target or default outside its field's domain |
| `assert_binding_validity_is_sane` | data_quality | A binding that starts in the future, or two decisions under one key |
| `assert_type_map_display_matches_catalog` | data_quality | A type map display that disagrees with the tracker's catalog |
| `assert_changelog_items_not_double_encoded` | connector_quality, jira | A changelog row whose `items` is an escaped string instead of an array |
| `assert_changelog_traceable_to_bronze` | connector_quality, jira | Silver changelog row with no matching `staging.jira_changelog_items` |
| `assert_initial_traceable_to_bronze` | connector_quality, jira | Silver initial row with no matching `bronze_jira.jira_issue` |
| `assert_task_field_history_key_is_issue_keyed` | connector_quality, jira | A Jira silver row whose `unique_key` is not the issue-keyed formula |
| `assert_census_issue_has_full_record` | connector_quality, jira | A censused recent issue the incremental scan never fetched |
| `assert_availability_ids_and_timestamps` | connector_quality, jira | An availability row missing its identifier or timestamp |
| `assert_worklog_deletion_state_consistent` | connector_quality, jira | A tombstoned worklog still marked live |
| `assert_issue_link_sets_are_complete` | connector_quality, github | A link set collected with fewer members than the vendor states — a nested connection cut off at its page boundary |
| `assert_boards_yield_cards` | connector_quality, github | Projects V2 boards and board events collected, cards not |

## Not covered (too expensive)

Semantic monotonicity (each row's `value_ids` == previous row's value +/- delta). Requires
window functions over the full history per `(issue, field)`. Use as an ad-hoc spot check.
