-- Gold views for Insight dashboards (squashed migration).
-- Source: clickhouse-insight-alexey.sql (2026-04-22)
-- Replaces: 20260417000000, 20260417100000, 20260421000000
--
-- 29 objects: 1 MergeTree table + 28 views
-- Dependencies: bronze_bamboohr, bronze_jira, bronze_m365, bronze_zoom,
--               bronze_bitbucket_cloud, bronze_cursor, bronze_slack,
--               silver.class_focus_metrics, silver.class_collab_*,
--               staging.m365__collab_email_activity
--
-- Usage: clickhouse-client --multiquery < 20260422000000_gold-views.sql

-- Drop in reverse dependency order
DROP VIEW IF EXISTS insight.ic_timeoff;
DROP VIEW IF EXISTS insight.ic_drill;
DROP VIEW IF EXISTS insight.ic_chart_loc;
DROP VIEW IF EXISTS insight.ic_chart_delivery;
DROP VIEW IF EXISTS insight.ic_kpis;
DROP VIEW IF EXISTS insight.team_member;
DROP VIEW IF EXISTS insight.exec_summary;
DROP VIEW IF EXISTS insight.ai_company_stats;
DROP VIEW IF EXISTS insight.code_quality_company_stats;
DROP VIEW IF EXISTS insight.task_delivery_company_stats;
DROP VIEW IF EXISTS insight.collab_company_stats;
DROP VIEW IF EXISTS insight.ai_person_period;
DROP VIEW IF EXISTS insight.code_quality_person_period;
DROP VIEW IF EXISTS insight.task_delivery_person_period;
DROP VIEW IF EXISTS insight.collab_person_period;
DROP VIEW IF EXISTS insight.git_bullet_rows;
DROP VIEW IF EXISTS insight.ai_bullet_rows;
DROP VIEW IF EXISTS insight.code_quality_bullet_rows;
DROP VIEW IF EXISTS insight.task_delivery_bullet_rows;
DROP VIEW IF EXISTS insight.collab_bullet_rows;
DROP VIEW IF EXISTS insight.zoom_person_daily;
DROP VIEW IF EXISTS insight.teams_person_daily;
DROP VIEW IF EXISTS insight.files_person_daily;
DROP VIEW IF EXISTS insight.email_daily;
DROP VIEW IF EXISTS insight.comms_daily;
DROP VIEW IF EXISTS insight.commits_daily;
DROP TABLE IF EXISTS insight.jira_closed_tasks;
DROP VIEW IF EXISTS insight.jira_person_daily;
DROP VIEW IF EXISTS insight.people;

-- =====================================================================
-- people
-- =====================================================================
CREATE VIEW insight.people
(
    `person_id` Nullable(String),
    `display_name` Nullable(String),
    `org_unit_id` Nullable(String),
    `org_unit_name` Nullable(String),
    `seniority` String,
    `job_title` Nullable(String),
    `status` Nullable(String)
)
AS SELECT
    person_id,
    argMax(displayName, _airbyte_extracted_at) AS display_name,
    argMax(department, _airbyte_extracted_at) AS org_unit_id,
    argMax(department, _airbyte_extracted_at) AS org_unit_name,
    argMax(multiIf((jobTitle ILIKE '%senior%') OR (jobTitle ILIKE '%lead%') OR (jobTitle ILIKE '%principal%') OR (jobTitle ILIKE '%architect%') OR (jobTitle ILIKE '%director%') OR (jobTitle ILIKE '%head%'), 'Senior', (jobTitle ILIKE '%junior%') OR (jobTitle ILIKE '%intern%') OR (jobTitle ILIKE '%trainee%'), 'Junior', 'Mid'), _airbyte_extracted_at) AS seniority,
    argMax(jobTitle, _airbyte_extracted_at) AS job_title,
    argMax(status, _airbyte_extracted_at) AS status
FROM bronze_bamboohr.employees
WHERE (workEmail IS NOT NULL) AND (workEmail != '')
GROUP BY lower(workEmail) AS person_id
;

-- =====================================================================
-- jira_person_daily
-- =====================================================================
CREATE VIEW insight.jira_person_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `issue_type` Nullable(String),
    `status_name` Nullable(String),
    `resolution` Nullable(String),
    `due_date` Nullable(String),
    `time_estimate_sec` Nullable(Float64),
    `time_spent_sec` Nullable(Float64),
    `id_readable` Nullable(String)
)
AS SELECT
    lower(JSONExtractString(custom_fields_json, 'assignee', 'emailAddress')) AS person_id,
    toDate(parseDateTimeBestEffort(updated)) AS metric_date,
    JSONExtractString(custom_fields_json, 'issuetype', 'name') AS issue_type,
    JSONExtractString(custom_fields_json, 'status', 'name') AS status_name,
    JSONExtractString(custom_fields_json, 'resolution', 'name') AS resolution,
    due_date,
    JSONExtractFloat(custom_fields_json, 'timeoriginalestimate') AS time_estimate_sec,
    JSONExtractFloat(custom_fields_json, 'timespent') AS time_spent_sec,
    id_readable
