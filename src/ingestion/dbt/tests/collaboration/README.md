# collaboration dbt tests

Singular SQL tests on `silver.class_collab_meeting_activity`. Each file
returns rows **that represent a violation** — a test passes when zero
rows are returned.

Run:
```
dbt test --select test_name:assert_meeting_activity_unique_per_person_day --profiles-dir .
dbt test --select test_name:assert_meeting_activity_one_source_per_data_source --profiles-dir .
```

## What's covered

| Test | What it catches |
|------|-----------------|
| `assert_meeting_activity_unique_per_person_day` | `>1` row per `(tenant, person_key, date, data_source)` after `FINAL` — i.e. the silver class lost its grain (parallel/duplicate stream, broken `unique_key`, etc). Issue #283 reference case. |
| `assert_meeting_activity_one_source_per_data_source` | More than one `insight_source_id` per `data_source` per tenant. Almost always means a parallel/duplicate Airbyte source for the same external account (e.g. tenant kept the placeholder `main` source after switching to `zoom-main`). Issue #283 reference case. |
