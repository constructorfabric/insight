#!/usr/bin/env bash
# Create empty placeholder bronze + silver tables that gold-view migrations
# (scripts/migrations/*.sql) reference but that do NOT exist on a fresh
# cluster. Without these, ClickHouse's CREATE VIEW validation fails with
# UNKNOWN_TABLE / UNKNOWN_DATABASE and init.sh aborts.
#
# Two classes of placeholders:
#   1. bronze_<source>.<stream>  — populated by Airbyte connectors. Missing
#                                  on first install; Airbyte drops the
#                                  placeholder and recreates with its own
#                                  schema on the first sync.
#   2. silver.<dbt_model>        — built by `dbt run` (Argo workflow,
#                                  invoked AFTER init.sh registers it).
#                                  dbt drops the placeholder and creates a
#                                  ReplacingMergeTree on its first run.
#
# This is THE EXISTING WORKAROUND for an architectural issue: gold-view
# migrations run before dbt builds silver. The proper fix is either to
# split init.sh into pre-dbt and post-dbt phases, or to move the silver-
# dependent VIEW creation into dbt models. See ADR-0007 for the trade-off
# and tech-debt context — the placeholder list grows with every new gold
# view that adds a silver/bronze dependency.
#
# Schemas are minimum-viable: enough columns + reasonable types for the
# referenced migrations to type-check the SELECT. The real owner (Airbyte
# or dbt) overwrites with its full schema on first run.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Single-namespace umbrella (PR #224). Override via INSIGHT_NAMESPACE.
INSIGHT_NS="${INSIGHT_NAMESPACE:-insight}"
CH_POD="${CLICKHOUSE_POD:-statefulset/insight-clickhouse}"

# clickhouse-client inside the pod inherits CLICKHOUSE_USER /
# CLICKHOUSE_PASSWORD from the container env, so we do not pass --user /
# --password.
run_ch() {
  kubectl exec -i -n "$INSIGHT_NS" "$CH_POD" -- clickhouse-client --multiquery
}

ch_table_exists() {
  local db="$1" tbl="$2"
  local result
  result=$(kubectl exec -n "$INSIGHT_NS" "$CH_POD" -- clickhouse-client -q \
    "SELECT count() FROM system.tables WHERE database='$db' AND name='$tbl'" 2>/dev/null || echo "0")
  [[ "$result" == "1" ]]
}

echo "=== Placeholders (for missing connectors / unbuilt silver) ==="

run_ch <<'SQL'
CREATE DATABASE IF NOT EXISTS silver;
CREATE DATABASE IF NOT EXISTS bronze_jira;
CREATE DATABASE IF NOT EXISTS bronze_m365;
CREATE DATABASE IF NOT EXISTS bronze_zoom;
CREATE DATABASE IF NOT EXISTS bronze_cursor;
CREATE DATABASE IF NOT EXISTS bronze_slack;
CREATE DATABASE IF NOT EXISTS bronze_bamboohr;
CREATE DATABASE IF NOT EXISTS bronze_bitbucket_cloud;
SQL

# ---------------------------------------------------------------------------
# silver.* dbt-model placeholders
# ---------------------------------------------------------------------------

# silver.class_comms_events — gold-views (gold-views.sql) references this
if ! ch_table_exists silver class_comms_events; then
  echo "  Creating placeholder: silver.class_comms_events"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_comms_events (
    user_email    String,
    activity_date Date,
    emails_sent   Float64,
    source        String,
    _version      UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (user_email, activity_date);
SQL
fi

# silver.class_focus_metrics — HR dbt model. Used by ic-kpis-honest-nulls,
# team-member-honest-nulls, bullet-views-honest-nulls, views-from-silver.
if ! ch_table_exists silver class_focus_metrics; then
  echo "  Creating placeholder: silver.class_focus_metrics"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_focus_metrics (
    insight_tenant_id     String,
    email                 String,
    day                   Date,
    unique_key            String,
    meetings_count        Int64,
    meeting_hours         Float64,
    working_hours_per_day Float64,
    focus_time_pct        Float64,
    dev_time_h            Float64,
    _version              UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, day);
SQL
fi

# silver.class_collab_email_activity — collaboration dbt model.
if ! ch_table_exists silver class_collab_email_activity; then
  echo "  Creating placeholder: silver.class_collab_email_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_collab_email_activity (
    insight_tenant_id String,
    email             String,
    person_key        String,
    date              Date,
    data_source       String,
    sent_count        Float64,
    received_count    Float64,
    read_count        Float64,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date);
