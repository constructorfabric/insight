CREATE DATABASE IF NOT EXISTS `identity`;

CREATE TABLE IF NOT EXISTS identity.identity_inputs
(
    `unique_key` String,
    `insight_tenant_id` UUID,
    `insight_source_id` UUID,
    `insight_source_type` String,
    `source_account_id` Nullable(String),
    `value_type` String,
    `value` Nullable(String),
    `value_field_name` String,
    `operation_type` String,
    `_synced_at` DateTime64(3),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS identity.identity_persons
(
    `id` UInt64,
    `value_type` String,
    `insight_source_type` String,
    `insight_source_id` UUID,
    `insight_tenant_id` UUID,
    `value_id` Nullable(String),
    `value_full_text` Nullable(String),
    `value` Nullable(String),
    `value_effective` Nullable(String),
    `person_id` UUID,
    `author_person_id` UUID,
    `reason` Nullable(String),
    `created_at` DateTime64(6, 'UTC'),
    `_synced_at` DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY id
SETTINGS index_granularity = 8192
;

CREATE OR REPLACE VIEW identity.account_assignment
(
    `source_type` String,
    `source_id` UUID,
    `account_id` String,
    `person_id` UUID,
    `created_at` DateTime64(6, 'UTC')
)
AS SELECT
    insight_source_type AS source_type,
    insight_source_id AS source_id,
    lower(trimBoth(assumeNotNull(value_effective))) AS account_id,
    person_id,
    created_at
FROM identity.identity_persons
WHERE (value_type = 'id') AND (value_effective IS NOT NULL) AND (trimBoth(value_effective) != '')
ORDER BY
    source_type ASC,
    source_id ASC,
    account_id ASC,
    created_at DESC,
    id DESC
LIMIT 1 BY
    source_type,
    source_id,
    account_id
;

CREATE OR REPLACE VIEW identity.person_map
(
    `email` Nullable(String),
    `person_id` UUID
)
AS WITH account_emails AS
    (
        SELECT DISTINCT
            insight_source_type AS source_type,
            insight_source_id AS source_id,
            lower(trimBoth(source_account_id)) AS account_id,
            lower(trimBoth(value)) AS email
        FROM identity.identity_inputs
        WHERE (value_type = 'email') AND (operation_type = 'UPSERT') AND (coalesce(value, '') != '') AND (coalesce(source_account_id, '') != '')
    )
SELECT
    account_emails.email AS email,
    any(assignment.person_id) AS person_id
FROM account_emails
INNER JOIN identity.account_assignment AS assignment ON (assignment.source_type = account_emails.source_type) AND (assignment.source_id = account_emails.source_id) AND (assignment.account_id = account_emails.account_id)
WHERE (account_emails.email != '') AND (assignment.person_id != toUUID('ffffffff-ffff-ffff-ffff-ffffffffffff'))
GROUP BY account_emails.email
HAVING uniqExact(assignment.person_id) = 1
;