FROM bronze_jira.jira_issue
WHERE person_id != ''
;

-- =====================================================================
-- jira_closed_tasks
-- =====================================================================
CREATE TABLE insight.jira_closed_tasks
(
    `person_id` String,
    `metric_date` Date,
    `tasks_closed` UInt64,
    `bugs_fixed` UInt64,
    `on_time_count` UInt64,
    `has_due_date_count` UInt64,
    `avg_time_spent` Nullable(Float64),
    `avg_time_estimate` Nullable(Float64)
)
ENGINE = MergeTree
ORDER BY (person_id, metric_date)
SETTINGS index_granularity = 8192
;

-- Populate from bronze_jira (JSON extraction too slow for a VIEW)
INSERT INTO insight.jira_closed_tasks
SELECT
    lower(JSONExtractString(custom_fields_json, 'assignee', 'emailAddress')) AS person_id,
    toDate(parseDateTimeBestEffort(updated)) AS metric_date,
    count() AS tasks_closed,
    countIf(JSONExtractString(custom_fields_json, 'issuetype', 'name') = 'Bug') AS bugs_fixed,
    countIf(due_date IS NOT NULL AND due_date != ''
            AND toDate(parseDateTimeBestEffort(updated)) <= toDate(due_date)) AS on_time_count,
    countIf(due_date IS NOT NULL AND due_date != '') AS has_due_date_count,
    avgIf(JSONExtractFloat(custom_fields_json, 'timespent'),
          JSONExtractFloat(custom_fields_json, 'timeoriginalestimate') > 0) AS avg_time_spent,
    avgIf(JSONExtractFloat(custom_fields_json, 'timeoriginalestimate'),
          JSONExtractFloat(custom_fields_json, 'timeoriginalestimate') > 0) AS avg_time_estimate
FROM bronze_jira.jira_issue
WHERE lower(JSONExtractString(custom_fields_json, 'assignee', 'emailAddress')) != ''
  AND JSONExtractString(custom_fields_json, 'status', 'name') IN ('Closed', 'Resolved', 'Verified')
GROUP BY person_id, metric_date;

-- =====================================================================
-- commits_daily
-- =====================================================================
CREATE VIEW insight.commits_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `commits` UInt64
)
AS SELECT
    lower(author_email) AS person_id,
    toDate(parseDateTimeBestEffortOrNull(assumeNotNull(date))) AS metric_date,
    count() AS commits
FROM bronze_bitbucket_cloud.commits
WHERE (author_email IS NOT NULL) AND (author_email LIKE '%@virtuozzo.com') AND (date IS NOT NULL)
GROUP BY
    person_id,
    metric_date
;

-- =====================================================================
-- comms_daily
-- =====================================================================
CREATE VIEW insight.comms_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(String),
    `emails_sent` Nullable(Float64),
    `zoom_calls` Float64,
    `meeting_hours` Nullable(Float64),
    `teams_messages` Float64,
    `teams_meetings` Float64,
    `files_shared` Float64
)
AS SELECT
    person_id,
    toString(metric_date) AS metric_date,
    sum(emails_sent) AS emails_sent,
    sum(zoom_calls) AS zoom_calls,
    sum(meeting_hours) AS meeting_hours,
    sum(teams_messages) AS teams_messages,
    sum(teams_meetings) AS teams_meetings,
    sum(files_shared) AS files_shared
FROM
(
    SELECT
        lower(person_key) AS person_id,
        date AS metric_date,
        toFloat64(sent_count) AS emails_sent,
        toFloat64(0) AS zoom_calls,
        toFloat64(0) AS meeting_hours,
        toFloat64(0) AS teams_messages,
        toFloat64(0) AS teams_meetings,
        toFloat64(0) AS files_shared
    FROM staging.m365__collab_email_activity
    UNION ALL
    SELECT
        lower(p.email) AS person_id,
        toDate(parseDateTimeBestEffort(p.join_time)) AS metric_date,
        toFloat64(0) AS emails_sent,
        toFloat64(1) AS zoom_calls,
        dateDiff('second', parseDateTimeBestEffort(p.join_time), parseDateTimeBestEffort(p.leave_time)) / 3600. AS meeting_hours,
        toFloat64(0) AS teams_messages,
        toFloat64(0) AS teams_meetings,
        toFloat64(0) AS files_shared
    FROM bronze_zoom.participants AS p
    WHERE (p.email IS NOT NULL) AND (p.email != '')
    UNION ALL
    SELECT
        lower(userPrincipalName) AS person_id,
        toDate(lastActivityDate) AS metric_date,
        toFloat64(0) AS emails_sent,
        toFloat64(0) AS zoom_calls,
        toFloat64(0) AS meeting_hours,
        toFloat64(ifNull(teamChatMessageCount, 0)) + toFloat64(ifNull(privateChatMessageCount, 0)) AS teams_messages,
        toFloat64(ifNull(meetingsAttendedCount, 0)) AS teams_meetings,
        toFloat64(0) AS files_shared
    FROM bronze_m365.teams_activity
    WHERE (userPrincipalName IS NOT NULL) AND (userPrincipalName != '') AND (userPrincipalName != '(Unknown)')
    UNION ALL
    SELECT
        lower(userPrincipalName) AS person_id,
        toDate(lastActivityDate) AS metric_date,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(ifNull(sharedInternallyFileCount, 0)) + toFloat64(ifNull(sharedExternallyFileCount, 0))
    FROM bronze_m365.onedrive_activity
    WHERE (userPrincipalName IS NOT NULL) AND (userPrincipalName != '')
    UNION ALL
    SELECT
        lower(userPrincipalName) AS person_id,
        toDate(lastActivityDate) AS metric_date,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(ifNull(sharedInternallyFileCount, 0)) + toFloat64(ifNull(sharedExternallyFileCount, 0))
    FROM bronze_m365.sharepoint_activity
    WHERE (userPrincipalName IS NOT NULL) AND (userPrincipalName != '')
) AS sub
GROUP BY
    person_id,
    metric_date
