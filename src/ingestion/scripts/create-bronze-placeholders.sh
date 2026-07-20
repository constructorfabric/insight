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

# ClickHouse access helpers (run_ch, ch_table_exists) that talk to the
# external ClickHouse over HTTP. Requires CLICKHOUSE_URL/USER/PASSWORD in
# the env — set by the clickhouse-migrate Hook Job. See lib/ch-exec.sh.
source "$SCRIPT_DIR/lib/ch-exec.sh"

echo "=== Placeholders (for missing connectors / unbuilt silver) ==="

run_ch <<'SQL'
-- `insight` is normally created before this script runs (k8s: the
-- clickhouse-init-svcdbs Job; compose: /docker-entrypoint-initdb.d on
-- first CH start). Asserting it here keeps the compose seed idempotent
-- after an operator wipe (DROP DATABASE insight SYNC for a re-bootstrap)
-- — the initdb script does not re-run on an existing volume.
CREATE DATABASE IF NOT EXISTS insight;
CREATE DATABASE IF NOT EXISTS silver;
CREATE DATABASE IF NOT EXISTS bronze_jira;
CREATE DATABASE IF NOT EXISTS bronze_m365;
CREATE DATABASE IF NOT EXISTS bronze_zoom;
CREATE DATABASE IF NOT EXISTS bronze_cursor;
CREATE DATABASE IF NOT EXISTS bronze_slack;
CREATE DATABASE IF NOT EXISTS bronze_bamboohr;
CREATE DATABASE IF NOT EXISTS bronze_bitbucket_cloud;
CREATE DATABASE IF NOT EXISTS bronze_zulip_proxy;
CREATE DATABASE IF NOT EXISTS bronze_claude_team;
CREATE DATABASE IF NOT EXISTS bronze_claude_enterprise;
CREATE DATABASE IF NOT EXISTS bronze_outline;
CREATE DATABASE IF NOT EXISTS bronze_confluence;
CREATE DATABASE IF NOT EXISTS bronze_chatgpt_team;
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
    -- `tenant_id` is the column the real dbt model emits (connectors SELECT
    -- `tenant_id`); the gold model collab_metric_observations reads it. The
    -- placeholder must carry it too or the deploy-time gold build fails with
    -- UNKNOWN_IDENTIFIER. Mirrors the class_git_* placeholders (both cols).
    tenant_id         String,
    email             String,
    person_key        String,
    date              Date,
    data_source       String,
    sent_count        Float64,
    received_count    Float64,
    read_count        Float64,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  # Reconcile a pre-existing PLACEHOLDER (warm cluster / re-deploy / warm e2e
  # ClickHouse) to the tenant_id column contract — a bare CREATE ... IF NOT
  # EXISTS never adds the column to an already-created table. Guarded on the
  # marker so a real dbt-built table (which already has tenant_id) is left
  # untouched. Idempotent: ADD COLUMN IF NOT EXISTS.
  class_collab_email_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_collab_email_activity' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_collab_email_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_collab_email_activity"
    run_ch <<'SQL'
ALTER TABLE silver.class_collab_email_activity ADD COLUMN IF NOT EXISTS tenant_id String;
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_collab_email_activity is not a placeholder"
  fi
fi

# silver.class_collab_meeting_activity — collaboration dbt model.
if ! ch_table_exists silver class_collab_meeting_activity; then
  echo "  Creating placeholder: silver.class_collab_meeting_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_collab_meeting_activity (
    insight_tenant_id              String,
    -- `tenant_id` (real dbt/connector column, read by gold) — see the
    -- class_collab_email_activity note above. meetings_organized /
    -- adhoc_meetings_attended / scheduled_meetings_attended are likewise
    -- selected by collab_metric_observations and must exist here.
    tenant_id                      String,
    email                          String,
    person_key                     String,
    date                           Date,
    data_source                    String,
    meetings_attended              Float64,
    meetings_organized             Float64,
    adhoc_meetings_attended        Float64,
    scheduled_meetings_attended    Float64,
    calls_count                    Float64,
    participants                   Float64,
    audio_duration_seconds         Float64,
    video_duration_seconds         Float64,
    screen_share_duration_seconds  Float64,
    _version                       UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  # Reconcile a pre-existing PLACEHOLDER to the current column contract —
  # tenant_id plus the meeting counts collab_metric_observations reads. See
  # the class_collab_email_activity note above. Idempotent + marker-guarded.
  class_collab_meeting_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_collab_meeting_activity' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_collab_meeting_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_collab_meeting_activity"
    run_ch <<'SQL'
ALTER TABLE silver.class_collab_meeting_activity ADD COLUMN IF NOT EXISTS tenant_id String;
ALTER TABLE silver.class_collab_meeting_activity ADD COLUMN IF NOT EXISTS meetings_organized Float64;
ALTER TABLE silver.class_collab_meeting_activity ADD COLUMN IF NOT EXISTS adhoc_meetings_attended Float64;
ALTER TABLE silver.class_collab_meeting_activity ADD COLUMN IF NOT EXISTS scheduled_meetings_attended Float64;
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_collab_meeting_activity is not a placeholder"
  fi
fi

