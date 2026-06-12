#!/usr/bin/env bash
# Create empty placeholder bronze + silver tables that gold-view migrations
# (scripts/migrations/*.sql) reference but that do NOT exist on a fresh
# cluster. Without these, ClickHouse's CREATE VIEW validation fails with
# UNKNOWN_TABLE / UNKNOWN_DATABASE and init.sh aborts.
#
# Two classes of placeholders:
#   1. bronze_<source>.<stream>  — populated by Airbyte connectors. The
#                                  placeholder ships the four CDK v2
#                                  internal columns (see ADR-0007 rule 3),
#                                  so Airbyte's destination v2 accepts it
#                                  via `ensureSchemaMatches` and writes to
#                                  it in place on the first sync (rule 5).
#   2. silver.<dbt_model>        — built by `dbt run` (Argo workflow,
#                                  invoked AFTER init.sh registers it).
#                                  Each silver placeholder carries the
#                                  marker comment INSIGHT_PLACEHOLDER_v1;
#                                  the dbt on-run-start hook
#                                  `drop_silver_placeholders_at_start`
#                                  drops it on the first eligible run so
#                                  the model materialises with its real
#                                  schema (ADR-0007 rule 5).
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
#
# Bronze placeholders include the four Airbyte CDK v2 internal columns
# (_airbyte_raw_id, _airbyte_extracted_at, _airbyte_meta,
# _airbyte_generation_id). Airbyte's ClickHouse destination v2 calls
# `ensureSchemaMatches` on every sync and refuses to write to a table
# missing them; including them here makes the placeholder a valid drop-in
# from day zero. See ADR-0007 §Decision rule 3.
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
#
# Each silver placeholder carries `COMMENT 'INSIGHT_PLACEHOLDER_v1'` so the
# dbt `drop_silver_placeholders_at_start` macro (see
# src/ingestion/dbt/macros/drop_silver_placeholders_at_start.sql) can detect
# and drop it on the first real dbt run via the project-level
# `on-run-start` hook, before the silver model rebuilds the table with its
# full schema. This is the bridge that keeps placeholder schema drift from
# corrupting silver writes.
#
# The marker + the macro can be retired once gold-view migrations are
# split into a post-dbt phase (Variant A in ADR-0007's "Better fixes"
# section) — at that point silver tables will be created exclusively by
# dbt, never as init.sh stubs.
#
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (user_email, activity_date) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, day) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
    cost_cents           Nullable(UInt32),
    prs_with_cc_count    Nullable(UInt32),
    prs_total_count      Nullable(UInt32),
    _version             UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, day) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_support_activity — support (Zendesk) dbt model. Referenced by the