;

-- =====================================================================
-- email_daily
-- =====================================================================
CREATE VIEW insight.email_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `user_email` Nullable(String),
    `emails_sent` Nullable(Decimal(38, 9)),
    `source` String
)
AS SELECT
    lower(person_key) AS person_id,
    date AS metric_date,
    lower(person_key) AS user_email,
    sent_count AS emails_sent,
    data_source AS source
FROM staging.m365__collab_email_activity
;

-- =====================================================================
-- files_person_daily
-- =====================================================================
CREATE VIEW insight.files_person_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `files_shared` Float64
)
AS SELECT
    person_id,
    metric_date,
    sum(shared_internally) + sum(shared_externally) AS files_shared
FROM
(
    SELECT
        lower(userPrincipalName) AS person_id,
        toDate(lastActivityDate) AS metric_date,
        toFloat64(ifNull(sharedInternallyFileCount, 0)) AS shared_internally,
        toFloat64(ifNull(sharedExternallyFileCount, 0)) AS shared_externally
    FROM bronze_m365.onedrive_activity
    WHERE (userPrincipalName IS NOT NULL) AND (userPrincipalName != '')
    UNION ALL
    SELECT
        lower(userPrincipalName) AS person_id,
        toDate(lastActivityDate) AS metric_date,
        toFloat64(ifNull(sharedInternallyFileCount, 0)) AS shared_internally,
        toFloat64(ifNull(sharedExternallyFileCount, 0)) AS shared_externally
    FROM bronze_m365.sharepoint_activity
    WHERE (userPrincipalName IS NOT NULL) AND (userPrincipalName != '')
) AS sub
GROUP BY
    person_id,
    metric_date
;

-- =====================================================================
-- teams_person_daily
-- =====================================================================
CREATE VIEW insight.teams_person_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `teams_messages` Float64,
    `teams_meetings` Float64,
    `teams_calls` Float64
)
AS SELECT
    lower(userPrincipalName) AS person_id,
    toDate(lastActivityDate) AS metric_date,
    toFloat64(ifNull(teamChatMessageCount, 0)) + toFloat64(ifNull(privateChatMessageCount, 0)) AS teams_messages,
    toFloat64(ifNull(meetingsAttendedCount, 0)) AS teams_meetings,
    toFloat64(ifNull(callCount, 0)) AS teams_calls
FROM bronze_m365.teams_activity
WHERE (userPrincipalName IS NOT NULL) AND (userPrincipalName != '') AND (userPrincipalName != '(Unknown)')
;

-- =====================================================================
-- zoom_person_daily
-- =====================================================================
CREATE VIEW insight.zoom_person_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `user_email` Nullable(String),
    `zoom_calls` UInt64,
    `meeting_hours` Nullable(Float64)
)
AS SELECT
    lower(p.email) AS person_id,
    toDate(parseDateTimeBestEffort(p.join_time)) AS metric_date,
    lower(p.email) AS user_email,
    countDistinct(p.meeting_uuid) AS zoom_calls,
    sum(dateDiff('second', parseDateTimeBestEffort(p.join_time), parseDateTimeBestEffort(p.leave_time))) / 3600. AS meeting_hours
FROM bronze_zoom.participants AS p
WHERE (p.email IS NOT NULL) AND (p.email != '')
GROUP BY
    lower(p.email),
    toDate(parseDateTimeBestEffort(p.join_time))
;

-- =====================================================================
-- collab_bullet_rows
-- =====================================================================
CREATE VIEW insight.collab_bullet_rows
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(String),
    `metric_key` String,
    `metric_value` Nullable(Float64)
)
AS SELECT
    lower(e.email) AS person_id,
    p.org_unit_id,
    toString(e.date) AS metric_date,
    'm365_emails_sent' AS metric_key,
    toFloat64(ifNull(e.sent_count, 0)) AS metric_value