# silver.class_collab_chat_activity — collaboration dbt model.
if ! ch_table_exists silver class_collab_chat_activity; then
  echo "  Creating placeholder: silver.class_collab_chat_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_collab_chat_activity (
    insight_tenant_id             String,
    -- `tenant_id` (real dbt/connector column, read by gold) — see the
    -- class_collab_email_activity note above. `direct_and_group_messages`
    -- (#266) is also selected by collab_metric_observations; the
    -- apply-ch-migrations.sh heal only ADDs it to a REAL table, never to a
    -- placeholder, so a fresh deploy needs it declared here.
    tenant_id                     String,
    email                         String,
    person_key                    String,
    date                          Date,
    data_source                   String,
    total_chat_messages           Float64,
    channel_messages_posted_count Float64,
    channel_posts                 Float64,
    channel_replies               Float64,
    direct_and_group_messages     Nullable(Int64),
    _version                      UInt64
-- `data_source` in the sort key: the compose seed writes one row per
-- source (insight_m365 + insight_slack) for the same (email, date);
-- without it ReplacingMergeTree collapses them.
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  # Reconcile a pre-existing PLACEHOLDER to the current column contract —
  # tenant_id plus direct_and_group_messages (#266). The apply-ch-migrations.sh
  # heal only ALTERs a REAL table (it skips placeholders), so the placeholder
  # itself must be reconciled here. See the class_collab_email_activity note
  # above. Idempotent + marker-guarded.
  class_collab_chat_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_collab_chat_activity' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_collab_chat_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_collab_chat_activity"
    run_ch <<'SQL'
ALTER TABLE silver.class_collab_chat_activity ADD COLUMN IF NOT EXISTS tenant_id String;
ALTER TABLE silver.class_collab_chat_activity ADD COLUMN IF NOT EXISTS direct_and_group_messages Nullable(Int64);
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_collab_chat_activity is not a placeholder"
  fi
fi

# silver.class_collab_document_activity — collaboration dbt model.
if ! ch_table_exists silver class_collab_document_activity; then
  echo "  Creating placeholder: silver.class_collab_document_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_collab_document_activity (
    insight_tenant_id        String,
    -- `tenant_id` (real dbt/connector column, read by gold) — see the
    -- class_collab_email_activity note above.
    tenant_id                String,
    email                    String,
    person_key               String,
    date                     Date,
    data_source              String,
    shared_internally_count  Float64,
    shared_externally_count  Float64,
    viewed_or_edited_count   Float64,
    _version                 UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  # Reconcile a pre-existing PLACEHOLDER to the tenant_id column contract.
  # See the class_collab_email_activity note above. Idempotent + marker-guarded.
  class_collab_document_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_collab_document_activity' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_collab_document_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_collab_document_activity"
    run_ch <<'SQL'
ALTER TABLE silver.class_collab_document_activity ADD COLUMN IF NOT EXISTS tenant_id String;
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_collab_document_activity is not a placeholder"
  fi
fi

# silver.class_ai_dev_usage — AI dbt model. Aggregates Cursor + Claude
# Code + others.
if ! ch_table_exists silver class_ai_dev_usage; then
  echo "  Creating placeholder: silver.class_ai_dev_usage"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_ai_dev_usage (
    insight_tenant_id    String,
    source_id            String DEFAULT '',
    unique_key           String,
    email                String,
    api_key_id           Nullable(String),
    day                  Date,
    tool                 String,
    is_active            UInt8,
    conversation_count   Nullable(Float64),
    agent_sessions       Nullable(Float64),
    chat_requests        Nullable(Float64),
    tool_use_offered     Nullable(Float64),
    tool_use_accepted    Nullable(Float64),
    lines_added          Nullable(Float64),
    lines_removed        Nullable(Float64),
    total_lines_added    Nullable(Float64),
    total_lines_removed  Nullable(Float64),
    accepted_lines_added Nullable(Float64),
    spec_lines           Nullable(Float64),
    session_count        Nullable(Float64),
    total_chat_messages  Nullable(Float64),
    -- Nullable(Float64), not Nullable(UInt32): the compose seed
    -- (deploy/seed) inserts float() values into these five.
    cost_cents           Nullable(Float64),
    commits_count        Nullable(Float64),
    pull_requests_count  Nullable(Float64),
    prs_with_cc_count    Nullable(Float64),
    prs_total_count      Nullable(Float64),
    tool_action_breakdown_json Nullable(String),
    source               String DEFAULT '',
    data_source          String,
    collected_at         Nullable(DateTime64(3)),
    _version             UInt64
-- `tool` MUST be in the sort key — the compose seed writes cursor and
-- claude_code rows for the same (email, day); without it,
-- ReplacingMergeTree collapses them, suppressing whichever was
-- inserted first.
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, day, tool) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  class_ai_dev_usage_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_ai_dev_usage' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_ai_dev_usage_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_ai_dev_usage"
    run_ch <<'SQL'
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS source_id String;
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS unique_key String;
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS api_key_id Nullable(String);
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS conversation_count Nullable(Float64);
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS lines_removed Nullable(Float64);
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS total_lines_removed Nullable(Float64);
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS commits_count Nullable(Float64);
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS pull_requests_count Nullable(Float64);
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS tool_action_breakdown_json Nullable(String);
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS source String;
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS data_source String;
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS collected_at Nullable(DateTime64(3));
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_ai_dev_usage is not a placeholder"
  fi
fi

# silver.class_ai_overage — per-seat AI spend-vs-limit (Claude Team). Referenced
# by the cc_overage branch of the ai_bullet_rows gold view
# (20260618000000_ai-claude-team-overage-gold.sql); without this placeholder
# CREATE VIEW fails at migration time (CH 24.x validates view source tables) in
# any env where dbt hasn't built the silver model yet.
if ! ch_table_exists silver class_ai_overage; then
  echo "  Creating placeholder: silver.class_ai_overage"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_ai_overage (
    insight_tenant_id    String,
    source_id            String,
    unique_key           String,
    email                String,
    account_id           String,
    period_month         Date,
    tool                 String,
    seat_tier            Nullable(String),
    currency             String,
    credit_limit_cents   Nullable(UInt32),
    used_amount_cents    UInt32,
    overage_cents        Nullable(UInt32),
    is_over_limit        Nullable(UInt8),
    is_enabled           Nullable(UInt8),
    overage_metrics_json String,
    source               String,
    data_source          String,
    collected_at         Nullable(DateTime64(3)),
    _version             UInt64
-- ORDER BY unique_key mirrors the real dbt model (unique_key is
-- tenant-source-seat-YYYY-MM, one row per seat per month).
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
    -- insight_tenant_id is what the compose seed writes; tenant_id is
    -- the column the real dbt model produces. Both live here until the
    -- seed generators are aligned with the dbt naming (ADR-0007 debt).
    insight_tenant_id   String,
    insight_source_id   String,
    unique_key          String,
    data_source         String,
    person_key          String,
    email               String,
    date                Date,
    -- Nullable(Float64), not Nullable(UInt32): the compose seed inserts
    -- float() values (real dbt model: UInt32 countIf aggregates).
    updates             Nullable(Float64),
    public_comments     Nullable(Float64),
    private_comments    Nullable(Float64),
    solved              Nullable(Float64),
    kb_articles_created Nullable(UInt32),
    csat_good           Nullable(Float64),
    csat_total          Nullable(Float64),
    collected_at        DateTime,
    _version            UInt64
-- `data_source` in the sort key: multi-source rows for the same
-- (person_key, date) must not collapse under ReplacingMergeTree.
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_key, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
else
  class_ai_assistant_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_ai_assistant_usage' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_ai_assistant_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_ai_assistant_usage"
    run_ch <<'SQL'
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_ai_assistant_usage is not a placeholder"
  fi
fi

# silver.class_people — identity dbt model. Used by crm-gold-views and any
# future migration that joins person → department / org-unit. Minimum-viable
# columns the crm gold views select.
if ! ch_table_exists silver class_people; then
  echo "  Creating placeholder: silver.class_people"
  run_ch <<'SQL'
# Versionless RMT to match the dbt model (silver/_shared/class_people.sql
# unions staging with dedup_version_col=none — no `_version` column).
CREATE TABLE IF NOT EXISTS silver.class_people (
    unique_key      String,
    workspace_id    String DEFAULT '',
    email           Nullable(String),
    valid_from      Nullable(String),
    department_name Nullable(String),
    org_unit_id     Nullable(String)
) ENGINE = ReplacingMergeTree ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  class_people_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_people' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_people_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_people"
    run_ch <<'SQL'
ALTER TABLE silver.class_people ADD COLUMN IF NOT EXISTS workspace_id String DEFAULT '';
ALTER TABLE silver.class_people ADD COLUMN IF NOT EXISTS valid_from Nullable(String);
ALTER TABLE silver.class_people ADD COLUMN IF NOT EXISTS org_unit_id Nullable(String);
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_people is not a placeholder"
  fi
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
    repo_slug         String  DEFAULT '',
    tenant_id         String,
    source_id         String  DEFAULT '',
    author_email      String,
    date              Date,
    is_merge_commit   UInt8,
    file_path         String  DEFAULT '',
    data_source       String  DEFAULT '',
    -- Non-Nullable so `toFloat64(sum(c.lines_added + c.lines_removed))`
    -- in the git_bullet_rows view stays Float64 (the view's structure
    -- declares metric_value as Float64, not Nullable(Float64)).
    lines_added       Float64 DEFAULT 0,
    lines_removed     Float64 DEFAULT 0,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (commit_hash) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  class_git_commits_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_git_commits' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_git_commits_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_git_commits"
    run_ch <<'SQL'
ALTER TABLE silver.class_git_commits ADD COLUMN IF NOT EXISTS source_id String DEFAULT '';
ALTER TABLE silver.class_git_commits ADD COLUMN IF NOT EXISTS data_source String DEFAULT '';
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_git_commits is not a placeholder"
  fi
fi

# silver.class_git_pull_requests — git dbt model.
if ! ch_table_exists silver class_git_pull_requests; then
  echo "  Creating placeholder: silver.class_git_pull_requests"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_git_pull_requests (
    insight_tenant_id String,
    pr_id             Int64,
    tenant_id         String  DEFAULT '',
    source_id         String  DEFAULT '',
    project_key       String  DEFAULT '',
    repo_slug         String  DEFAULT '',
    author_email      String,
    author_name       String,
    state             String,
    created_on        DateTime,
    merged_on         Nullable(DateTime),
    closed_on         Nullable(DateTime),
    data_source       String  DEFAULT '',
    -- Non-Nullable on purpose. The git_bullet_rows view's UNION branch
    -- for `pr_size` declares the column as Float64 (non-null); a
    -- Nullable placeholder makes the UNION type Nullable, which then
    -- collides with the view structure under join_use_nulls=1.
    lines_added       Float64 DEFAULT 0,
    lines_removed     Float64 DEFAULT 0,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (pr_id) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  class_git_prs_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_git_pull_requests' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_git_prs_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_git_pull_requests"
    run_ch <<'SQL'
ALTER TABLE silver.class_git_pull_requests ADD COLUMN IF NOT EXISTS tenant_id String DEFAULT '';
ALTER TABLE silver.class_git_pull_requests ADD COLUMN IF NOT EXISTS source_id String DEFAULT '';
ALTER TABLE silver.class_git_pull_requests ADD COLUMN IF NOT EXISTS project_key String DEFAULT '';
ALTER TABLE silver.class_git_pull_requests ADD COLUMN IF NOT EXISTS repo_slug String DEFAULT '';
ALTER TABLE silver.class_git_pull_requests ADD COLUMN IF NOT EXISTS data_source String DEFAULT '';
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_git_pull_requests is not a placeholder"
  fi
fi

# silver.class_git_pull_requests_commits — git dbt model. Links pull
# requests to their commits; the unified git gold view resolves PR author
# identity through it.
if ! ch_table_exists silver class_git_pull_requests_commits; then
  echo "  Creating placeholder: silver.class_git_pull_requests_commits"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_git_pull_requests_commits (
    tenant_id   String,
    source_id   String DEFAULT '',
    project_key String DEFAULT '',
    repo_slug   String DEFAULT '',
    pr_id       Int64,
    commit_hash String,
    data_source String DEFAULT '',
    _version    UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (pr_id, commit_hash) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
    repo_slug         String DEFAULT '',
    tenant_id         String,
    source_id         String DEFAULT '',
    file_path         String,
    lines_added       Int64,
    lines_removed     Int64,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (commit_hash, file_path) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  class_git_fc_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_git_file_changes' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_git_fc_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_git_file_changes"
    run_ch <<'SQL'
ALTER TABLE silver.class_git_file_changes ADD COLUMN IF NOT EXISTS source_id String DEFAULT '';
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_git_file_changes is not a placeholder"
  fi
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
# `email` by `(insight_source_id, user_id)` for the assignee_email column, and
# by the gold task observation chain (tenant + email attribution), which
# builds against this placeholder on a fresh instance's first deploy.
if ! ch_table_exists silver class_task_users; then
  echo "  Creating placeholder: silver.class_task_users"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_task_users (
    insight_tenant_id String,
    tenant_id         Nullable(String),
    insight_source_id String,
    user_id           String,
    email             Nullable(String),
    unique_key        String,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  # Reconcile a pre-existing PLACEHOLDER to the tenant_id column contract —
  # a bare CREATE ... IF NOT EXISTS never adds the column to an
  # already-created table, and the deploy-time gold build reads it. Guarded
  # on the marker so a real dbt-built table (healed separately) is left
  # untouched. Idempotent: ADD COLUMN IF NOT EXISTS.
  class_task_users_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='class_task_users' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$class_task_users_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.class_task_users"
    run_ch <<'SQL'
ALTER TABLE silver.class_task_users ADD COLUMN IF NOT EXISTS tenant_id Nullable(String);
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.class_task_users is not a placeholder"
  fi
fi

# silver.class_task_statuses — task-tracking status dimension (issue #1541).
# Referenced by the task-delivery gold migration
# (20260708000000_task-delivery-status-category.sql) which JOINs it to derive
# `status_category` (done/in_progress) for close-detection. Must exist before
# that migration's CREATE ... LEFT JOIN silver.class_task_statuses.
if ! ch_table_exists silver class_task_statuses; then
  echo "  Creating placeholder: silver.class_task_statuses"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_task_statuses (
    insight_source_id String,
    data_source       String,
    status_id         String,
    status_name       Nullable(String),
    category_id       Nullable(Int32),
    category_key      Nullable(String),
    status_category   String,
    collected_at      DateTime64(3),
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

# silver.class_wiki_pages — per-page snapshot. Referenced by the wiki gold
# view 20260620000000_wiki-bullet-rows.sql; CREATE VIEW fails with
# UNKNOWN_TABLE on a fresh cluster (migrations run before dbt builds silver)
# unless this placeholder exists. Columns mirror the dbt model (the view
# reads tenant_id/page_id/author_id/author_email/version_count/created_at).
if ! ch_table_exists silver class_wiki_pages; then
  echo "  Creating placeholder: silver.class_wiki_pages"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_wiki_pages (
    tenant_id         Nullable(String),
    source_id         Nullable(String),
    unique_key        String,
    page_id           Nullable(String),
    space_id          Nullable(String),
    space_name        Nullable(String),
    title             Nullable(String),
    status            Nullable(String),
    author_id         Nullable(String),
    author_email      Nullable(String),
    last_editor_id    Nullable(String),
    last_editor_email Nullable(String),
    parent_page_id    Nullable(String),
    version_count     UInt32,
    created_at        Nullable(DateTime64(3)),
    updated_at        Nullable(DateTime64(3)),
    space_url         Nullable(String),
    source            String,
    data_source       String,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
fi

# silver.class_wiki_engagement — per-page per-day comment engagement.
# Referenced by the wiki gold view 20260620000000_wiki-bullet-rows.sql
# (reads tenant_id/page_id/day/total_comments). Same fresh-cluster CREATE
# VIEW guard as class_wiki_pages above.
if ! ch_table_exists silver class_wiki_engagement; then
  echo "  Creating placeholder: silver.class_wiki_engagement"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS silver.class_wiki_engagement (
    tenant_id               Nullable(String),
    source_id               Nullable(String),
    unique_key              String,
    page_id                 Nullable(String),
    day                     Nullable(Date),
    total_comments          UInt32,
    footer_comments         Nullable(UInt32),
    inline_comments         Nullable(UInt32),
    replies                 Nullable(UInt32),
    unique_commenters       Nullable(UInt32),
    unresolved_inline_count Nullable(UInt32),
    source                  String,
    data_source             String,
    collected_at            Nullable(DateTime64(3)),
    _version                UInt64
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
    tenant_id         String,
    person_key        String,
    week              Date,
    unique_key        String,
    commits           UInt64,
    prs_merged        UInt64,
    code_loc          Int64,
    spec_lines        Int64,
    -- config_loc: referenced by ic_chart_loc (migration 20260624
    -- ic-chart-loc-git-breakdown); the view CREATE succeeds without it
    -- but SELECTing the view throws UNKNOWN_IDENTIFIER.
    config_loc        Int64,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_key, week) COMMENT 'INSIGHT_PLACEHOLDER_v1';
SQL
else
  # Reconcile a pre-existing placeholder (only — never a dbt-built table,
  # which already has the column) to the ic_chart_loc column contract.
  mtr_git_person_weekly_placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='silver' AND name='mtr_git_person_weekly' AND comment='INSIGHT_PLACEHOLDER_v1'" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  if [[ "$mtr_git_person_weekly_placeholder_count" == "1" ]]; then
    echo "  Reconciling placeholder schema: silver.mtr_git_person_weekly"
    run_ch <<'SQL'
ALTER TABLE silver.mtr_git_person_weekly ADD COLUMN IF NOT EXISTS config_loc Int64;
SQL
  else
    echo "  Skipping placeholder schema reconciliation: silver.mtr_git_person_weekly is not a placeholder"
  fi
fi

# bronze_jira — needed by gold-views jira_person_daily, jira_closed_tasks
# Column set mirrors what the Jira staging dbt models actually read (verified
# against connectors/task-tracking/jira/dbt/jira__issue_field_snapshot.sql and
# jira__changelog_items.sql): the snapshot does `LIMIT 1 BY source_id, jira_id`
# and extracts status/assignee/issuetype/… out of `custom_fields_json`, plus
# `created`, `parent_id`, `project_key`. The minimum-viable original placeholder
# only carried the handful of columns the legacy gold-view migrations type-check;
# the additional columns are additive and the real Airbyte sync still overwrites
# the schema in prod.
if ! ch_table_exists bronze_jira jira_issue; then
  echo "  Creating placeholder: bronze_jira.jira_issue"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue (
    id String,
    source_id String,
    jira_id String,
    unique_key String,
    id_readable String,
    issue_type String,
    created String,
    updated String,
    due_date String,
    parent_id String,
    project_key String,
    reporter_id Nullable(String),
    custom_fields_json String,
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY id;
SQL
else
  # Reconcile a pre-existing placeholder to the current column contract — the
  # snapshot/changelog staging models and jira-enrich read these columns, and a
  # `CREATE TABLE IF NOT EXISTS` alone never adds them to an older table (e.g. a
  # warm e2e ClickHouse from a prior run). Idempotent: ADD COLUMN IF NOT EXISTS.
  echo "  Reconciling placeholder schema: bronze_jira.jira_issue"
  run_ch <<'SQL'
ALTER TABLE bronze_jira.jira_issue ADD COLUMN IF NOT EXISTS source_id String;
ALTER TABLE bronze_jira.jira_issue ADD COLUMN IF NOT EXISTS jira_id String;
ALTER TABLE bronze_jira.jira_issue ADD COLUMN IF NOT EXISTS created String;
ALTER TABLE bronze_jira.jira_issue ADD COLUMN IF NOT EXISTS parent_id String;
ALTER TABLE bronze_jira.jira_issue ADD COLUMN IF NOT EXISTS project_key String;
ALTER TABLE bronze_jira.jira_issue ADD COLUMN IF NOT EXISTS reporter_id Nullable(String);
SQL
fi

# bronze_jira.jira_user — identity anchor. jira__task_users (silver:class_task_users)
# is a PURE dbt projection of this table, so seeding it lets the e2e rig resolve a
# Jira assignee account_id → email without the Rust jira-enrich step. Columns mirror
# connectors/task-tracking/jira/dbt/jira__task_users.sql.
if ! ch_table_exists bronze_jira jira_user; then
  echo "  Creating placeholder: bronze_jira.jira_user"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_jira.jira_user (
    unique_key String,
    source_id String,
    -- tenant_id is read by confluence__wiki_pages' jira_user join (account_id →
    -- email identity resolution). The real Airbyte stream carries it; the
    -- placeholder omitted it, which broke that model's compile on a fresh
    -- cluster. Nullable so connectors that seed jira_user without it still load.
    tenant_id Nullable(String),
    account_id String,
    email Nullable(String),
    display_name Nullable(String),
    account_type Nullable(String),
    active Nullable(Bool),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
else
  # Reconcile a pre-existing jira_user (warm cluster, or one already created by
  # the Jira connector before this column was added): the create branch above is
  # skipped, so add tenant_id in place. confluence__wiki_pages' jira_user join
  # reads it and fails to compile otherwise. Idempotent (ADD COLUMN IF NOT EXISTS).
  echo "  Reconciling placeholder schema: bronze_jira.jira_user (tenant_id)"
  run_ch <<'SQL'
ALTER TABLE bronze_jira.jira_user ADD COLUMN IF NOT EXISTS tenant_id Nullable(String);
SQL
fi

# bronze_jira.jira_issue_history — Jira changelog. jira__changelog_items.sql
# explodes the `items` JSON array into one row per field change; the Rust
# jira-enrich binary consumes the resulting staging.jira_changelog_items.
# Columns mirror what that staging model reads.
if ! ch_table_exists bronze_jira jira_issue_history; then
  echo "  Creating placeholder: bronze_jira.jira_issue_history"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue_history (
    unique_key String,
    source_id String,
    tenant_id String,
    id_readable String,
    changelog_id String,
    created_at String,
    author_account_id Nullable(String),
    items String,
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_jira.jira_fields — Jira field catalog. jira__task_field_metadata.sql
# classifies each field by cardinality/id-ness; jira-enrich reads the resulting
# staging.jira__task_field_metadata to shape value_ids/value_displays. Columns
# mirror what that staging model reads (schema_type/schema_items/schema_custom).
if ! ch_table_exists bronze_jira jira_fields; then
  echo "  Creating placeholder: bronze_jira.jira_fields"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_jira.jira_fields (
    unique_key String,
    source_id String,
    field_id String,
    name String,
    schema_type String,
    schema_items String,
    schema_custom String,
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_jira.jira_statuses — Jira status lookup (GET /rest/api/3/status).
# jira__task_statuses.sql (silver:class_task_statuses) reads it to map
# status_id -> status_category (done/in_progress/…) so gold detects closed
# tasks by statusCategory, not display name (issue #1541). Columns mirror the
# AddFields in connectors/task-tracking/jira/connector.yaml jira_statuses stream.
if ! ch_table_exists bronze_jira jira_statuses; then
  echo "  Creating placeholder: bronze_jira.jira_statuses"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_jira.jira_statuses (
    unique_key String,
    source_id String,
    status_id String,
    name String,
    category_id Nullable(Int32),
    category_name String,
    category_key String,
    collected_at String,
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_jira.jira_worklogs — Jira worklog rows. jira__task_worklogs.sql
# (tag silver:class_task_worklogs) reads these to build silver.class_task_worklogs,
# the numerator behind worklog_logging_accuracy. Columns mirror what that staging
# model reads (worklog_id, author_account_id, started, time_spent_seconds, …).
if ! ch_table_exists bronze_jira jira_worklogs; then
  echo "  Creating placeholder: bronze_jira.jira_worklogs"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_jira.jira_worklogs (
    unique_key String,
    source_id String,
    worklog_id String,
    id_readable String,
    author_account_id String,
    started String,
    time_spent_seconds Nullable(Int64),
    comment Nullable(String),
    collected_at String,
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_m365 -- needed by gold-views teams_person_daily, files_person_daily, comms_daily.
# Each table is checked and created independently so a partially-seeded
# state (e.g. teams_activity exists, onedrive_activity does not) gets the
# missing ones repaired on a re-run.
# NOTE: column set mirrors the real Airbyte M365 streams (verified against a
# live dev sync) so the collaboration dbt staging models — which read
# reportRefreshDate, the message/meeting counters, the ISO-8601 duration
# strings, etc. — can build from a seeded bronze in the e2e rig. The original
# minimum-viable placeholders only carried the handful of columns the legacy
# gold-view migrations type-check; the additional columns are additive and the
# real Airbyte sync still overwrites the schema in prod. ORDER BY unique_key
# matches the live engine (Airbyte's per-(user,date) natural key) so RMT
# collapses re-synced duplicates the same way prod does.
if ! ch_table_exists bronze_m365 teams_activity; then
  echo "  Creating placeholder: bronze_m365.teams_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_m365.teams_activity (
    tenant_id Nullable(String),
    source_id Nullable(String),
    unique_key Nullable(String),
    userPrincipalName String,
    reportRefreshDate Nullable(String),
    reportPeriod Nullable(String),
    lastActivityDate Nullable(String),
    teamChatMessageCount Nullable(Decimal(38, 9)),
    privateChatMessageCount Nullable(Decimal(38, 9)),
    postMessages Nullable(Decimal(38, 9)),
    replyMessages Nullable(Decimal(38, 9)),
    urgentMessages Nullable(Decimal(38, 9)),
    callCount Nullable(Decimal(38, 9)),
    meetingsAttendedCount Nullable(Decimal(38, 9)),
    meetingsOrganizedCount Nullable(Decimal(38, 9)),
    adHocMeetingsAttendedCount Nullable(Decimal(38, 9)),
    adHocMeetingsOrganizedCount Nullable(Decimal(38, 9)),
    scheduledOneTimeMeetingsAttendedCount Nullable(Decimal(38, 9)),
    scheduledOneTimeMeetingsOrganizedCount Nullable(Decimal(38, 9)),
    scheduledRecurringMeetingsAttendedCount Nullable(Decimal(38, 9)),
    scheduledRecurringMeetingsOrganizedCount Nullable(Decimal(38, 9)),
    audioDuration Nullable(String),
    videoDuration Nullable(String),
    screenShareDuration Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key
  SETTINGS allow_nullable_key = 1;
SQL
fi
if ! ch_table_exists bronze_m365 email_activity; then
  echo "  Creating placeholder: bronze_m365.email_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_m365.email_activity (
    tenant_id Nullable(String),
    source_id Nullable(String),
    unique_key Nullable(String),
    userPrincipalName String,
    displayName Nullable(String),
    reportRefreshDate Nullable(String),
    reportPeriod Nullable(String),
    lastActivityDate Nullable(String),
    assignedProducts Nullable(String),
    isDeleted Nullable(Bool),
    sendCount Nullable(Decimal(38, 9)),
    receiveCount Nullable(Decimal(38, 9)),
    readCount Nullable(Decimal(38, 9)),
    meetingCreatedCount Nullable(Decimal(38, 9)),
    meetingInteractedCount Nullable(Decimal(38, 9)),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key
  SETTINGS allow_nullable_key = 1;
SQL
fi
if ! ch_table_exists bronze_m365 onedrive_activity; then
  echo "  Creating placeholder: bronze_m365.onedrive_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_m365.onedrive_activity (
    tenant_id Nullable(String),
    source_id Nullable(String),
    unique_key Nullable(String),
    userPrincipalName String,
    reportRefreshDate Nullable(String),
    reportPeriod Nullable(String),
    lastActivityDate Nullable(String),
    viewedOrEditedFileCount Nullable(Decimal(38, 9)),
    syncedFileCount Nullable(Decimal(38, 9)),
    sharedInternallyFileCount Nullable(Decimal(38, 9)),
    sharedExternallyFileCount Nullable(Decimal(38, 9)),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key
  SETTINGS allow_nullable_key = 1;
SQL
fi
if ! ch_table_exists bronze_m365 sharepoint_activity; then
  echo "  Creating placeholder: bronze_m365.sharepoint_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_m365.sharepoint_activity (
    tenant_id Nullable(String),
    source_id Nullable(String),
    unique_key Nullable(String),
    userPrincipalName String,
    reportRefreshDate Nullable(String),
    reportPeriod Nullable(String),
    lastActivityDate Nullable(String),
    viewedOrEditedFileCount Nullable(Decimal(38, 9)),
    syncedFileCount Nullable(Decimal(38, 9)),
    sharedInternallyFileCount Nullable(Decimal(38, 9)),
    sharedExternallyFileCount Nullable(Decimal(38, 9)),
    visitedPageCount Nullable(Decimal(38, 9)),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key
  SETTINGS allow_nullable_key = 1;
SQL
fi

# bronze_zoom — needed by gold-views comms_daily, zoom_person_daily
#
# Key shape mirrors the PROMOTED table (`promote_bronze_to_rmt(order_by='unique_key')`
# in zoom__bronze_promoted.sql): ORDER BY unique_key, allow_nullable_key. The
# previous placeholder ordered by `email`, which silently collapsed every
# participant row of one person into a single row on RMT merge — deployed
# tables were unaffected (Airbyte + the promote macro produce the unique_key
# shape), but the e2e rig runs on the placeholder and could not seed more than
# one meeting per person.
if ! ch_table_exists bronze_zoom participants; then
  echo "  Creating placeholder: bronze_zoom.participants"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_zoom.participants (
    tenant_id String,
    source_id String,
    unique_key Nullable(String),
    email String,
    user_name Nullable(String),
    meeting_uuid String,
    participant_uuid String,
    join_time String,
    leave_time String,
    camera Nullable(String),
    share_desktop Nullable(Bool),
    share_application Nullable(Bool),
    share_whiteboard Nullable(Bool),
    video_connection_type Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key
  SETTINGS allow_nullable_key = 1;
SQL
else
  echo "  Reconciling placeholder schema: bronze_zoom.participants"
  run_ch <<'SQL'
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS tenant_id String;
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS source_id String;
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS unique_key Nullable(String);
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS user_name Nullable(String);
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS participant_uuid String;
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS camera Nullable(String);
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS share_desktop Nullable(Bool);
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS share_application Nullable(Bool);
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS share_whiteboard Nullable(Bool);
ALTER TABLE bronze_zoom.participants ADD COLUMN IF NOT EXISTS video_connection_type Nullable(String);
SQL
fi

# bronze_zoom.meetings — read by zoom__meeting_sessions (session stitching).
# The meeting-hours path computes duration from participants join/leave, so this
# table may be empty in tests; it needs to EXIST so the sessions model builds
# (the participant→session LEFT JOIN then falls back to meeting_uuid), and it is
# seeded by session-stitching e2e fixtures. Key shape mirrors the promoted table
# (ORDER BY unique_key), same as participants above.
if ! ch_table_exists bronze_zoom meetings; then
  echo "  Creating placeholder: bronze_zoom.meetings"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_zoom.meetings (
    tenant_id String,
    source_id String,
    unique_key Nullable(String),
    id Nullable(String),
    uuid String,
    start_time Nullable(String),
    end_time Nullable(String),
    has_video Nullable(Bool),
    has_screen_share Nullable(Bool),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key
  SETTINGS allow_nullable_key = 1;
SQL
else
  echo "  Reconciling placeholder schema: bronze_zoom.meetings"
  run_ch <<'SQL'
ALTER TABLE bronze_zoom.meetings ADD COLUMN IF NOT EXISTS unique_key Nullable(String);
SQL
fi

# bronze_cursor — needed by ic-kpis-honest-nulls, team-member-honest-nulls,
# bullet-views-honest-nulls, AND the cursor__ai_dev_usage dbt model. Full schema
# mirrors src/ingestion/connectors/ai/cursor/connector.yaml (stream
# cursor_daily_usage InlineSchemaLoader). Previously this placeholder carried
# only a 14-column subset on the assumption that Airbyte overwrites it on first
# sync — but the e2e rig (and any pre-sync env) has no Airbyte, so the dbt model
# (reads userId/date/tenant_id/source_id/unique_key/…) could not build. Keep this
# in lockstep with the connector's InlineSchemaLoader. `date` is epoch-millis.
if ! ch_table_exists bronze_cursor cursor_daily_usage; then
  echo "  Creating placeholder: bronze_cursor.cursor_daily_usage"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_cursor.cursor_daily_usage (
    tenant_id                String,
    source_id                String,
    unique_key               String,
    userId                   Nullable(String),
    email                    String,
    day                      Nullable(String),
    date                     Nullable(Float64),
    isActive                 Nullable(UInt8),
    chatRequests             Nullable(Float64),
    cmdkUsages               Nullable(Float64),
    composerRequests         Nullable(Float64),
    agentRequests            Nullable(Float64),
    bugbotUsages             Nullable(Float64),
    totalTabsShown           Nullable(Float64),
    totalTabsAccepted        Nullable(Float64),
    totalAccepts             Nullable(Float64),
    totalApplies             Nullable(Float64),
    totalRejects             Nullable(Float64),
    totalLinesAdded          Nullable(Float64),
    totalLinesDeleted        Nullable(Float64),
    acceptedLinesAdded       Nullable(Float64),
    acceptedLinesDeleted     Nullable(Float64),
    mostUsedModel            Nullable(String),
    tabMostUsedExtension     Nullable(String),
    applyMostUsedExtension   Nullable(String),
    clientVersion            Nullable(String),
    subscriptionIncludedReqs Nullable(Float64),
    usageBasedReqs           Nullable(Float64),
    apiKeyReqs               Nullable(Float64),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
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
    -- Remaining real BambooHR columns (aligned to the live Airbyte schema so
    -- the YAML rig can seed full rows). raw_data is JSON in prod; declared
    -- Nullable(String) here to avoid CH 24.8 experimental-JSON (always null in tests).
    city                  Nullable(String),
    country               Nullable(String),
    location              Nullable(String),
    hireDate              Nullable(String),
    originalHireDate      Nullable(String),
    terminationDate       Nullable(String),
    lastChanged           Nullable(String),
    employmentHistoryStatus Nullable(String),
    supervisorEId         Nullable(String),
    employeeNumber        Nullable(String),
    source_id             Nullable(String),
    tenant_id             Nullable(String),
    unique_key            Nullable(String),
    raw_data              Nullable(String),
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

# bronze_zulip_proxy.messages — per-(sender, bucket) aggregated chat counts
# from the Zulip proxy. zulip_proxy__collab_chat_activity dedups by `uniq`,
# joins users on sender_id = id, and sums `count` per (sender email, date).
# The real Airbyte connector overwrites this on first sync (full schema in
# src/ingestion/connectors/collaboration/zulip-proxy/connector.yaml).
if ! ch_table_exists bronze_zulip_proxy messages; then
  echo "  Creating placeholder: bronze_zulip_proxy.messages"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_zulip_proxy.messages (
    uniq                   String,
    sender_id              Nullable(Int64),
    count                  Nullable(Int64),
    created_at             String,
    tenant_id              Nullable(String),
    source_id              Nullable(String),
    unique_key             String,
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_claude_team.claude_team_code_metrics — per-user/day Claude Code usage
# (claude-team-proxy → /api/claude_code/metrics_aggs/users). Column set mirrors
# the connector InlineSchemaLoader (connectors/ai/claude-team/connector.yaml);
# claude_team__ai_dev_usage reads status/email/metric_date/total_*/prs_*.
if ! ch_table_exists bronze_claude_team claude_team_code_metrics; then
  echo "  Creating placeholder: bronze_claude_team.claude_team_code_metrics"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_claude_team.claude_team_code_metrics (
    tenant_id                  Nullable(String),
    source_id                  Nullable(String),
    unique_key                 String,
    collected_at               Nullable(String),
    data_source                Nullable(String),
    metric_date                Nullable(String),
    email                      Nullable(String),
    api_key_name               Nullable(String),
    status                     Nullable(String),
    avg_cost_per_day           Nullable(String),
    avg_lines_accepted_per_day Nullable(Float64),
    total_cost                 Nullable(String),
    total_lines_accepted       Nullable(Float64),
    total_sessions             Nullable(Float64),
    last_active                Nullable(String),
    prs_with_cc                Nullable(Float64),
    total_prs                  Nullable(Float64),
    prs_with_cc_percentage     Nullable(Float64),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_claude_team.claude_team_overage_spend — per-seat credit spend-vs-limit
# snapshot (claude.ai /overage_spend_limits). claude_team__ai_overage reads
# account_uuid/account_email/monthly_credit_limit/used_credits/etc. Identity
# columns are non-null `string` in the connector InlineSchemaLoader → String here.
if ! ch_table_exists bronze_claude_team claude_team_overage_spend; then
  echo "  Creating placeholder: bronze_claude_team.claude_team_overage_spend"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_claude_team.claude_team_overage_spend (
    tenant_id              String,
    source_id              String,
    unique_key             String,
    collected_at           String,
    data_source            String,
    account_uuid           String,
    account_email          String,
    account_name           String,
    seat_tier              Nullable(String),
    is_enabled             Nullable(Bool),
    monthly_credit_limit   Nullable(Float64),
    used_credits           Nullable(Float64),
    currency               Nullable(String),
    out_of_credits         Nullable(Bool),
    used_credits_basis     Nullable(String),
    limit_type             Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_zulip_proxy.users — Zulip user directory (full-refresh each sync).
# Joined by id = messages.sender_id to attach the sender email.
if ! ch_table_exists bronze_zulip_proxy users; then
  echo "  Creating placeholder: bronze_zulip_proxy.users"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_zulip_proxy.users (
    id                     Nullable(Int64),
    uuid                   Nullable(String),
    email                  Nullable(String),
    full_name              Nullable(String),
    role                   Nullable(Int64),
    is_active              Nullable(Bool),
    recipient_id           Nullable(Int64),
    tenant_id              Nullable(String),
    source_id              Nullable(String),
    unique_key             String,
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_claude_enterprise.claude_enterprise_users — per-user/day Claude Enterprise
# usage (admin Analytics API). claude_enterprise__ai_dev_usage (tool='claude_code')
# reads user_email/date/code_*; tool_use_accepted ← code_tool_accepted_count,
# tool_use_offered ← code_tool_accepted_count + code_tool_rejected_count. Mirrors
# the connector InlineSchemaLoader (unique_key + date are non-null String).
if ! ch_table_exists bronze_claude_enterprise claude_enterprise_users; then
  echo "  Creating placeholder: bronze_claude_enterprise.claude_enterprise_users"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_claude_enterprise.claude_enterprise_users (
    unique_key                   String,
    tenant_id                    Nullable(String),
    source_id                    Nullable(String),
    date                         String,
    user_id                      Nullable(String),
    user_email                   Nullable(String),
    chat_conversation_count      Nullable(Int64),
    chat_message_count           Nullable(Int64),
    chat_projects_created_count  Nullable(Int64),
    chat_projects_used_count     Nullable(Int64),
    chat_files_uploaded_count    Nullable(Int64),
    chat_artifacts_created_count Nullable(Int64),
    chat_thinking_message_count  Nullable(Int64),
    chat_skills_used_count       Nullable(Int64),
    chat_connectors_used_count   Nullable(Int64),
    code_commit_count            Nullable(Int64),
    code_pull_request_count      Nullable(Int64),
    code_lines_added             Nullable(Int64),
    code_lines_removed           Nullable(Int64),
    code_session_count           Nullable(Int64),
    code_tool_accepted_count     Nullable(Int64),
    code_tool_rejected_count     Nullable(Int64),
    web_search_count             Nullable(Int64),
    excel_session_count          Nullable(Int64),
    excel_message_count          Nullable(Int64),
    powerpoint_session_count     Nullable(Int64),
    powerpoint_message_count     Nullable(Int64),
    cowork_session_count         Nullable(Int64),
    cowork_message_count         Nullable(Int64),
    cowork_action_count          Nullable(Int64),
    cowork_dispatch_turn_count   Nullable(Int64),
    cowork_skills_used_count     Nullable(Int64),
    chat_metrics_json            Nullable(String),
    claude_code_metrics_json     Nullable(String),
    office_metrics_json          Nullable(String),
    cowork_metrics_json          Nullable(String),
    collected_at                 Nullable(String),
    data_source                  Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_chatgpt_team.chatgpt_team_codex_user_daily — per-user/day Codex usage
# pulled via the chatgpt-team-proxy from chatgpt.com's usage-leaderboard.
# chatgpt_team__ai_dev_usage reads email/date/n_threads/lines_added/credits/etc.
# Identity columns are non-null `string` in the connector catalog → String here.
if ! ch_table_exists bronze_chatgpt_team chatgpt_team_codex_user_daily; then
  echo "  Creating placeholder: bronze_chatgpt_team.chatgpt_team_codex_user_daily"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_codex_user_daily (
    tenant_id              String,
    source_id              String,
    unique_key             String,
    collected_at           String,
    data_source            String,
    date                   String,
    email                  String,
    user_id                Nullable(String),
    name                   Nullable(String),
    credits                Nullable(Float64),
    n_threads              Nullable(Float64),
    n_turns                Nullable(Float64),
    current_streak         Nullable(Float64),
    text_tokens            Nullable(Float64),
    lines_added            Nullable(Float64),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_outline.wiki_pages — Outline document snapshot (author/version/space). Feeds outline__wiki_pages → class_wiki_pages.
if ! ch_table_exists bronze_outline wiki_pages; then
  echo "  Creating placeholder: bronze_outline.wiki_pages"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_outline.wiki_pages (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    page_id                  Nullable(String),
    space_id                 Nullable(String),
    title                    Nullable(String),
    status                   Nullable(String),
    author_id                Nullable(String),
    author_email             Nullable(String),
    last_editor_id           Nullable(String),
    last_editor_email        Nullable(String),
    parent_page_id           Nullable(String),
    version_number           Nullable(Int64),
    created_at               Nullable(String),
    updated_at               Nullable(String),
    collected_at             Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_outline.wiki_spaces — collection metadata (LEFT JOIN in outline__wiki_pages).
if ! ch_table_exists bronze_outline wiki_spaces; then
  echo "  Creating placeholder: bronze_outline.wiki_spaces"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_outline.wiki_spaces (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    space_id                 Nullable(String),
    name                     Nullable(String),
    url                      Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_outline.wiki_users — user directory (LEFT JOIN for author_email in outline__wiki_pages).
if ! ch_table_exists bronze_outline wiki_users; then
  echo "  Creating placeholder: bronze_outline.wiki_users"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_outline.wiki_users (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    user_id                  Nullable(String),
    email                    Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_outline.wiki_comments — page comments. Feeds outline__wiki_engagement → class_wiki_engagement.
if ! ch_table_exists bronze_outline wiki_comments; then
  echo "  Creating placeholder: bronze_outline.wiki_comments"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_outline.wiki_comments (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    page_id                  Nullable(String),
    comment_id               Nullable(String),
    author_id                Nullable(String),
    created_at               Nullable(String),
    resolution_status        Nullable(String),
    parent_comment_id        Nullable(String),
    anchor_text              Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_confluence.wiki_pages — page snapshot (author/version/space). Feeds confluence__wiki_pages → class_wiki_pages.
if ! ch_table_exists bronze_confluence wiki_pages; then
  echo "  Creating placeholder: bronze_confluence.wiki_pages"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_pages (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    page_id                  Nullable(String),
    space_id                 Nullable(String),
    title                    Nullable(String),
    status                   Nullable(String),
    author_id                Nullable(String),
    last_editor_id           Nullable(String),
    parent_page_id           Nullable(String),
    version_number           Nullable(Int64),
    created_at               Nullable(String),
    updated_at               Nullable(String),
    collected_at             Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_confluence.wiki_spaces — space metadata (LEFT JOIN in confluence__wiki_pages).
if ! ch_table_exists bronze_confluence wiki_spaces; then
  echo "  Creating placeholder: bronze_confluence.wiki_spaces"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_spaces (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    space_id                 Nullable(String),
    name                     Nullable(String),
    url                      Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_outline.wiki_page_versions — revision history. Feeds outline__wiki_activity → class_wiki_activity.
if ! ch_table_exists bronze_outline wiki_page_versions; then
  echo "  Creating placeholder: bronze_outline.wiki_page_versions"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_outline.wiki_page_versions (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    page_id                  Nullable(String),
    revision_id              Nullable(String),
    author_id                Nullable(String),
    author_email             Nullable(String),
    created_at               Nullable(String),
    message                  Nullable(String),
    collected_at             Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_confluence.wiki_page_versions — version history. Feeds confluence__wiki_activity → class_wiki_activity.
if ! ch_table_exists bronze_confluence wiki_page_versions; then
  echo "  Creating placeholder: bronze_confluence.wiki_page_versions"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_page_versions (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    page_id                  Nullable(String),
    version_number           Nullable(Int64),
    author_id                Nullable(String),
    created_at               Nullable(String),
    message                  Nullable(String),
    minor_edit               Nullable(Bool),
    collected_at             Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_confluence.wiki_footer_comments — footer top-level comments. Feeds confluence__wiki_engagement → class_wiki_engagement.
if ! ch_table_exists bronze_confluence wiki_footer_comments; then
  echo "  Creating placeholder: bronze_confluence.wiki_footer_comments"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_footer_comments (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    page_id                  Nullable(String),
    comment_id               Nullable(String),
    parent_comment_id        Nullable(String),
    author_id                Nullable(String),
    created_at               Nullable(String),
    resolution_status        Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_confluence.wiki_footer_comment_replies — footer replies comments. Feeds confluence__wiki_engagement → class_wiki_engagement.
if ! ch_table_exists bronze_confluence wiki_footer_comment_replies; then
  echo "  Creating placeholder: bronze_confluence.wiki_footer_comment_replies"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_footer_comment_replies (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    page_id                  Nullable(String),
    comment_id               Nullable(String),
    parent_comment_id        Nullable(String),
    author_id                Nullable(String),
    created_at               Nullable(String),
    resolution_status        Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_confluence.wiki_inline_comments — inline top-level comments. Feeds confluence__wiki_engagement → class_wiki_engagement.
if ! ch_table_exists bronze_confluence wiki_inline_comments; then
  echo "  Creating placeholder: bronze_confluence.wiki_inline_comments"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_inline_comments (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    page_id                  Nullable(String),
    comment_id               Nullable(String),
    parent_comment_id        Nullable(String),
    author_id                Nullable(String),
    created_at               Nullable(String),
    resolution_status        Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_confluence.wiki_inline_comment_replies — inline replies comments. Feeds confluence__wiki_engagement → class_wiki_engagement.
if ! ch_table_exists bronze_confluence wiki_inline_comment_replies; then
  echo "  Creating placeholder: bronze_confluence.wiki_inline_comment_replies"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_inline_comment_replies (
    unique_key               String,
    tenant_id                Nullable(String),
    source_id                Nullable(String),
    page_id                  Nullable(String),
    comment_id               Nullable(String),
    parent_comment_id        Nullable(String),
    author_id                Nullable(String),
    created_at               Nullable(String),
    resolution_status        Nullable(String),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

# bronze_chatgpt_team.chatgpt_team_chat_activity — per-user/day chat usage pulled
# via the chatgpt-team-proxy from chatgpt.com's analytics user_list endpoint.
# chatgpt_team__ai_assistant_usage reads email/date/messages/*_messages/etc.
if ! ch_table_exists bronze_chatgpt_team chatgpt_team_chat_activity; then
  echo "  Creating placeholder: bronze_chatgpt_team.chatgpt_team_chat_activity"
  run_ch <<'SQL'
CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_chat_activity (
    tenant_id              String,
    source_id              String,
    unique_key             String,
    collected_at           String,
    data_source            String,
    date                   String,
    email                  String,
    name                   Nullable(String),
    seat_type              Nullable(String),
    messages               Nullable(Float64),
    gpt_messages           Nullable(Float64),
    tool_messages          Nullable(Float64),
    connector_messages     Nullable(Float64),
    project_messages       Nullable(Float64),
    credits_used           Nullable(Float64),
    _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
    _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
    _airbyte_meta          String        DEFAULT '{}',
    _airbyte_generation_id UInt32        DEFAULT 0
) ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
fi

echo "=== Placeholders: done ==="
