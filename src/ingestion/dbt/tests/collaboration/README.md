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
| `assert_m365_staging_days_were_reported` | A day in an M365 staging feeder (email, chat, meeting, OneDrive, SharePoint) whose bronze report holds a row with activity while its `lastActivityDate` is earlier — a carried-forward copy Microsoft serves for a day it never computed. The feeders drop such days on build (macro `m365_reported_days`, shared with this test); a stored failure means the rows predate the filter and a `dbt --full-refresh` over `tag:m365+` is due. Also flags a bronze `lastActivityDate` whose first ten characters are not a date: on such a day a copy cannot be detected. Not in the `data_quality` selector: the relations exist only where M365 synced. |
| `assert_slack_bronze_rows_reach_metrics` | Slack bronze rows that never reach metrics, reported as log-only findings (`connector_quality`, tier `warn`) after each Slack sync. `refused_day_row`: no `user_id` or no parseable `date`, the shape the declarative connector stored for a day Slack refused; staging drops it. `missing_counter`: a member row without `messages_posted_count` or `channel_messages_posted_count`; staging drops it instead of reading it as 0. `member_without_email`: a non-guest member with no `email_address`; it reaches silver keyed by `user_id`, but gold keeps only email-keyed rows, so the member has no Slack metrics. One row per member, with the day count. |