FROM silver.class_collab_email_activity AS e
LEFT JOIN insight.people AS p ON lower(e.email) = p.person_id
UNION ALL
SELECT
    lower(m.email) AS person_id,
    p.org_unit_id,
    toString(m.date) AS metric_date,
    'zoom_calls',
    toFloat64(ifNull(m.meetings_attended, 0))
FROM silver.class_collab_meeting_activity AS m
LEFT JOIN insight.people AS p ON lower(m.email) = p.person_id
WHERE m.data_source = 'insight_zoom'
UNION ALL
SELECT
    f.email AS person_id,
    p.org_unit_id,
    toString(f.day) AS metric_date,
    'meeting_hours',
    least(toFloat64(ifNull(f.meeting_hours, 0)), f.working_hours_per_day)
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id
UNION ALL
SELECT
    lower(c.email) AS person_id,
    p.org_unit_id,
    toString(c.date) AS metric_date,
    'm365_teams_messages',
    toFloat64(c.total_chat_messages)
FROM silver.class_collab_chat_activity AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(d.email) AS person_id,
    p.org_unit_id,
    toString(d.date) AS metric_date,
    'm365_files_shared',
    toFloat64(ifNull(d.shared_internally_count, 0)) + toFloat64(ifNull(d.shared_externally_count, 0))
FROM silver.class_collab_document_activity AS d
LEFT JOIN insight.people AS p ON lower(d.email) = p.person_id
UNION ALL
SELECT
    f.email AS person_id,
    p.org_unit_id,
    toString(f.day) AS metric_date,
    'meeting_free',
    if(ifNull(f.meeting_hours, 0) = 0, toFloat64(1), toFloat64(0))
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id
UNION ALL
SELECT
    lower(s.email_address) AS person_id,
    p.org_unit_id,
    s.date AS metric_date,
    'slack_thread_participation',
    toFloat64(ifNull(s.channel_messages_posted_count, 0))
FROM bronze_slack.users_details AS s
LEFT JOIN insight.people AS p ON lower(s.email_address) = p.person_id
UNION ALL
SELECT
    lower(s.email_address) AS person_id,
    p.org_unit_id,
    s.date AS metric_date,
    'slack_message_engagement',
    toFloat64(ifNull(s.messages_posted_count, 0))
FROM bronze_slack.users_details AS s
LEFT JOIN insight.people AS p ON lower(s.email_address) = p.person_id
UNION ALL
SELECT
    lower(s.email_address) AS person_id,
    p.org_unit_id,
    s.date AS metric_date,
    'slack_dm_ratio',
    if(ifNull(s.messages_posted_count, 0) > 0, round(((toFloat64(ifNull(s.messages_posted_count, 0)) - toFloat64(ifNull(s.channel_messages_posted_count, 0))) / toFloat64(s.messages_posted_count)) * 100, 1), toFloat64(0))
FROM bronze_slack.users_details AS s
LEFT JOIN insight.people AS p ON lower(s.email_address) = p.person_id
;

-- =====================================================================
-- task_delivery_bullet_rows
-- =====================================================================
CREATE VIEW insight.task_delivery_bullet_rows
(
    `person_id` String,
    `org_unit_id` Nullable(String),
    `metric_date` String,
    `metric_key` String,
    `metric_value` Nullable(Float64)
)
AS SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date) AS metric_date,
    'tasks_completed' AS metric_key,
    toFloat64(j.tasks_closed) AS metric_value
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'task_dev_time',
    round(ifNull(j.avg_time_spent, 0) / 3600., 1)
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'task_reopen_rate',
    toFloat64(0)
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'due_date_compliance',
    if(j.has_due_date_count > 0, round((toFloat64(j.on_time_count) / toFloat64(j.has_due_date_count)) * 100, 1), toFloat64(0))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'estimation_accuracy',
    if((ifNull(j.avg_time_spent, 0) > 0) AND (j.avg_time_estimate IS NOT NULL), round((j.avg_time_estimate / j.avg_time_spent) * 100, 1), toFloat64(0))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
;

-- =====================================================================
-- code_quality_bullet_rows
-- =====================================================================
CREATE VIEW insight.code_quality_bullet_rows
(
    `person_id` String,
    `org_unit_id` Nullable(String),
    `metric_date` String,
    `metric_key` String,
    `metric_value` Float64
)
AS SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date) AS metric_date,
    'bugs_fixed' AS metric_key,
    toFloat64(j.bugs_fixed) AS metric_value
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'prs_per_dev',
    toFloat64(0)
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'pr_cycle_time',
    toFloat64(0)
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'build_success',
    toFloat64(0)
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
;

