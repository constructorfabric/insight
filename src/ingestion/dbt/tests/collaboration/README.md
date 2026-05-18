# collaboration dbt tests

Singular SQL tests on `silver.class_collab_meeting_activity` and
`silver.class_collab_chat_activity`. Each file returns rows **that
represent a violation** — a test passes when zero rows are returned.

Run:
```bash
dbt test --select test_name:assert_meeting_activity_unique_per_person_day --profiles-dir .
dbt test --select test_name:assert_meeting_activity_one_source_per_data_source --profiles-dir .
dbt test --select test_name:assert_meeting_duration_caps --profiles-dir .
```

## What's covered

| Test | What it catches |
|------|-----------------|
| `assert_meeting_activity_unique_per_person_day` | `>1` row per `(tenant, person_key, date, data_source)` after `FINAL` — i.e. the silver class lost its grain (parallel/duplicate stream, broken `unique_key`, etc). Issue #283 reference case. |
| `assert_meeting_activity_one_source_per_data_source` | More than one `insight_source_id` per `data_source` per tenant. Almost always means a parallel/duplicate Airbyte source for the same external account (e.g. tenant kept the placeholder `main` source after switching to `zoom-main`). Issue #283 reference case. |
| `assert_meeting_duration_caps` | A `class_collab_meeting_activity` row where `video_duration_seconds` or `screen_share_duration_seconds` exceeds `audio_duration_seconds`. By construction this should never happen — audio = session length, video/screen-share are sessions gated by per-user flags. Issue #263 reference. |
