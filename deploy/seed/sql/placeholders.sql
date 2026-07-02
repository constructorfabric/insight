-- `insight` is created by /docker-entrypoint-initdb.d/00-init.sql on
-- first CH start, but that script doesn't re-run if the operator
-- wipes the db (e.g., DROP DATABASE insight SYNC for a re-bootstrap).
-- Asserting it here makes the seed-sample idempotent across re-runs.
CREATE DATABASE IF NOT EXISTS insight;
CREATE DATABASE IF NOT EXISTS silver;
CREATE DATABASE IF NOT EXISTS bronze_jira;
CREATE DATABASE IF NOT EXISTS bronze_m365;
CREATE DATABASE IF NOT EXISTS bronze_zoom;
CREATE DATABASE IF NOT EXISTS bronze_cursor;
CREATE DATABASE IF NOT EXISTS bronze_slack;
CREATE DATABASE IF NOT EXISTS bronze_bamboohr;
CREATE DATABASE IF NOT EXISTS bronze_bitbucket_cloud;
CREATE TABLE IF NOT EXISTS silver.class_comms_events (
    user_email    String,
    activity_date Date,
    emails_sent   Float64,
    source        String,
    _version      UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (user_email, activity_date) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
    lines_removed        Nullable(Float64),
    total_lines_added    Nullable(Float64),
    total_lines_removed  Nullable(Float64),
    accepted_lines_added Nullable(Float64),
    spec_lines           Nullable(Float64),
    session_count        Nullable(Float64),
    total_chat_messages  Nullable(Float64),
    -- Five columns below: the upstream placeholder script forgot to
    -- mirror them after migration 20260601 ai-claude-team-metrics added
    -- them. Without them every ai_bullet_rows view recreation fails
    -- with UNKNOWN_IDENTIFIER. The real silver model has them.
    cost_cents           Nullable(Float64),
    commits_count        Nullable(Float64),
    pull_requests_count  Nullable(Float64),
    prs_with_cc_count    Nullable(Float64),
    prs_total_count      Nullable(Float64),
    -- source / source_id: dimensions the ai_personal_gold views select +
    -- GROUP BY (migration 20260623_ai_personal_gold_views, from #1514).
    -- Same placeholder-mirror gap as the block above — the upstream
    -- placeholder script wasn't updated when that migration landed. The
    -- real silver model has them.
    source               String DEFAULT '',
    source_id            String DEFAULT '',
    _version             UInt64
-- `tool` MUST be in the sort key — without it, ReplacingMergeTree
-- collapses cursor and claude_code rows for the same (email, day)
-- into one, suppressing whichever was inserted first.
) ENGINE = ReplacingMergeTree(_version) ORDER BY (email, day, tool) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
-- Added because migration 20260618000000 ai-claude-team-overage-gold creates
-- the cc_overage gold view from silver.class_ai_overage; the upstream
-- placeholder script never mirrored this table, so the view recreation fails
-- with UNKNOWN_TABLE. Columns track the subset the gold views consume from
-- the real class_ai_overage silver model (email / source / collected_at /
-- overage_cents); the full model carries the 19-column union contract.
CREATE TABLE IF NOT EXISTS silver.class_ai_overage (
    insight_tenant_id  String,
    source_id          String,
    unique_key         String,
    email              String,
    source             String,
    currency           Nullable(String),
    credit_limit_cents Nullable(UInt32),
    used_amount_cents  Nullable(UInt32),
    overage_cents      Nullable(UInt32),
    collected_at       Nullable(DateTime64(3)),
    _version           UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
-- Added because migration 20260620000000 wiki-bullet-rows creates the
-- insight.wiki_bullet_rows gold view from these two STABLE wiki Silver
-- classes; the upstream placeholder script never mirrored them, so the view
-- creation fails with UNKNOWN_TABLE. Columns track the subset the gold view
-- consumes (the real models carry the full union-by-tag contract). NB: the
-- wiki classes key on `tenant_id`, not `insight_tenant_id` — the view joins
-- engagement→pages on `e.tenant_id = pg.tenant_id`.
CREATE TABLE IF NOT EXISTS silver.class_wiki_pages (
    tenant_id      String,
    source_id      String,
    unique_key     String,
    page_id        String,
    author_id      Nullable(String),
    author_email   Nullable(String),
    version_count  UInt32,
    created_at     Nullable(DateTime64(3)),
    _version       UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
CREATE TABLE IF NOT EXISTS silver.class_wiki_engagement (
    tenant_id      String,
    source_id      String,
    unique_key     String,
    page_id        String,
    day            Date,
    total_comments UInt32,
    _version       UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
CREATE TABLE IF NOT EXISTS silver.class_people (
    unique_key      String,
    email           Nullable(String),
    department_name Nullable(String),
    _version        UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
CREATE TABLE IF NOT EXISTS silver.class_crm_users (
    unique_key String,
    user_id    String,
    hs_user_id Nullable(String),
    email      Nullable(String),
    _version   UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
CREATE TABLE IF NOT EXISTS silver.class_crm_activities (
    unique_key         String,
    timestamp          Nullable(DateTime64(3)),
    activity_type      String,
    owner_id           Nullable(String),
    created_by_user_id Nullable(String),
    _version           UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
CREATE TABLE IF NOT EXISTS silver.class_git_commits (
    insight_tenant_id String,
    commit_hash       String,
    project_key       String,
    repo_slug         String  DEFAULT '',
    tenant_id         String,
    author_email      String,
    date              Date,
    is_merge_commit   UInt8,
    file_path         String  DEFAULT '',
    -- Non-Nullable so `toFloat64(sum(c.lines_added + c.lines_removed))`
    -- in the git_bullet_rows view stays Float64 (the view's structure
    -- declares metric_value as Float64, not Nullable(Float64)).
    lines_added       Float64 DEFAULT 0,
    lines_removed     Float64 DEFAULT 0,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (commit_hash) COMMENT 'INSIGHT_PLACEHOLDER_v1';
CREATE TABLE IF NOT EXISTS silver.class_git_pull_requests (
    insight_tenant_id String,
    pr_id             String,
    author_email      String,
    author_name       String,
    state             String,
    created_on        DateTime,
    merged_on         Nullable(DateTime),
    closed_on         Nullable(DateTime),
    -- Non-Nullable on purpose. The git_bullet_rows view's UNION branch
    -- for `pr_size` declares the column as Float64 (non-null); a
    -- Nullable placeholder makes the UNION type Nullable, which then
    -- collides with the view structure under join_use_nulls=1.
    lines_added       Float64 DEFAULT 0,
    lines_removed     Float64 DEFAULT 0,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (pr_id) COMMENT 'INSIGHT_PLACEHOLDER_v1';
CREATE TABLE IF NOT EXISTS silver.class_git_file_changes (
    insight_tenant_id String,
    commit_hash       String,
    project_key       String,
    repo_slug         String DEFAULT '',
    tenant_id         String,
    file_path         String,
    lines_added       Int64,
    lines_removed     Int64,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (commit_hash, file_path) COMMENT 'INSIGHT_PLACEHOLDER_v1';
CREATE TABLE IF NOT EXISTS silver.class_task_daily (
    insight_tenant_id String,
    person_id         String,
    metric_date       Date,
    tasks_closed      Float64,
    bugs_fixed        Float64,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_id, metric_date) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
CREATE TABLE IF NOT EXISTS silver.class_task_users (
    insight_tenant_id String,
    insight_source_id String,
    user_id           String,
    email             Nullable(String),
    unique_key        String,
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
CREATE TABLE IF NOT EXISTS silver.class_support_activity (
    insight_tenant_id String,
    person_key        String,
    date              Date,
    data_source       String,
    updates           Nullable(Float64),
    public_comments   Nullable(Float64),
    private_comments  Nullable(Float64),
    solved            Nullable(Float64),
    csat_good         Nullable(Float64),
    csat_total        Nullable(Float64),
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_key, date, data_source) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
CREATE TABLE IF NOT EXISTS silver.mtr_git_person_weekly (
    insight_tenant_id String,
    person_key        String,
    week              Date,
    commits           UInt64,
    lines_added       Int64,
    lines_removed     Int64,
    prs_merged        Float64,
    -- code_loc / spec_lines added because insight.ic_chart_loc, the IC-KPIs
    -- git-fanout view, and other views reference them; upstream placeholder
    -- script omits both.
    code_loc          Float64,
    spec_lines        Nullable(Float64),
    _version          UInt64
) ENGINE = ReplacingMergeTree(_version) ORDER BY (person_key, week) COMMENT 'INSIGHT_PLACEHOLDER_v1';
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
CREATE DATABASE IF NOT EXISTS bronze_m365;
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