-- =====================================================================
-- ai_bullet_rows
-- =====================================================================
CREATE VIEW insight.ai_bullet_rows
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(String),
    `metric_key` String,
    `metric_value` Nullable(Float64)
)
AS SELECT
    lower(c.email) AS person_id,
    p.org_unit_id,
    c.day AS metric_date,
    'active_ai_members' AS metric_key,
    if(c.isActive = true, toFloat64(1), toFloat64(0)) AS metric_value
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cursor_active',
    if(c.isActive = true, toFloat64(1), toFloat64(0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cc_active',
    toFloat64(0)
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'codex_active',
    toFloat64(0)
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'team_ai_loc',
    toFloat64(ifNull(c.acceptedLinesAdded, 0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cursor_acceptance',
    if(toFloat64(ifNull(c.totalTabsShown, 0)) > 0, round((toFloat64(ifNull(c.totalTabsAccepted, 0)) / toFloat64(c.totalTabsShown)) * 100, 1), toFloat64(0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cc_tool_acceptance',
    toFloat64(0)
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cursor_completions',
    toFloat64(ifNull(c.totalTabsShown, 0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cursor_agents',
    toFloat64(ifNull(c.agentRequests, 0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cursor_lines',
    toFloat64(ifNull(c.acceptedLinesAdded, 0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cc_sessions',
    toFloat64(0)
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cc_tool_accept',
    toFloat64(0)
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'cc_lines',
    toFloat64(0)
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'ai_loc_share2',
    if(toFloat64(ifNull(c.totalLinesAdded, 0)) > 0, round((toFloat64(ifNull(c.acceptedLinesAdded, 0)) / toFloat64(c.totalLinesAdded)) * 100, 1), toFloat64(0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'claude_web',
    toFloat64(0)
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email),
    p.org_unit_id,
    c.day,
    'chatgpt',
    toFloat64(0)
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
;

-- =====================================================================
-- git_bullet_rows
-- =====================================================================
CREATE VIEW insight.git_bullet_rows
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(String),
    `metric_key` String,
    `metric_value` Float64
)
AS SELECT
    c.person_id,
    p.org_unit_id,
    toString(c.metric_date) AS metric_date,
    'commits' AS metric_key,
    toFloat64(c.commits) AS metric_value
FROM insight.commits_daily AS c
LEFT JOIN insight.people AS p ON c.person_id = p.person_id
;

-- =====================================================================
-- collab_person_period
-- =====================================================================
CREATE VIEW insight.collab_person_period
(
    `metric_key` String,
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(String),
    `v` Nullable(Float64)
)
AS SELECT
    metric_key,
    person_id,
    any(org_unit_id) AS org_unit_id,
    max(metric_date) AS metric_date,
    multiIf((metric_key IN ('m365_emails_sent', 'zoom_calls', 'meeting_hours', 'm365_teams_messages', 'm365_files_shared', 'meeting_free', 'slack_thread_participation', 'slack_message_engagement')), sum(metric_value), avg(metric_value)) AS v
FROM insight.collab_bullet_rows
GROUP BY
    metric_key,
    person_id
;

-- =====================================================================
-- task_delivery_person_period
-- =====================================================================
CREATE VIEW insight.task_delivery_person_period
(
    `metric_key` String,
    `person_id` String,
    `org_unit_id` Nullable(String),
    `metric_date` String,
    `v` Nullable(Float64)
)
AS SELECT
    metric_key,
    person_id,
    any(org_unit_id) AS org_unit_id,
    max(metric_date) AS metric_date,
    multiIf(metric_key = 'tasks_completed', sum(metric_value), metric_key = 'estimation_accuracy', if(countIf((metric_value > 0) AND (metric_value <= 200)) > 0, greatest(toFloat64(0), toFloat64(100) - avgIf(abs(toFloat64(100) - metric_value), (metric_value > 0) AND (metric_value <= 200))), NULL), avg(metric_value)) AS v
FROM insight.task_delivery_bullet_rows
GROUP BY
    metric_key,
    person_id
;

-- =====================================================================
-- code_quality_person_period
-- =====================================================================
CREATE VIEW insight.code_quality_person_period
(
    `metric_key` String,
    `person_id` String,
    `org_unit_id` Nullable(String),
    `metric_date` String,
    `v` Float64
)
AS SELECT
    metric_key,
    person_id,
    any(org_unit_id) AS org_unit_id,
    max(metric_date) AS metric_date,
    multiIf((metric_key IN ('bugs_fixed', 'prs_per_dev')), sum(metric_value), avg(metric_value)) AS v
FROM insight.code_quality_bullet_rows
GROUP BY
    metric_key,
    person_id
;

-- =====================================================================
-- ai_person_period
-- =====================================================================
CREATE VIEW insight.ai_person_period
(
    `metric_key` String,
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(String),
    `v` Nullable(Float64)
)
AS SELECT
    metric_key,
    person_id,
    any(org_unit_id) AS org_unit_id,
    max(metric_date) AS metric_date,
    multiIf((metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc')), sum(metric_value), (metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active')), max(metric_value), avg(metric_value)) AS v
FROM insight.ai_bullet_rows
GROUP BY
    metric_key,
    person_id
;

-- =====================================================================
-- collab_company_stats
-- =====================================================================
CREATE VIEW insight.collab_company_stats
(
    `metric_key` String,
    `company_value` Nullable(Float64),
    `company_median` Nullable(Float64),
    `company_p5` Nullable(Float64),
    `company_p95` Nullable(Float64)
)
AS SELECT
    metric_key,
    avg(v) AS company_value,
    quantileExact(0.5)(v) AS company_median,
    min(v) AS company_p5,
    max(v) AS company_p95
FROM insight.collab_person_period
GROUP BY metric_key
;

-- =====================================================================
-- task_delivery_company_stats
-- =====================================================================
CREATE VIEW insight.task_delivery_company_stats
(
    `metric_key` String,
    `company_value` Nullable(Float64),
    `company_median` Nullable(Float64),
    `company_p5` Nullable(Float64),
    `company_p95` Nullable(Float64)
)
AS SELECT
    metric_key,
    avg(v) AS company_value,
    quantileExact(0.5)(v) AS company_median,
    min(v) AS company_p5,
    max(v) AS company_p95
FROM insight.task_delivery_person_period
GROUP BY metric_key
;

-- =====================================================================
-- code_quality_company_stats
-- =====================================================================
CREATE VIEW insight.code_quality_company_stats
(
    `metric_key` String,
    `company_value` Float64,
    `company_median` Float64,
    `company_p5` Float64,
    `company_p95` Float64
)
AS SELECT
    metric_key,
    avg(v) AS company_value,
    quantileExact(0.5)(v) AS company_median,
    min(v) AS company_p5,
    max(v) AS company_p95
FROM insight.code_quality_person_period
GROUP BY metric_key
;

-- =====================================================================
-- ai_company_stats
-- =====================================================================
CREATE VIEW insight.ai_company_stats
(
    `metric_key` String,
    `company_value` Nullable(Float64),
    `company_median` Nullable(Float64),
    `company_p5` Nullable(Float64),
    `company_p95` Nullable(Float64)
)
AS SELECT
    metric_key,
    multiIf((metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active')), sum(v), avg(v)) AS company_value,
    multiIf((metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active')), toFloat64(0), quantileExact(0.5)(v)) AS company_median,
    multiIf((metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active')), toFloat64(0), min(v)) AS company_p5,
    multiIf((metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active')), toFloat64(count()), max(v)) AS company_p95
FROM insight.ai_person_period
GROUP BY metric_key
;

-- =====================================================================
-- exec_summary
-- =====================================================================
CREATE VIEW insight.exec_summary
(
    `org_unit_id` Nullable(String),
    `org_unit_name` Nullable(String),
    `headcount` UInt32,
    `tasks_closed` UInt64,
    `bugs_fixed` UInt64,
    `build_success_pct` Nullable(Float64),
    `focus_time_pct` Nullable(Float64),
    `ai_adoption_pct` Float64,
    `ai_loc_share_pct` Float64,
    `pr_cycle_time_h` Float64,
    `metric_date` Nullable(String)
)
AS SELECT
    base.org_unit_id AS org_unit_id,
    base.org_unit_name AS org_unit_name,
    org.headcount AS headcount,
    ifNull(j.tasks_closed, 0) AS tasks_closed,
    ifNull(j.bugs_fixed, 0) AS bugs_fixed,
    CAST(NULL, 'Nullable(Float64)') AS build_success_pct,
    greatest(0, least(100, round(base.avg_focus_pct, 1))) AS focus_time_pct,
    round((ifNull(ai.active_count, 0) * 100.) / greatest(org.headcount, 1), 1) AS ai_adoption_pct,
    round(ifNull(ai.avg_ai_loc_share, 0), 1) AS ai_loc_share_pct,
    toFloat64(0) AS pr_cycle_time_h,
    base.metric_date AS metric_date
FROM
(
    SELECT
        pe.org_unit_id,
        any(pe.org_unit_name) AS org_unit_name,
        toString(f.day) AS metric_date,
        avg(f.focus_time_pct) AS avg_focus_pct
    FROM silver.class_focus_metrics AS f
    INNER JOIN insight.people AS pe ON (f.email = pe.person_id) AND (pe.status = 'Active')
    GROUP BY
        pe.org_unit_id,
        f.day
) AS base
INNER JOIN
(
    SELECT
        org_unit_id,
        toUInt32(count()) AS headcount
    FROM insight.people
    WHERE status = 'Active'
    GROUP BY org_unit_id
) AS org ON base.org_unit_id = org.org_unit_id
LEFT JOIN
(
    SELECT
        pe.org_unit_id,
        toString(j.metric_date) AS metric_date,
        sum(j.tasks_closed) AS tasks_closed,
        sum(j.bugs_fixed) AS bugs_fixed
    FROM insight.jira_closed_tasks AS j
    INNER JOIN insight.people AS pe ON (j.person_id = pe.person_id) AND (pe.status = 'Active')
    GROUP BY
        pe.org_unit_id,
        j.metric_date
) AS j ON (base.org_unit_id = j.org_unit_id) AND (base.metric_date = j.metric_date)
LEFT JOIN
(
    SELECT
        pe.org_unit_id,
        c.day AS metric_date,
        countDistinctIf(lower(c.email), c.isActive = true) AS active_count,
        avgIf(if(toFloat64(ifNull(c.totalLinesAdded, 0)) > 0, (toFloat64(ifNull(c.acceptedLinesAdded, 0)) / toFloat64(c.totalLinesAdded)) * 100, 0), c.isActive = true) AS avg_ai_loc_share
    FROM bronze_cursor.cursor_daily_usage AS c
    INNER JOIN insight.people AS pe ON (lower(c.email) = pe.person_id) AND (pe.status = 'Active')
    GROUP BY
        pe.org_unit_id,
        c.day
) AS ai ON (base.org_unit_id = ai.org_unit_id) AND (base.metric_date = ai.metric_date)
;

-- =====================================================================
-- team_member
-- =====================================================================
CREATE VIEW insight.team_member
(
    `person_id` Nullable(String),
    `display_name` Nullable(String),
    `seniority` String,
    `org_unit_id` Nullable(String),
    `tasks_closed` Float64,
    `bugs_fixed` Float64,
    `dev_time_h` Float64,
    `prs_merged` Float64,
    `build_success_pct` Nullable(Float64),
    `focus_time_pct` Float64,
    `ai_tools` Array(String),
    `ai_loc_share_pct` Float64,
    `metric_date` Nullable(String)
)
AS SELECT
    p.person_id AS person_id,
    p.display_name AS display_name,
    p.seniority AS seniority,
    p.org_unit_id AS org_unit_id,
    toFloat64(ifNull(j.tasks_closed, 0)) AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0)) AS bugs_fixed,
    greatest(0, round(ifNull(f.dev_time_h, 8.), 1)) AS dev_time_h,
    toFloat64(0) AS prs_merged,
    CAST(NULL, 'Nullable(Float64)') AS build_success_pct,
    greatest(0, least(100, round(ifNull(f.focus_time_pct, 100), 1))) AS focus_time_pct,
    if(ifNull(cur.is_active, 0) = 1, ['Cursor'], CAST([], 'Array(String)')) AS ai_tools,
    round(ifNull(cur.ai_loc_share_pct, 0), 1) AS ai_loc_share_pct,
    f.metric_date AS metric_date
FROM insight.people AS p
INNER JOIN
(
    SELECT
        email,
        toString(day) AS metric_date,
        focus_time_pct,
        dev_time_h
    FROM silver.class_focus_metrics
) AS f ON p.person_id = f.email
LEFT JOIN
(
    SELECT
        person_id,
        toString(metric_date) AS metric_date,
        sum(tasks_closed) AS tasks_closed,
        sum(bugs_fixed) AS bugs_fixed
    FROM insight.jira_closed_tasks
    GROUP BY
        person_id,
        metric_date
) AS j ON (p.person_id = j.person_id) AND (f.metric_date = j.metric_date)
LEFT JOIN
(
    SELECT
        lower(email) AS person_id,
        day AS metric_date,
        if(isActive = true, 1, 0) AS is_active,
        if(toFloat64(ifNull(totalLinesAdded, 0)) > 0, round((toFloat64(ifNull(acceptedLinesAdded, 0)) / toFloat64(totalLinesAdded)) * 100, 1), 0) AS ai_loc_share_pct
    FROM bronze_cursor.cursor_daily_usage
) AS cur ON (p.person_id = cur.person_id) AND (f.metric_date = cur.metric_date)
WHERE p.status = 'Active'
;

-- =====================================================================
-- ic_kpis
-- =====================================================================
CREATE VIEW insight.ic_kpis
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(String),
    `loc` Float64,
    `ai_loc_share_pct` Float64,
    `prs_merged` Float64,
    `pr_cycle_time_h` Float64,
    `focus_time_pct` Float64,
    `tasks_closed` Float64,
    `bugs_fixed` Float64,
    `build_success_pct` Nullable(Float64),
    `ai_sessions` Float64
)
AS SELECT
    f.email AS person_id,
    p.org_unit_id,
    toString(f.day) AS metric_date,
    toFloat64(ifNull(cur.total_lines, 0)) AS loc,
    round(ifNull(cur.ai_loc_share_pct, 0), 1) AS ai_loc_share_pct,
    toFloat64(0) AS prs_merged,
    toFloat64(0) AS pr_cycle_time_h,
    greatest(0, least(100, round(ifNull(f.focus_time_pct, 100), 1))) AS focus_time_pct,
    toFloat64(ifNull(j.tasks_closed, 0)) AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0)) AS bugs_fixed,
    CAST(NULL, 'Nullable(Float64)') AS build_success_pct,
    toFloat64(ifNull(cur.ai_sessions, 0)) AS ai_sessions
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id
LEFT JOIN
(
    SELECT
        person_id,
        toString(metric_date) AS metric_date,
        sum(tasks_closed) AS tasks_closed,
        sum(bugs_fixed) AS bugs_fixed
    FROM insight.jira_closed_tasks
    GROUP BY
        person_id,
        metric_date
) AS j ON (f.email = j.person_id) AND (toString(f.day) = j.metric_date)
LEFT JOIN
(
    SELECT
        lower(email) AS person_id,
        day AS metric_date,
        toFloat64(ifNull(totalLinesAdded, 0)) AS total_lines,
        if(toFloat64(ifNull(totalLinesAdded, 0)) > 0, round((toFloat64(ifNull(acceptedLinesAdded, 0)) / toFloat64(totalLinesAdded)) * 100, 1), 0) AS ai_loc_share_pct,
        (toFloat64(ifNull(agentRequests, 0)) + toFloat64(ifNull(chatRequests, 0))) + toFloat64(ifNull(composerRequests, 0)) AS ai_sessions
    FROM bronze_cursor.cursor_daily_usage
) AS cur ON (f.email = cur.person_id) AND (toString(f.day) = cur.metric_date)
;

-- =====================================================================
-- ic_chart_delivery
-- =====================================================================
CREATE VIEW insight.ic_chart_delivery
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `date_bucket` Nullable(String),
    `metric_date` Nullable(String),
    `commits` UInt64,
    `prs_merged` UInt64,
    `tasks_done` UInt64
)
AS WITH
    weekly_commits AS
    (
        SELECT
            person_id,
            toStartOfWeek(metric_date) AS week,
            sum(commits) AS commits
        FROM insight.commits_daily
        GROUP BY
            person_id,
            week
    ),
    weekly_jira AS
    (
        SELECT
            person_id,
            toStartOfWeek(metric_date) AS week,
            sum(tasks_closed) AS tasks_done
        FROM insight.jira_closed_tasks
        GROUP BY
            person_id,
            week
    ),
    days_all AS
    (
        SELECT
            person_id,
            week
        FROM weekly_commits
        UNION DISTINCT
        SELECT
            person_id,
            week
        FROM weekly_jira
    )
SELECT
    d.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    toString(d.week) AS date_bucket,
    toString(d.week) AS metric_date,
    toUInt64(ifNull(c.commits, 0)) AS commits,
    toUInt64(0) AS prs_merged,
    toUInt64(ifNull(j.tasks_done, 0)) AS tasks_done
FROM days_all AS d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id
LEFT JOIN weekly_commits AS c ON (c.person_id = d.person_id) AND (c.week = d.week)
LEFT JOIN weekly_jira AS j ON (j.person_id = d.person_id) AND (j.week = d.week)
;

-- =====================================================================
-- ic_chart_loc
-- =====================================================================
CREATE VIEW insight.ic_chart_loc
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `date_bucket` Nullable(String),
    `metric_date` Nullable(String),
    `ai_loc` Float64,
    `code_loc` Float64,
    `spec_lines` Float64
)
AS SELECT
    lower(c.email) AS person_id,
    p.org_unit_id,
    toString(toStartOfWeek(toDate(c.day))) AS date_bucket,
    toString(toStartOfWeek(toDate(c.day))) AS metric_date,
    toFloat64(sum(ifNull(c.acceptedLinesAdded, 0))) AS ai_loc,
    toFloat64(sum(ifNull(c.totalLinesAdded, 0)) - sum(ifNull(c.acceptedLinesAdded, 0))) AS code_loc,
    toFloat64(0) AS spec_lines
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
GROUP BY
    lower(c.email),
    p.org_unit_id,
    toStartOfWeek(toDate(c.day))
;

-- =====================================================================
-- ic_drill
-- =====================================================================
CREATE VIEW insight.ic_drill
(
    `person_id` String,
    `org_unit_id` String,
    `metric_date` String,
    `drill_id` String,
    `title` String,
    `source` String,
    `src_class` String,
    `value` String,
    `filter` String,
    `columns` Array(String),
    `rows` Array(String)
)
AS SELECT
    '' AS person_id,
    '' AS org_unit_id,
    '' AS metric_date,
    '' AS drill_id,
    '' AS title,
    '' AS source,
    '' AS src_class,
    '' AS value,
    '' AS filter,
    CAST([], 'Array(String)') AS columns,
    CAST([], 'Array(String)') AS rows
FROM system.one
WHERE 0
;

-- =====================================================================
-- ic_timeoff
-- =====================================================================
CREATE VIEW insight.ic_timeoff
(
    `person_id` String,
    `org_unit_id` String,
    `metric_date` String,
    `days` UInt32,
    `date_range` String,
    `bamboo_hr_url` String
)
AS SELECT
    '' AS person_id,
    '' AS org_unit_id,
    '' AS metric_date,
    toUInt32(0) AS days,
    '' AS date_range,
    '' AS bamboo_hr_url
FROM system.one
WHERE 0
;