SQL
fi

# silver.class_collab_meeting_activity — collaboration dbt model.
if ! ch_table_exists silver class_collab_meeting_activity; then
  echo "  Creating placeholder: silver.class_collab_meeting_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_collab_meeting_activity (
    insight_tenant_id              String,
    email                          String,
    person_key                     String,
    date                           Date,
    data_source                    String,
    meetings_attended              Float64,
    calls_count                    Float64,
    participants                   Float64,
    audio_duration_seconds         Float64,
    video_duration_seconds         Float64,
    screen_share_duration_seconds  Float64,
    _version                       UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date);
SQL
fi

# silver.class_collab_chat_activity — collaboration dbt model.
if ! ch_table_exists silver class_collab_chat_activity; then
  echo "  Creating placeholder: silver.class_collab_chat_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_collab_chat_activity (
    insight_tenant_id             String,
    email                         String,
    person_key                    String,
    date                          Date,
    data_source                   String,
    total_chat_messages           Float64,
    channel_messages_posted_count Float64,
    channel_posts                 Float64,
    _version                      UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date);
SQL
fi

# silver.class_collab_document_activity — collaboration dbt model.
if ! ch_table_exists silver class_collab_document_activity; then
  echo "  Creating placeholder: silver.class_collab_document_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_collab_document_activity (
    insight_tenant_id        String,
    email                    String,
    person_key               String,
    date                     Date,
    data_source              String,
    shared_internally_count  Float64,
    shared_externally_count  Float64,
    viewed_or_edited_count   Float64,
    _version                 UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date);
SQL
fi

# silver.class_ai_dev_usage — AI dbt model. Aggregates Cursor + Claude
# Code + others.
if ! ch_table_exists silver class_ai_dev_usage; then
  echo "  Creating placeholder: silver.class_ai_dev_usage"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_ai_dev_usage (
    insight_tenant_id    String,
    email                String,
    day                  Date,
    tool                 String,
    is_active            UInt8,
    completions_count    Nullable(Float64),
    agent_sessions       Nullable(Float64),
    chat_requests        Nullable(Float64),
    tool_use_offered     Nullable(Float64),
    tool_use_accepted    Nullable(Float64),
    lines_added          Nullable(Float64),
    total_lines_added    Nullable(Float64),
    accepted_lines_added Nullable(Float64),
    spec_lines           Nullable(Float64),
    session_count        Nullable(Float64),
    total_chat_messages  Nullable(Float64),
    _version             UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, day);
SQL
fi

