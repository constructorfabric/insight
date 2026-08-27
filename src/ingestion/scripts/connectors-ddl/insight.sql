CREATE DATABASE IF NOT EXISTS `insight`;

CREATE TABLE IF NOT EXISTS insight.account_attribute_values
(
    `insight_tenant_id` String,
    `insight_source_type` String,
    `insight_source_id` String,
    `source_account_id` String,
    `field_id` String,
    `value_id` Nullable(String),
    `value_label` String,
    `valid_from` DateTime64(3),
    `valid_to` Nullable(DateTime64(3)),
    `ingested_at` DateTime64(3)
)
ENGINE = MergeTree
ORDER BY (insight_tenant_id, insight_source_type, insight_source_id, source_account_id, field_id, valid_from)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_cost_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_cost_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ci_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `source_entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, Nullable(String))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ci_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.collab_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.collab_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_authored_commits
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `commit_hash` String,
    `data_source` String,
    `author_name` String,
    `message` String,
    `observed_at` Nullable(DateTime),
    `entity_id` String,
    `metric_date` Nullable(Date),
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64),
    `branch_scope_value` String,
    `branch_scope_label` String,
    `project_value` String,
    `project_label` String,
    `repository_value` String,
    `repository_label` String,
    `source_value` String,
    `source_label` String,
    `source_dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
ORDER BY (tenant_id, data_source, commit_hash)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_commit_file_changes
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `commit_hash` String,
    `observed_at` Nullable(DateTime),
    `data_source` String,
    `file_path` String,
    `file_extension` String,
    `change_type` String,
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64),
    `pre_image_oid` Nullable(String),
    `post_image_oid` Nullable(String)
)
ENGINE = MergeTree
ORDER BY (tenant_id, data_source, commit_hash)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_default_branch_commits
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `commit_hash` String
)
ENGINE = MergeTree
ORDER BY (tenant_id, source_id, project_key, repo_slug, commit_hash)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_derived_commits
(
    `tenant_id` Nullable(String),
    `data_source` String,
    `commit_hash` String,
    `reason` String
)
ENGINE = MergeTree
ORDER BY (tenant_id, data_source, commit_hash)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_review_events
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `event_kind` String,
    `event_key` Nullable(String),
    `pr_id` Int64,
    `pr_number` Int64,
    `entity_id` String,
    `metric_date` Nullable(Date),
    `observed_at` Nullable(DateTime),
    `title` String,
    `author_name` String,
    `author_email` String,
    `actor_person_id` String,
    `author_person_id` String,
    `comment_target_value` String,
    `comment_target_label` String,
    `project_value` String,
    `project_label` String,
    `repository_value` String,
    `repository_label` String,
    `destination_branch_value` String,
    `destination_branch_label` String,
    `source_value` String,
    `source_label` String,
    `source_dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
ORDER BY (tenant_id, entity_id, metric_date)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_issue_state
(
    `tenant_id` Nullable(String),
    `entity_id` Nullable(String),
    `insight_source_id` String,
    `data_source` String,
    `issue_id` String,
    `id_readable` String,
    `title` Nullable(String),
    `status_category` String,
    `issue_type` String,
    `issue_kind` String,
    `issue_type_key` Nullable(String),
    `issue_type_name` Nullable(String),
    `due_date` Nullable(Date),
    `time_estimate_seconds` Nullable(Float64),
    `time_spent_seconds` Nullable(Float64),
    `created_at` DateTime64(3),
    `final_close_at` Nullable(DateTime64(3)),
    `last_status_event_at` DateTime64(3)
)
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_status_spans
(
    `insight_source_id` String,
    `issue_id` String,
    `interval_start` DateTime64(3),
    `interval_end` DateTime64(3),
    `status_category` String,
    `duration_seconds` Float64
)
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id, interval_start)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_worklog_flow
(
    `tenant_id` String,
    `entity_id` String,
    `metric_date` Date,
    `in_progress_seconds` Float64,
    `worklog_seconds` Float64
)
ENGINE = MergeTree
ORDER BY (tenant_id, entity_id, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.wiki_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.wiki_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE OR REPLACE VIEW insight.identity_resolution_coverage
(
    `source_key` String,
    `observation_rows` UInt64,
    `unresolved_rows` UInt64,
    `unresolved_people` UInt64,
    `match_rate_pct` Float64
)
AS WITH
    source_identities AS
    (
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.git_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.ai_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.collab_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.task_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.wiki_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.ai_cost_metric_evidence
        UNION ALL
        SELECT DISTINCT
            'hr_cohorts' AS source_key,
            lower(trimBoth(assumeNotNull(email))) AS email,
            '' AS account_source_type,
            '' AS account_source_id,
            '' AS account_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    resolution AS
    (
        SELECT
            source_identities.source_key AS source_key,
            source_identities.email AS email,
            source_identities.account_id AS account_id,
            (coalesce(account_map.account_id, '') != '') OR (coalesce(person_map.email, '') != '') AS resolved
        FROM source_identities
        LEFT JOIN identity.account_assignment AS account_map ON (account_map.source_type = source_identities.account_source_type) AND (account_map.source_id = toUUID(UUIDNumToString(sipHash128(coalesce(source_identities.account_source_id, ''))))) AND (account_map.account_id = lower(trimBoth(coalesce(source_identities.account_id, ''))))
        LEFT JOIN identity.person_map AS person_map ON person_map.email = source_identities.email
    )
SELECT
    source_key,
    count() AS observation_rows,
    countIf(NOT resolved) AS unresolved_rows,
    uniqExactIf(if(email != '', email, coalesce(account_id, '')), NOT resolved) AS unresolved_people,
    round((100 * countIf(resolved)) / count(), 1) AS match_rate_pct
FROM resolution
GROUP BY source_key
;

CREATE OR REPLACE VIEW insight.metric_entity_cohorts_current
(
    `tenant_id` String,
    `entity_type` String,
    `entity_id` String,
    `cohort_key` String,
    `cohort_id` Nullable(String)
)
AS SELECT
    tenant_id,
    entity_type,
    entity_id,
    cohort_key,
    resolved_cohort_id AS cohort_id
FROM
(
    SELECT
        tenant_id,
        entity_type,
        entity_id,
        cohort_key,
        any(cohort_id) AS resolved_cohort_id
    FROM
    (
        SELECT
            assumeNotNull(people.tenant_id) AS tenant_id,
            'person' AS entity_type,
            toString(person_map.person_id) AS entity_id,
            'org_unit' AS cohort_key,
            people.cohort_id AS cohort_id
        FROM
        (
            SELECT
                workspace_id AS tenant_id,
                lower(trimBoth(assumeNotNull(email))) AS email,
                nullIf(department_name, '') AS cohort_id
            FROM silver.class_people
            WHERE (email IS NOT NULL) AND (email != '') AND (workspace_id IS NOT NULL) AND (workspace_id != '')
            ORDER BY
                tenant_id ASC,
                email ASC,
                coalesce(parseDateTimeBestEffortOrNull(toString(valid_from)), toDateTime('1970-01-01')) DESC,
                unique_key DESC
            LIMIT 1 BY
                tenant_id,
                email
        ) AS people
        INNER JOIN identity.person_map AS person_map ON person_map.email = people.email
        WHERE (people.tenant_id IS NOT NULL) AND (people.tenant_id != '') AND (people.email != '') AND (people.cohort_id IS NOT NULL)
    ) AS resolved
    GROUP BY
        tenant_id,
        entity_type,
        entity_id,
        cohort_key
    HAVING uniqExact(cohort_id) = 1
)
;

