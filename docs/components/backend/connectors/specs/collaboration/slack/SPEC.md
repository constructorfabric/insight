# Slack connector

The Slack connector reads data from Slack: member activity, messages, channels, huddles, alerts, AI use, apps and workflows, and organization data.
Now, Insight uses only member activity, for chat and meeting metrics (section 10).

The data that the connector reads depends on the Slack plan of the customer.
The connector finds the plan and reads only the streams that the plan permits.

Code: `src/ingestion/connectors/collaboration/slack` (Python CDK connector, image `source-slack-insight`).

This document gives the intended behavior. For the differences from `main`, refer to [Gaps](#13-gaps).
For all the data that Slack supplies, refer to [DATA-SOURCES.md](DATA-SOURCES.md).

## 1. Streams

| Stream | Slack API | Plans | Scopes | Bronze table | Status |
|---|---|---|---|---|---|
| Activity | `admin.analytics.getFile?type=member` | B+, Enterprise | `admin.analytics:read` | `users_details` | In `main` |
| Members | `users.list` | All | `users:read`, `users:read.email` | `users` | #4341 |
| Message activity | `conversations.history`, `conversations.replies` | Free, Pro | `channels:history`, `groups:history`, `im:history`, `mpim:history` | `messages` (proposal, section 7.3) | #4349 |
| Channels | `admin.analytics.getFile?type=public_channel` | B+, Enterprise | `admin.analytics:read` | `channels_details` (proposal, section 7.3) | #4350 |
| Alerts | `conversations.history`, `conversations.replies` | All | `channels:history`, `groups:history` | `messages` (proposal, section 7.3) | #4351 |
| Huddles | Audit Logs API, actions `huddle_*` | Enterprise | `auditlogs:read` | `huddle_events` (proposal, section 7.3) | #4352 |
| AI use | `admin.analytics.slackbot.tools`, `.skills`, Audit Logs `slack_ai_*`, `mcp_slack_*`, `agentforce_*` | Enterprise | `admin.analytics:read`, `auditlogs:read` | `ai_usage_daily`, `ai_events` (proposal, section 7.3) | #4353 |
| Message metadata | `admin.analytics.messages.metadata` | Enterprise | `admin.analytics:read` | `messages` (proposal, section 7.3) | #4354 |
| Apps and workflows | `admin.apps.approved.list`, `admin.workflows.search`, `admin.apps.activities.list` | Enterprise | `admin.apps:read`, `admin.workflows:read`, `admin.app_activities:read` | `apps`, `workflows`, `app_activities` (proposal, section 7.3) | #4355 |
| User groups, SCIM | `usergroups.list`, `usergroups.users.list`, SCIM API | All (groups), B+ (SCIM) | `usergroups:read`, `admin` | `usergroup_members`, `member_org` (proposal, section 7.3) | #4356, decision open |

Plan names:

| Name | Slack plan | `team.billing.info` value |
|---|---|---|
| Free | Free | `free` |
| Pro | Pro | `std` |
| B+ | Business+ | `plus` |
| Enterprise | Enterprise Grid, Enterprise+, Enterprise Compliance Select | `enterprise`, `compliance` |

Streams for each plan:

| Plan | Streams |
|---|---|
| Free, Pro | Members, Message activity, Alerts, User groups |
| B+ | Activity, Members, Channels, Alerts, User groups, SCIM |
| Enterprise | All streams except Message activity |

The main stream gives the daily activity of each member. On B+ and Enterprise, the main stream is Activity. On Free and Pro, the main stream is Message activity. All other streams are optional.

Each issue in the "Status" column sets the columns and the sync rules of its stream.
Sections 5, 6, 7.1, 7.2, 8 and 9 give the rules for Activity and Members. The other sections apply to all streams.

## 2. Slack access

| Item | Value |
|---|---|
| Token | One user OAuth token (`xoxp-…`) for all streams. The app has no bot user. The admin and analytics APIs do not accept bot tokens (`xoxb-…`). |
| Install | An Org Owner or an admin installs the app. For the Audit Logs API, the Org Owner installs the app at the organization level. |
| Plan scope | `team.billing:read`. This scope is necessary on all plans. |
| Stream scopes | Refer to section 1. The scopes of the token set which streams the connector can read. |
| App type | Internal app of the customer. Slack does not apply its 2025 history limit to internal apps. |

### 2.1 Activity file

| Item | Value |
|---|---|
| Request | `GET https://slack.com/api/admin.analytics.getFile?type=member&date=YYYY-MM-DD` |
| Day | One UTC day for each request. Slack publishes a day 2 to 4 days after the day ends. |
| History | Slack keeps 13 months. |
| Served day | HTTP 200 with a gzip file of JSON Lines. Each line is one member. |
| Refused day | HTTP 200 with JSON `{"ok": false, "error": "<code>"}`. |

Each file lists **all members of the workspace**. Inactive members have zero counts.

### 2.2 Members list

| Item | Value |
|---|---|
| Request | `GET https://slack.com/api/users.list?limit=200&cursor=<next_cursor>` |
| Pages | The connector reads all pages in each sync. |

On Enterprise, `users.list` can need a `team_id` for each workspace. Make sure of this on a real organization before you write the code (#4341).

## 3. Plan detection

1. At the start of each sync and each connection check, the connector calls `team.billing.info`.
2. The connector reads only the streams that the plan permits (section 1).
3. If the connector cannot get the plan, the sync fails. Section 6.1 sets the failure type, the same as for all other calls. For example, a missing `team.billing:read` scope is `config_error`, and an HTTP 5xx after the tries is `system_error`. The message names the Slack error code.
4. If the plan changes, the next sync reads the streams of the new plan. Bronze keeps the rows of the streams that stop.

Missing scopes:

| Stream | Scope is missing | Result |
|---|---|---|
| Main stream | Yes | The sync fails (`config_error`). The message names the scope. |
| Optional stream | Yes | The connector does not read the stream. It writes a warning with the scope to the log. The other streams continue. |

The connection check shows the plan, the streams that the connector will read, and the missing scopes.

## 4. Configuration

**Intended:** connector management configures the connector. Connector management does not exist now.

**Current state:** a Kubernetes Secret with the label `app.kubernetes.io/part-of: insight`:

| Key | Required | Meaning |
|---|---|---|
| annotation `insight.cyberfabric.com/connector` | Yes | `slack` |
| annotation `insight.cyberfabric.com/source-id` | Yes | Instance ID, for example `slack-main` |
| `slack_bot_token` | Yes | The user token (section 2). The name of the key is historical. |
| `slack_start_date` | Yes | The first day to read, `YYYY-MM-DD`, for all streams with dates. The operator sets it. There is no default. For Activity and Channels, it must be in the last 13 months. |
| `slack_alert_channels` | No | The channels for the Alerts stream (#4351). |

Reconcile adds two fields to the connector configuration. These fields are not in the Secret data:

- `insight_tenant_id`: the Helm value `ingestion.reconcile.tenantId`. There is one value for each installation.
- `insight_source_id`: the value of the `source-id` annotation.

Schedule: each day at **04:00 UTC**. The descriptor sets the schedule. The Secret annotation `insight.cyberfabric.com/schedule` replaces it.

## 5. Sync

### 5.1 Activity

- Each sync reads the days from **`max(slack_start_date, cursor − 2 days)`** to **today − 2 days** (UTC). It sends one request for each day, in sequence.
- Each sync reads the cursor day and the 2 days before it again. Slack can change these days late. The new rows replace the stored rows.
- The first sync starts at `slack_start_date`. After a stop, the next sync reads all missing days.
- The state is `{"date": "YYYY-MM-DD"}`. This is the last day that the connector read fully.
- The connector sends the state after each day. Airbyte keeps the state for each connection in its own database. Airbyte saves the state after it writes the rows of that day. The connector image keeps no data.
- If you move `slack_start_date` (section 4) after the cursor, the connector does not read the days between them.

### 5.2 Members

- Each sync reads all pages of `users.list`. It updates all members that Slack returns.
- This stream has no state.

## 6. Errors

### 6.1 All streams

| Condition | Result |
|---|---|
| HTTP 429 or 5xx | The connector tries again 5 times. It waits for the time in `Retry-After` (seconds or HTTP date). If there is no `Retry-After`, it waits 2, 4, 8, 16 and 32 s. Each wait is 300 s maximum. Then the sync fails (`system_error`). |
| Network error or timeout (60 s) | The connector tries again in the same way. Then the sync fails (`transient_error`). |
| Temporary Slack codes: `ratelimited`, `internal_error`, `fatal_error`, `service_unavailable`, `request_timeout` | The connector tries again in the same way. Then the sync fails (`system_error`). |
| Token, scope and plan codes: `invalid_auth`, `not_authed`, `token_revoked`, `token_expired`, `account_inactive`, `missing_scope`, `not_allowed_token_type`, `not_an_admin`, `no_permission`, `access_denied`, `feature_not_available`, `ekm_access_denied`, `team_access_not_granted`, `two_factor_setup_required`, `enterprise_is_restricted` | The sync fails (`config_error`). The message names the code. For a missing scope of an optional stream, refer to section 3. |
| Other Slack codes | The sync fails (`system_error`). The message names the code. |

After a failure, the data that the connector read before the failure keeps its rows and its state. The next sync starts again from the state.

The error messages name the code and, for a stream with dates, the day. They do not include data from Slack, because the data contains personal data.

### 6.2 Analytics files (Activity, Channels)

| Condition | Result |
|---|---|
| `file_not_yet_available`, the day is 5 days old or less | The sync stops at the last published day. The sync is **successful**. The next sync tries the day again. |
| `file_not_yet_available`, an older day | The sync fails (`system_error`). |
| `member_analytics_disabled`, `org_level_email_display_disabled` | The sync fails (`config_error`). The message names the code. |
| Other codes, for example `file_not_found`, `data_not_available` | The sync fails (`system_error`). The message names the code and the day. **The sync fails on each run** until an operator moves `slack_start_date` after that day. |
| The body is not gzip or JSON Lines, a line is not an object, or a row has no key field (`user_id` for Activity) | The sync fails (`system_error`). |
| The file is larger than a limit (refer to the limits below) | The sync fails (`system_error`). |
| The file is empty | The sync is successful with 0 rows. |

After a failure, the days before the failure keep their rows and their state. The next sync reads the failed day again.

**Limits for each day file:** 512 MiB compressed, 2 GiB decompressed, 1 MiB for each line, 2 gzip layers maximum. The connector reads the file as a stream. It does not keep the full file in memory.

### 6.3 Where you see a failure

| Place | Data |
|---|---|
| Alert `insight-connector-sync-failed` (Grafana) | The connector and the `source_id`. No reason. |
| Airbyte UI, connection job history | The failure type and message, for example `Slack refused admin.analytics.getFile for 2026-10-17: file_not_found` |
| Argo `ingestion-pipeline` workflow, log of the `sync` step | The event `sync.failure_reason` with the same type and message |
| ClickHouse `ingestion_history.sync_events` | `status = 'failed'` for each job. No reason. |

EPIC #4274 gives the failure reasons as metrics.

### 6.4 Connection check

The check makes sure that:

- The required keys are present.
- The start date is a valid date.
- The connector can get the plan (section 3).
- The connector can read the first page of each stream that the plan permits. For Activity, this is the file for today − 2. `file_not_yet_available` is a pass.

## 7. Bronze

All tables are in the database `bronze_slack`.

A table contains one type of data. The API that supplies the data does not set the table. If two streams supply the same type of data, they write to the same table.

| Rule | Value |
|---|---|
| Write mode | `append_dedup`, primary key `unique_key` |
| Engine, single node | `ReplacingMergeTree(_airbyte_extracted_at)`, `ORDER BY unique_key` |
| Engine, cluster | `ReplicatedReplacingMergeTree(_airbyte_extracted_at)`, `ORDER BY unique_key`. The destination creates each table `ON CLUSTER <name>`. Each replica has all rows. There is no `Distributed` table. The destination writes without a quorum, so a replica can be behind for a short time. Helm values `clickhouse.clusterMode` and `clickhouse.clusterName` select the mode. |
| Valid row | The newest load of a key. Read with `FINAL`. |
| Platform columns | `insight_tenant_id` and `insight_source_id`. The connector copies them from its configuration (section 4). |
| Not nullable | `insight_tenant_id`, `insight_source_id`, `unique_key`, and the key fields of each table: `user_id` and `date` in Activity, `user_id` in Members. The connector does not send a row without these values (section 6). |
| Nullable | All other columns. If the destination cannot convert a value, it stores NULL and records the change in `_airbyte_meta.changes`. The load does not fail. |
| Retention | **No limit.** Slack keeps some data for a limited time, for example the analytics files for 13 months. Bronze is the long-term record. |
| Airbyte columns | `_airbyte_raw_id`, `_airbyte_extracted_at`, `_airbyte_meta`, `_airbyte_generation_id` |

### 7.1 Activity: `users_details`

`unique_key` = `<insight_tenant_id>-<insight_source_id>-<user_id>-<date>`.

| Column | Type | Meaning | Use |
|---|---|---|---|
| `date` | String `YYYY-MM-DD` | UTC day | Metric day |
| `user_id` | String | Slack member ID | Key, identity account |
| `email_address` | String | Member email | Identity claim |
| `team_id` | String | Workspace ID | – |
| `is_guest` | Bool | The account is a guest on that day | Who is counted |
| `is_billable_seat` | Bool | The account is billable on that day. Inactivity changes this value. | – |
| `is_active`, `is_active_ios`, `_android`, `_desktop`, `_apps`, `_workflows`, `_slack_connect` | Bool | Activity on that day, for each client | – |
| `messages_posted_count` | Int64 | All messages: channels, threads, DMs, group DMs | Messages |
| `channel_messages_posted_count` | Int64 | Messages in channels, with thread replies | Channel posts |
| `slack_huddles_count` | Int64 | Huddles that the member joined | Meetings |
| `reactions_added_count`, `files_added_count`, `total_calls_count`, `slack_calls_count`, `search_count` | Int64 | Other activity | – |
| `date_claimed` | Int64 | Time when the account was claimed (epoch) | – |
| `insight_tenant_id`, `insight_source_id` | String | Platform columns | Tenant, instance |
| `unique_key` | String | Key | Key |

### 7.2 Members: `users`

`unique_key` = `<insight_tenant_id>-<insight_source_id>-<user_id>`.

- There is one row for each member. The newest load is the valid row.
- If a member changes the name in Slack, the next sync replaces the old name. Bronze does not keep the old name.
- A deactivated member stays, with `deleted = true`. A member that `users.list` does not return keeps the last row.
- The connector keeps only the columns below. It removes all other profile fields (phone, title, photo, status) before bronze.

| Column | Type | Meaning | Use |
|---|---|---|---|
| `user_id` | String | Slack member ID | Key, identity account |
| `real_name` | String | Full name | List of unmapped members |
| `display_name` | String | Display name | List of unmapped members |
| `deleted` | Bool | The member is deactivated in Slack | List of unmapped members |
| `email` | String | Member email. Needs the scope `users:read.email`. | Identity claim |
| `is_restricted` | Bool | Multi-channel guest | Who is counted |
| `is_ultra_restricted` | Bool | Single-channel guest | Who is counted |
| `is_bot` | Bool | The member is a bot | Who is counted, identity |
| `insight_tenant_id`, `insight_source_id` | String | Platform columns | Tenant, instance |
| `unique_key` | String | Key | Key |

### 7.3 Other streams

These streams do not exist now. The table names and the row grain are proposals. The issue of each stream confirms or changes them, and sets the columns.

| Stream | Table (proposal) | One row for each | `unique_key` after `<insight_tenant_id>-<insight_source_id>-` | Issue |
|---|---|---|---|---|
| Message activity, Alerts, Message metadata | `messages` | Message or thread reply | `<channel_id>-<ts>` | #4349, #4351, #4354 |
| Channels | `channels_details` | Public channel and day | `<channel_id>-<date>` | #4350 |
| Huddles | `huddle_events` | Audit Logs event `huddle_*` | `<event_id>` | #4352 |
| AI use | `ai_usage_daily` | Member, day and Slackbot tool or skill | `<user_id>-<date>-<tool or skill>` | #4353 |
| AI use | `ai_events` | Audit Logs event `slack_ai_*`, `mcp_slack_*`, `agentforce_*` | `<event_id>` | #4353 |
| Apps and workflows | `apps` | App | `<app_id>` | #4355 |
| Apps and workflows | `workflows` | Workflow | `<workflow_id>` | #4355 |
| Apps and workflows | `app_activities` | Run event of a workflow or a function | `<trace_id>-<event_type>-<ts>` | #4355 |
| User groups | `usergroup_members` | Group and member | `<group_id>-<user_id>` | #4356 |
| SCIM | `member_org` | Member: department, manager, title | `<user_id>` | #4356 |

All rules of section 7 apply to these tables.

Rules for `messages`:

- The three streams write the same columns. A bot message has `bot_id`. An alert is a bot message in a channel from `slack_alert_channels`.
- The newest row replaces the full row. If two streams can write the same message, they must supply the same columns. If they do not, the newer row deletes the values of the older row.

The tables that come from messages keep only metadata. The connector removes the content before bronze, but keeps counts and IDs from it:

| Slack field | Content (removed) | Metadata (kept) |
|---|---|---|
| `text`, `blocks` | Message text, formatted text, link URLs | `text_length`, `link_count`, `link_hosts` |
| `files` | File names, titles, previews, URLs | `file_ids`, `file_types`, `file_count` |
| `attachments` | Link previews, bot payloads (titles, text, fields) | `attachment_count` |

The file IDs let Insight count each file once for each day, the same as `collab.files_shared` for M365.

Link rules:

- A link is an absolute `http` or `https` URL in the message. Mentions of users, channels and groups are not links.
- `link_count` is the number of links in the message.
- `link_hosts` is the list of hosts of these links, for example `github.com`. The connector does not keep the path or the query of a URL, because they can contain names and IDs.

## 8. Who is counted

These rules apply to the main stream (section 1) on all plans. The issue of each other stream sets its rules.

**Bots** are never counted. A member is a bot when `is_bot = true` in Members, or when `user_id = USLACKBOT` (Slackbot). This rule applies to all plans.

**Guests** are never counted. The guest flag comes from:

| Plan | Guest flag |
|---|---|
| B+, Enterprise | Activity `is_guest = true` for that day |
| Free, Pro | Members `is_restricted = true` or `is_ultra_restricted = true` |

A member-day is counted when:

- the member is not a bot, **and**
- `user_id` and `date` are present, **and**
- the message counters are present (a NULL counter is a broken row, not zero), **and**
- the member is not a guest. If the guest flag is NULL, the member is counted only if the member resolves to a person (section 9).

These values do not change the decision: `is_billable_seat`, `is_active`, email domain. Non-billable members and members with a partner domain are counted.

Inactive members are counted with zeros. Their message metrics are `0`. They add no active day.

## 9. Identity

- Slack sends one identity input for each member: the account `(slack, <insight_source_id>, <user_id>)` with the claim of the email.
- The email comes from Activity `email_address` (B+, Enterprise) or from Members `email` (all plans). If both are present, Activity wins, because it is the value for that day.
- Bots (section 8) bind to the excluded person (not a human). Identity resolution does not resolve them to a person, and does not show them as unmapped.
- Identity resolution finds the person for a Slack row **by the account first** (`identity.account_assignment`). Then it uses the email (`identity.person_map`).
- Thus a member with a new email stays the same person, because the `user_id` does not change. A member without an email can also have a person.
- Identity resolution gives each member one `person_id`. Identity resolution solves conflicts, for example one email on more than one person. The connector does not.
- Person and team views do not show rows without a person.
- The identity resolution service shows these members as unmapped Slack members. It shows the `user_id`, the email and the name (from Members). An operator can then bind each member to a person.

## 10. Metrics

Path: bronze → staging `slack__collab_chat_activity` → silver `class_collab_chat_activity` (and `class_collab_meeting_activity` for huddles) → gold `collab_metric_evidence` → `collab_metric_observations`.
In metrics, these rows have the dimension `tool = slack`.

| Bronze | Measure | Metrics |
|---|---|---|
| `messages_posted_count` | `total_chat_messages` | `collab.messages_sent`, `collab.msgs_per_active_day` |
| `channel_messages_posted_count` | `channel_posts` | `collab.channel_posts` |
| Messages minus channel messages (0 minimum) | `direct_and_group_messages`, with `split_chat_messages` as denominator | `collab.dm_ratio` |
| A day with messages > 0 | `chat_active_day`, `active_day` | `collab.msgs_per_active_day`, `collab.active_days`, and the work days for `collab.meeting_free_days` and `collab.focus_time_pct` |
| `slack_huddles_count` | `meetings_attended` (count only: no duration, no participants) | `collab.meetings_count`. A day with a huddle is **not** meeting-free. |

A Slack day is the UTC day from Slack. There is no time-zone change.

On Free and Pro, Message activity supplies the same measures (#4349).
On Enterprise, the Huddles stream supplies meetings with duration and participants. It replaces the huddle count (#4352).
Planned measures for files and links:

| Measure | Plans | Source | Metric | Issue |
|---|---|---|---|---|
| `files_shared` | Free, Pro, Enterprise | `messages.file_ids`, each file once for each day | `collab.files_shared` | #4360 |
| `files_shared` | B+ | Activity `files_added_count`. No dedup for each file. | `collab.files_shared` | #4360 |
| `links_shared` | Free, Pro | `messages.link_count` | `collab.links_shared` (new, #4362) | #4360 |

On B+ and Enterprise, no stream gives links now. `messages.metadata` does not have links. To get links on these plans, the connector must also read the message history.

Slack has no `scope` (internal or external) for files now. Slack files are counted in `collab.files_shared` only, not in `files_shared_internal` or `files_shared_external`.

The issues of the other streams set their metrics.

## 11. Pipeline and monitoring

After each sync, the Argo workflow `ingestion-pipeline` runs these steps:

1. dbt `tag:slack`: staging and identity inputs
2. Identity seed
3. dbt `tag:slack+`: silver and gold
4. Connector checks: `dbt test --select tag:connector_quality,tag:slack`

The data-quality check `assert_slack_bronze_rows_reach_metrics` finds Activity rows that do not get to the metrics:

| Problem | Meaning |
|---|---|
| `refused_day_row` | A row without `user_id` or `date`. The old connector wrote these rows. |
| `missing_counter` | A non-guest row with a NULL message counter |
| `unmapped_member` | A non-guest member without an identity binding |

Monitoring alerts (Grafana):

| Alert | Condition |
|---|---|
| `insight-connector-sync-failed` | The last sync failed. |
| `insight-connector-stale` | No new rows in the table of the main stream for 26 h. Rows in other tables do not stop the alert. |
| `insight-dq-finding` | A data-quality finding with the tier `error`. |

## 12. Deployment

- `descriptor.yaml`: `type: cdk`, namespace `bronze_slack`, image `ghcr.io/constructorfabric/source-slack-insight:<tag>`. After each build on `main`, the CI job `bump-descriptors` sets the tag.
- Reconcile adds the image to Airbyte as a custom source.
- Reconcile moved the old declarative (manifest) connector to this image. It made the source and the connection again and put back the cursor state.

## 13. Gaps

This table gives the differences between this document and `main`. EPIC #4332 contains all issues.
The streams with an issue in section 1 do not exist in `main`.

| This document | `main` now | Issue |
|---|---|---|
| The connector finds the plan and reads only the permitted streams (section 3) | No plan detection. The connector reads Activity on all plans. | #4348 |
| An optional stream without its scope is not read. The sync continues (section 3). | `missing_scope` makes the sync fail. | #4348 |
| Optional key `slack_alert_channels` (section 4) | The key does not exist. | #4351 |
| Connector management configures the connector (section 4) | Kubernetes Secret. No connector management. | #670 |
| The token key asks for a user token (section 4) | The key is `slack_bot_token`. | #4330 |
| Each sync reads the cursor day and the 2 days before it (section 5.1) | Each sync reads the cursor day only. | #4334 |
| The bronze columns are `insight_tenant_id` and `insight_source_id` (section 7) | The columns are `tenant_id` and `source_id`. | #4342 |
| Bots are never counted and bind to the excluded person (sections 8, 9) | No bot flag. Slack bots are not excluded. | #4341 |
| Guests are never counted (section 8) | Guest rows stay on stands that did not do the 2027.0.0 full refresh. | #4316 |
| A NULL `is_guest` is counted only if the member resolves (section 8) | A NULL `is_guest` is always counted. | #4335 |
| Slack sends identity inputs. Rows resolve by `user_id` (section 9). | No Slack identity inputs. Only email. Members without email are not counted. | #4320 |
| An email on more than one person resolves to one person (section 9) | The email resolves to no person (`identity/person_map.sql`). | #4343 |
| Identity resolution shows unmapped members (section 9) | There is no such list. | #4319 |
| Slack files feed `collab.files_shared`, Slack links feed `collab.links_shared` (section 10) | No metric reads Slack files. `collab.links_shared` does not exist. | #4360, #4362 |
| Huddles are counted as meetings (section 10) | Huddles are not used. | #4321 |
| Each new bronze day gets to staging (section 11) | Staging reads only the last 3 days of loads. After a long stop, some days do not get to staging. | #4317 |
| Data-quality problems make an alert (section 11) | The check has the tier `warn`. It only writes to the log. | #4324 |
| `unmapped_member`: a non-guest member without an identity binding (section 11) | `member_without_email`: a non-guest member without an email | #4324 |
| The stale alert checks the table of the main stream (section 11) | The alert checks all tables in `bronze_slack`. When Members writes on each sync, the alert does not fire if Activity stops. | #4358 |
| The move to a new connector type never loses the cursor (section 12) | The state backup is in the reconcile pod. | #4315 |
