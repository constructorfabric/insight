# Jira field macros

The SQL that turns Jira's field catalogue, issue JSON and changelog into the
`class_task_field_history` contract. Four files, one job each:

| file | what it decides |
|---|---|
| `jira_field_kind.sql` | which of the closed `field_kind` values a field is, from its catalogue row |
| `jira_field_value.sql` | how a field's value in the issue JSON becomes `(value_ids, value_displays)` |
| `jira_field_delta.sql` | how one changelog item becomes the state before and after that event |
| `jira_field_id_type.sql` | the contract's `value_id_type` and `field_cardinality` for a kind |

Design and rationale: [`FIELD-HISTORY-IN-DBT.md`](../../../connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md).

## Why these exist

Jira serializes the same logical thing several ways, and which way it picks is a
property of the **field**, not of the value. The pipeline these macros replace
decided it by inspecting the value instead — searching a rendered string for
`", "` to guess whether it was a list — which discarded every event of a
labels-type field and mangled every bracketed-id one. Every rule here keys on
the catalogue row and nothing else.

## Rules that are load-bearing

Change these and something downstream breaks silently, so they are called out
rather than left to be rediscovered:

- **No `customfield_` literal may appear in any of these files.** Field ids are
  instance-specific; Jira type constants are not. A rule written against an id
  works on one Jira and silently misclassifies on the next.
  `assert_jira_field_kind_is_id_independent` enforces it.
- **Structure first, plugin keys only to disambiguate.** `schema_type` and
  `schema_items` are a closed set defined by Jira; `schema_custom` is open —
  any installed app can add a key. Keying on `schema_custom` sends every unseen
  app field to `UNKNOWN`.
- **Anything unmatched must reach `UNKNOWN`, never a default.** `UNKNOWN` fails
  the run. `ignored` means somebody looked and decided; `UNKNOWN` means nobody
  has. The two defects these macros fix were both silent.
- **Ids are not consistently quoted.** An option id arrives as `"19272"` and a
  sprint id as `2151`, in the same position. Every id goes through
  `jira_json_unquote`; `JSONExtractString` alone returns `''` for a bare number.
- **The display probe order in `jira_json_obj_display` is deliberate.** A
  project object carries both `name` and `key` and the changelog renders its
  name, so `name` must be probed first, or the two sides stop reconciling.

## Who uses them

Only the Jira connector's models, in
`src/ingestion/connectors/task-tracking/jira/dbt/` — chiefly
`jira__task_field_kind`, `jira__issue_field_snapshot` and
`jira__field_history_derived`. Nothing in `silver/`, `gold/` or another
connector reads them; that is why they live in their own folder rather than
alongside the cross-source macros.

dbt searches `macro-paths` recursively, so no project configuration mentions
this directory.

## How they are tested

Three layers, all under [`../../tests/jira/`](../../tests/jira/):

1. **Unit, over literal fixtures.** `assert_jira_field_kind_fixtures`,
   `assert_jira_field_value_fixtures`, `assert_jira_field_delta_fixtures` and
   `assert_jira_field_delta_element_fixtures` invoke a macro on an inline
   `arrayJoin` of tuples and select the rows where actual differs from expected.
   They touch no table, so they are true unit tests and run in seconds.
   **Add a case here for every branch you add.**
2. **Transformation, over controlled inputs.** A pytest suite seeds the three
   bronze tables — field catalogue, issue snapshot, changelog — builds the Jira
   staging chain and compares the resulting journal against an expected row set.
   This is where "a field set at creation and never changed still produces a
   row" is actually proved.
3. **Invariant, over whatever data a deployment holds.**
   `assert_jira_field_history_round_trip` replays each field's newest state and
   requires it to equal the value the issue holds, so a separator or shape rule
   that is wrong for some other Jira instance fails there rather than on a
   dashboard. `assert_jira_field_kind_covers_catalogue` fails on any `UNKNOWN`.

Run the unit and invariant tests with `dbt test --select tests/jira`; the pytest
suite has its own runner, described in [`../../tests/jira/README.md`](../../tests/jira/README.md).