# silver.class_git_commits — git dbt model.
if ! ch_table_exists silver class_git_commits; then
  echo "  Creating placeholder: silver.class_git_commits"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_git_commits (
    insight_tenant_id String,
    commit_hash       String,
    project_key       String,
    tenant_id         String,
    author_email      String,
    date              Date,
    is_merge_commit   UInt8,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (commit_hash);
SQL
fi

# silver.class_git_pull_requests — git dbt model.
if ! ch_table_exists silver class_git_pull_requests; then
  echo "  Creating placeholder: silver.class_git_pull_requests"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_git_pull_requests (
    insight_tenant_id String,
    pr_id             String,
    author_email      String,
    author_name       String,
    state             String,
    created_on        DateTime,
    merged_on         Nullable(DateTime),
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (pr_id);
SQL
fi

# silver.class_git_file_changes — git dbt model.
if ! ch_table_exists silver class_git_file_changes; then
  echo "  Creating placeholder: silver.class_git_file_changes"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_git_file_changes (
    insight_tenant_id String,
    commit_hash       String,
    project_key       String,
    tenant_id         String,
    file_path         String,
    lines_added       Int64,
    lines_removed     Int64,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (commit_hash, file_path);
SQL
fi

# silver.class_task_daily — task-tracking dbt model.
if ! ch_table_exists silver class_task_daily; then
  echo "  Creating placeholder: silver.class_task_daily"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_task_daily (
    insight_tenant_id String,
    person_id         String,
    metric_date       Date,
    tasks_closed      Float64,
    bugs_fixed        Float64,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_id, metric_date);
SQL
fi

# silver.class_task_field_history — task-tracking dbt model.
if ! ch_table_exists silver class_task_field_history; then
  echo "  Creating placeholder: silver.class_task_field_history"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_task_field_history (
    insight_tenant_id String,
    issue_id          String,
    field_name        String,
    old_value         String,
    new_value         String,
    changed_at        DateTime,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (issue_id, changed_at);
SQL
fi

# silver.mtr_git_person_totals — pre-aggregated git person metrics.
if ! ch_table_exists silver mtr_git_person_totals; then
  echo "  Creating placeholder: silver.mtr_git_person_totals"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.mtr_git_person_totals (
    insight_tenant_id    String,
    person_key           String,
    commits              UInt64,
    lines_added          Int64,
    lines_removed        Int64,
    loc                  Float64,
    prs_merged           Float64,
    avg_pr_cycle_time_h  Nullable(Float64),
    _version             UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_key);
SQL
fi

# silver.mtr_git_person_weekly — pre-aggregated git person weekly metrics.
if ! ch_table_exists silver mtr_git_person_weekly; then
  echo "  Creating placeholder: silver.mtr_git_person_weekly"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.mtr_git_person_weekly (
    insight_tenant_id String,
    person_key        String,
    week              Date,
    commits           UInt64,
    lines_added       Int64,
    lines_removed     Int64,
    prs_merged        Float64,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_key, week);
SQL
fi

# bronze_jira — needed by gold-views jira_person_daily, jira_closed_tasks
if ! ch_table_exists bronze_jira jira_issue; then
  echo "  Creating placeholder: bronze_jira.jira_issue"
  run_ch <<'SQL'
CREATE DATABASE IF NOT EXISTS bronze_jira;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue (
    id String,
    unique_key String,
    id_readable String,
    issue_type String,
    updated String,
    due_date String,
    custom_fields_json String,
    _airbyte_extracted_at DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY id;
SQL
fi

# bronze_m365 -- needed by gold-views teams_person_daily, files_person_daily, comms_daily.
# Each table is checked and created independently so a partially-seeded
# state (e.g. teams_activity exists, onedrive_activity does not) gets the
# missing ones repaired on a re-run.
run_ch <<'SQL'
CREATE DATABASE IF NOT EXISTS bronze_m365;
SQL
if ! ch_table_exists bronze_m365 teams_activity; then
  echo "  Creating placeholder: bronze_m365.teams_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_m365.teams_activity (
    userPrincipalName String,
    lastActivityDate String,
    teamChatMessageCount Nullable(Float64),
    privateChatMessageCount Nullable(Float64),
    meetingsAttendedCount Nullable(Float64),
    callCount Nullable(Float64),
    _airbyte_extracted_at DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY userPrincipalName;
SQL
fi
if ! ch_table_exists bronze_m365 onedrive_activity; then
  echo "  Creating placeholder: bronze_m365.onedrive_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_m365.onedrive_activity (
    userPrincipalName String,
    lastActivityDate String,
    sharedInternallyFileCount Nullable(Float64),
    sharedExternallyFileCount Nullable(Float64),
    _airbyte_extracted_at DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY userPrincipalName;
SQL
fi
if ! ch_table_exists bronze_m365 sharepoint_activity; then
  echo "  Creating placeholder: bronze_m365.sharepoint_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_m365.sharepoint_activity (
    userPrincipalName String,
    lastActivityDate String,
    sharedInternallyFileCount Nullable(Float64),
    sharedExternallyFileCount Nullable(Float64),
    _airbyte_extracted_at DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY userPrincipalName;
SQL
fi

# bronze_zoom — needed by gold-views comms_daily, zoom_person_daily
if ! ch_table_exists bronze_zoom participants; then
  echo "  Creating placeholder: bronze_zoom.participants"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_zoom.participants (
    email String,
    meeting_uuid String,
    join_time String,
    leave_time String,
    _airbyte_extracted_at DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY email;
SQL
fi

# bronze_cursor — needed by ic-kpis-honest-nulls, team-member-honest-nulls,
# bullet-views-honest-nulls. The Cursor Airbyte connector overwrites this on
# first sync (full schema in src/ingestion/connectors/ai/cursor/connector.yaml).
if ! ch_table_exists bronze_cursor cursor_daily_usage; then
  echo "  Creating placeholder: bronze_cursor.cursor_daily_usage"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_cursor.cursor_daily_usage (
    email                 String,
    day                   String,
    isActive              Nullable(UInt8),
    totalLinesAdded       Nullable(Float64),
    acceptedLinesAdded    Nullable(Float64),
    totalTabsShown        Nullable(Float64),
    totalTabsAccepted     Nullable(Float64),
    agentRequests         Nullable(Float64),
    chatRequests          Nullable(Float64),
    composerRequests      Nullable(Float64),
    _airbyte_extracted_at DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY (email, day);
SQL
fi

# bronze_bamboohr.employees — primary HR people source. Identity-resolution
# loads this at startup (with graceful fallback to empty store), and silver
# class_focus_metrics joins it via class_collab_meeting_activity.
if ! ch_table_exists bronze_bamboohr employees; then
  echo "  Creating placeholder: bronze_bamboohr.employees"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_bamboohr.employees (
    id                    String,
    status                String,
    firstName             Nullable(String),
    lastName              Nullable(String),
    displayName           Nullable(String),
    workEmail             String,
    department            Nullable(String),
    division              Nullable(String),
    jobTitle              Nullable(String),
    supervisorEmail       Nullable(String),
    supervisor            Nullable(String),
    _airbyte_extracted_at DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY id;
SQL
fi

# bronze_bitbucket_cloud.commits — git commits. Used by mtr_git_person_*
# silver upstream and gold ic_chart_loc.
if ! ch_table_exists bronze_bitbucket_cloud commits; then
  echo "  Creating placeholder: bronze_bitbucket_cloud.commits"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.commits (
    hash                  String,
    date                  String,
    author_raw            Nullable(String),
    author_email          Nullable(String),
    author_name           Nullable(String),
    project_key           Nullable(String),
    repository            Nullable(String),
    message               Nullable(String),
    _airbyte_extracted_at DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY hash;
SQL
fi

# bronze_bitbucket_cloud.pull_requests — git PRs.
if ! ch_table_exists bronze_bitbucket_cloud pull_requests; then
  echo "  Creating placeholder: bronze_bitbucket_cloud.pull_requests"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pull_requests (
    id                    String,
    state                 Nullable(String),
    author_email          Nullable(String),
    author_name           Nullable(String),
    created_on            Nullable(String),
    updated_on            Nullable(String),
    merged_on             Nullable(String),
    repository            Nullable(String),
    _airbyte_extracted_at DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY id;
SQL
fi

# bronze_slack.users_details — per-user, per-day Slack activity rollup
# (despite the "details" name, this stream carries activity counts —
# messages_posted_count / channel_messages_posted_count — keyed by date).
if ! ch_table_exists bronze_slack users_details; then
  echo "  Creating placeholder: bronze_slack.users_details"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_slack.users_details (
    email_address                 String,
    date                          String,
    messages_posted_count         Nullable(Float64),
    channel_messages_posted_count Nullable(Float64),
    _airbyte_extracted_at         DateTime64(3, 'UTC') DEFAULT now64(3)
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY (email_address, date);
SQL
fi

echo "=== Placeholders: done ==="
