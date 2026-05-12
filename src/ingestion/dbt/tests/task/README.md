# task_tracker dbt tests

Singular SQL tests on `silver.class_task_field_history`. Each file returns rows
**that represent a violation** — a test passes when zero rows are returned.

Run:
```
dbt test --select tag:task_tracker --profiles-dir .
# or by test name
dbt test --select test_name:assert_multi_value_no_comma_strings --profiles-dir .
```

Note: these tests are not tagged yet (they'll be picked up by default `dbt test`). To
tag, add `config(tags=['task_tracker'])` as a SQL comment at the top of each test.

## What's covered

| Test | What it catches |
|------|-----------------|
| `assert_value_arrays_same_length` | `value_ids` and `value_displays` out of sync |
| `assert_no_duplicate_items_in_array` | `value_ids` contains the same item twice (buggy Add) |
| `assert_event_kind_matches_event_id` | `event_kind='initial'` without `initial:` prefix, or vice versa |
| `assert_no_duplicate_silver_rows` | PK collision after ReplacingMergeTree merge |
| `assert_multi_value_no_comma_strings` | Jira legacy comma-list treated as single Add (Sprint/Roadmap bug) |
| `assert_single_value_arrays_max_one` | Single-value field ended up with multiple elements |
| `assert_delta_action_matches_cardinality` | `delta_action` incompatible with `field_cardinality` |
| `assert_changelog_traceable_to_bronze` | Silver changelog row with no matching `staging.jira_changelog_items` |
| `assert_initial_traceable_to_bronze` | Silver initial row with no matching `bronze_jira.jira_issue` |
| `assert_initial_is_earliest_per_issue_field` | `event_at` of initial row later than first changelog |
| `assert_value_id_type_consistent` | Same field_id classified with different `value_id_type`s across runs |

## Not covered (too expensive)

Semantic monotonicity (each row's `value_ids` == previous row's value +/- delta). Requires
window functions over the full history per `(issue, field)`. Use as an ad-hoc spot check.