# support_bullet_rows gold view (20260611000000_support-bullet-rows.sql); without
# this placeholder CREATE VIEW fails at migration time (CH 24.x validates view
# source tables) in any env where dbt hasn't built the silver model yet.
if ! ch_table_exists silver class_support_activity; then
  echo "  Creating placeholder: silver.class_support_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_support_activity (
    tenant_id           String,
    insight_source_id   String,
    unique_key          String,
    data_source         String,
    person_key          String,
    email               String,
    date                Date,
    updates             Nullable(UInt32),
    public_comments     Nullable(UInt32),
    private_comments    Nullable(UInt32),
    solved              Nullable(UInt32),
    kb_articles_created Nullable(UInt32),
    csat_good           UInt32,
    csat_total          UInt32,
    collected_at        DateTime,
    _version            UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_key, date) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_ai_api_usage — programmatic AI API token usage (Claude Admin
# messages_usage; future OpenAI). Schema mirrors `silver/ai/class_ai_api_usage`
# dbt model order_by=['unique_key'] config — email is always NULL by design
# (API keys can't be attributed to users at request time; resolution happens
# in Silver Step 2 via api_key_id → person_id). dbt drops & replaces this
# placeholder on first run.
if ! ch_table_exists silver class_ai_api_usage; then
  echo "  Creating placeholder: silver.class_ai_api_usage"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_ai_api_usage (
    insight_tenant_id     Nullable(String),
    source_id             Nullable(String),
    unique_key            String,
    email                 Nullable(String),
    api_key_id            Nullable(String),
    workspace_id          Nullable(String),
    day                   Nullable(Date),
    provider              String,
    channel               String,
    input_tokens          Nullable(UInt64),
    output_tokens         Nullable(UInt64),
    cache_read_tokens     Nullable(UInt64),
    cache_creation_tokens Nullable(UInt64),
    cost_amount           Nullable(Decimal(18, 4)),
    cost_currency         Nullable(String),
    source                String,
    data_source           String,
    collected_at          Nullable(DateTime64(3)),
    _version              UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_ai_assistant_usage — per-person per-day AI assistant surface
# usage (Claude Enterprise chat / cowork / office / cross). One row per
# (tenant, email, day, surface). Schema mirrors `silver/ai/class_ai_assistant_usage`
# dbt model order_by=['unique_key'] config. dbt drops & replaces this
# placeholder on first run.
if ! ch_table_exists silver class_ai_assistant_usage; then
  echo "  Creating placeholder: silver.class_ai_assistant_usage"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_ai_assistant_usage (
    insight_tenant_id        String,
    source_id                String,
    unique_key               String,
    email                    String,
    day                      Date,
    tool                     String,
    surface                  String,
    session_count            Nullable(UInt32),
    conversation_count       Nullable(UInt32),
    message_count            Nullable(UInt32),
    action_count             Nullable(UInt32),
    files_uploaded_count     Nullable(UInt32),
    artifacts_created_count  Nullable(UInt32),
    projects_created_count   Nullable(UInt32),
    projects_used_count      Nullable(UInt32),
    skills_used_count        Nullable(UInt32),
    connectors_used_count    Nullable(UInt32),
    thinking_message_count   Nullable(UInt32),
    dispatch_turn_count      Nullable(UInt32),
    search_count             Nullable(UInt32),
    cost_cents               Nullable(UInt32),
    surface_metrics_json     Nullable(String),
    source                   String,
    data_source              String,
    collected_at             Nullable(DateTime64(3)),
    _version                 UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_people — identity dbt model. Used by crm-gold-views and any
# future migration that joins person → department / org-unit. Minimum-viable
# columns the crm gold views select.
if ! ch_table_exists silver class_people; then
  echo "  Creating placeholder: silver.class_people"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_people (
    unique_key      String,
    email           Nullable(String),
    department_name Nullable(String),
    _version        UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_crm_users — CRM dbt model (HubSpot owners + Salesforce users).
# Used by crm-gold-views to resolve activity/deal owner_id and
# hs_created_by_user_id back to canonical email.
if ! ch_table_exists silver class_crm_users; then
  echo "  Creating placeholder: silver.class_crm_users"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_crm_users (
    unique_key String,
    user_id    String,
    hs_user_id Nullable(String),
    email      Nullable(String),
    _version   UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_crm_deals — CRM dbt model. Used by crm-gold-views for
# pipeline-now / closed-won aggregates and the weekly deal-flow chart.
if ! ch_table_exists silver class_crm_deals; then
  echo "  Creating placeholder: silver.class_crm_deals"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_crm_deals (
    unique_key  String,
    created_at  Nullable(DateTime64(3)),
    close_date  Nullable(Date),
    is_won      Int8,
    is_closed   Int8,
    amount_home Nullable(Float64),
    owner_id    Nullable(String),
    _version    UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_crm_activities — CRM dbt model (HubSpot engagements +
# Salesforce tasks/events). Used by crm-gold-views for outreach-activity
# bullets (calls / emails / meetings / tasks).
if ! ch_table_exists silver class_crm_activities; then
  echo "  Creating placeholder: silver.class_crm_activities"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_crm_activities (
    unique_key         String,
    timestamp          Nullable(DateTime64(3)),
    activity_type      String,
    owner_id           Nullable(String),
    created_by_user_id Nullable(String),
    _version           UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (commit_hash) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (pr_id) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (commit_hash, file_path) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_id, metric_date) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_task_field_history — task-tracking event-sourced field history
# (per ADR-005). Schema mirrors the canonical staging table built by the
# `create_task_field_history_staging` macro (see src/ingestion/dbt/macros/) —
# silver is a thin SELECT * from staging via union_by_tag so the target
# columns match. Migrations like 20260427120000_views-from-silver.sql and
# 20260429000000_task-delivery-silver-rewrite.sql aggregate over
# (insight_source_id, data_source, issue_id, event_at, _version, field_id,
# value_displays, value_ids, delta_action, event_kind) so all of these
# need to exist in the placeholder for CREATE VIEW to type-check.
if ! ch_table_exists silver class_task_field_history; then
  echo "  Creating placeholder: silver.class_task_field_history"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_task_field_history (
    unique_key          String,
    insight_source_id   String,
    data_source         String,
    issue_id            String,
    id_readable         String,
    event_id            String,
    event_at            DateTime64(3),
    event_kind          Enum8('changelog' = 1, 'synthetic_initial' = 2),
    _seq                UInt32,
    author_id           Nullable(String),
    author_display      Nullable(String),
    field_id            String,
    field_name          String,
    field_cardinality   Enum8('single' = 1, 'multi' = 2),
    delta_action        Enum8('set' = 1, 'add' = 2, 'remove' = 3),
    delta_value_id      Nullable(String),
    delta_value_display Nullable(String),
    value_ids           Array(String),
    value_displays      Array(String),
    value_id_type       Enum8('opaque_id' = 1, 'account_id' = 2, 'string_literal' = 3, 'path' = 4, 'none' = 5),
    collected_at        DateTime64(3),
    _version            UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_task_users — task-tracking user directory (anchor for identity
# resolution). Referenced by `views-from-silver.sql` LEFT JOIN to look up
# `email` by `(insight_source_id, user_id)` for the assignee_email column.
if ! ch_table_exists silver class_task_users; then
  echo "  Creating placeholder: silver.class_task_users"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_task_users (
    insight_tenant_id String,
    insight_source_id String,
    user_id           String,
    email             Nullable(String),
    unique_key        String,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_task_worklogs — task-tracking worklog rows. Referenced by
# `views-from-silver.sql` for time-spent aggregations
# (author_email/author_id, work_date, duration_seconds/worklog_seconds).
if ! ch_table_exists silver class_task_worklogs; then
  echo "  Creating placeholder: silver.class_task_worklogs"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_task_worklogs (
    insight_tenant_id String,
    insight_source_id String,
    worklog_id        String,
    issue_id          Nullable(String),
    author_id         Nullable(String),
    author_email      Nullable(String),
    work_date         Nullable(Date),
    duration_seconds  Nullable(Float64),
    worklog_seconds   Nullable(Float64),
    unique_key        String,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_wiki_activity — per-user per-day wiki edit activity. Referenced
# by 20260505000000_drop-confluence-minor-edits.sql (ALTER TABLE DROP COLUMN
# IF EXISTS) — ALTER fails with UNKNOWN_TABLE if the silver target itself
# does not exist on a fresh cluster.
if ! ch_table_exists silver class_wiki_activity; then
  echo "  Creating placeholder: silver.class_wiki_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_wiki_activity (
    tenant_id     String,
    source_id     String,
    unique_key    String,
    author_id     String,
    author_email  Nullable(String),
    day           Date,
    pages_edited  Nullable(UInt32),
    total_edits   Nullable(UInt32),
    pages_created Nullable(UInt32),
    major_edits   Nullable(UInt32),
    minor_edits   Nullable(UInt32),
    _version      UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_key) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_key, week) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
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
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY (email_address, date);
SQL
fi

echo "=== Placeholders: done ==="
