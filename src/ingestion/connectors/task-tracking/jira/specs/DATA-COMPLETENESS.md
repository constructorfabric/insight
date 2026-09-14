# Jira Data Completeness: Custom Fields, Board Metadata, Validation

Status: implemented. Part of the data-completeness scope of issue #2419.
Deletion and visibility handling lives in
[DELETION-AND-VISIBILITY.md](DELETION-AND-VISIBILITY.md); JSM SLA support is
tracked separately (#1834).

## Story-points resolution

Jira stores story points in an instance-specific `customfield_NNNNN` whose id
differs between deployments and even between project styles inside one
deployment (company-managed vs team-managed projects use different fields).
A hardcoded id silently reads the wrong field — or nothing.

Resolution is metadata-driven, in dbt, from `bronze_jira.jira_fields`
(`GET /rest/api/3/field`):

1. fields whose `schema.custom` is the canonical greenhopper marker
   `com.pyxis.greenhopper.jira:jsw-story-points`;
2. fields named exactly `Story Points` (case-insensitive);
3. fields named exactly `Story point estimate`.

**Superseded by generic field extraction.** `jira__issue_field_snapshot` no
longer resolves story points as a special case: it unpivots every key the issue
JSON carries and classifies each by `jira__task_field_kind`, so a story-points
field is read like any other `scalar`
(see [FIELD-HISTORY-IN-DBT.md](FIELD-HISTORY-IN-DBT.md)). The candidate list and
the per-issue coalescing it required are gone.

What made the removal safe is a property of the data rather than of the code: an
issue holds a value in at most one candidate field, because a project's style
decides which field it uses. Coalescing therefore had nothing to merge, and
emitting each field under its own id produces the same one row per issue. A
singular test guards the property.

Both shapes emit the **Jira-native field id** as `field_id` (never a synthetic
`story_points` label), so snapshot rows merge with changelog rows for the same
field — the rule that also governs `duedate`.

Connector-side, the `jira_story_points_field_id` config key no longer has a
hardcoded default; it remains as an explicit operator override that lands in
`bronze_jira.jira_issue.story_points`.

## Board configuration

`jira_board_configuration` (`GET /rest/agile/1.0/board/{id}/configuration`,
substream of `jira_boards`, one request per board) fills the board-metadata
Bronze gap:

- `estimation_field_id` / `estimation_field_name` — the board's estimation
  field. Scrum boards carry it, kanban boards do not. Serves as per-board
  corroboration of the metadata-resolved story-points field.
- `column_config` — the column-to-status mapping (JSON), the board's own
  definition of its workflow lanes.
- `board_location` — project the board belongs to (JSON).

## Project lead

`/rest/api/3/project/search` omits the `lead` object unless asked;
`jira_projects` now requests `expand=lead`, so the (previously always-empty)
`lead_account_id` Bronze column is populated.

## API-to-Bronze completeness checks

dbt singular tests (`src/ingestion/dbt/tests/task/`) validate, on every run,
that what the API reported actually landed:

| test | catches |
|------|---------|
| `assert_census_issue_has_full_record` | an issue the census observed as present and the incremental scan enumerated, but whose full `jira_issue` record is missing — the sync committed the lightweight parent row and lost the payload (the green-but-empty failure mode) |
| `assert_availability_ids_and_timestamps` | availability rows with missing ids, empty state, or a present issue without a last-seen timestamp — malformed census emissions |
| `assert_worklog_deletion_state_consistent` | a worklog with an authoritative deletion tombstone still flagged alive in the class contract — a regression in the deletion signals |

Together with the source-freshness gates already declared on `bronze_jira`
(warn 36h / error 72h, now covering the census tables too) these are the
automated "missing IDs, values, timestamps, or deletion state" validation
required by #2419's definition of done.
