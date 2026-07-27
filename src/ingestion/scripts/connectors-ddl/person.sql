CREATE DATABASE IF NOT EXISTS `person`;

CREATE TABLE IF NOT EXISTS person.persons
(
    `id` UUID DEFAULT generateUUIDv7(),
    `insight_tenant_id` UUID,
    `display_name` String DEFAULT '',
    `display_name_source` LowCardinality(String) DEFAULT '',
    `status` LowCardinality(String) DEFAULT 'active',
    `email` String DEFAULT '',
    `email_source` LowCardinality(String) DEFAULT '',
    `username` String DEFAULT '',
    `username_source` LowCardinality(String) DEFAULT '',
    `role` String DEFAULT '',
    `role_source` LowCardinality(String) DEFAULT '',
    `manager_person_id` UUID DEFAULT toUUID('00000000-0000-0000-0000-000000000000'),
    `manager_person_id_source` LowCardinality(String) DEFAULT '',
    `org_unit_id` UUID DEFAULT toUUID('00000000-0000-0000-0000-000000000000'),
    `org_unit_id_source` LowCardinality(String) DEFAULT '',
    `location` String DEFAULT '',
    `location_source` LowCardinality(String) DEFAULT '',
    `completeness_score` Float32 DEFAULT 0.,
    `conflict_status` LowCardinality(String) DEFAULT 'clean',
    `created_at` DateTime64(3, 'UTC') DEFAULT now64(3),
    `updated_at` DateTime64(3, 'UTC') DEFAULT now64(3),
    `is_deleted` UInt8 DEFAULT 0
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (insight_tenant_id, id)
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS person.seed_persons_from_claude_admin
(
    `id` UUID,
    `insight_tenant_id` String,
    `display_name` String,
    `display_name_source` String,
    `status` String,
    `email` Nullable(String),
    `email_source` String,
    `username` String,
    `username_source` String,
    `role` String,
    `role_source` String,
    `manager_person_id` UUID,
    `manager_person_id_source` String,
    `org_unit_id` UUID,
    `org_unit_id_source` String,
    `location` String,
    `location_source` String,
    `completeness_score` Float64,
    `conflict_status` String,
    `created_at` DateTime64(3),
    `updated_at` DateTime64(3),
    `is_deleted` UInt8,
    `_version` Int64
)
ENGINE = MergeTree
ORDER BY id
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS person.seed_persons_from_cursor
(
    `id` UUID,
    `insight_tenant_id` String,
    `display_name` String,
    `display_name_source` String,
    `status` String,
    `email` String,
    `email_source` String,
    `username` String,
    `username_source` String,
    `role` String,
    `role_source` String,
    `manager_person_id` UUID,
    `manager_person_id_source` String,
    `org_unit_id` UUID,
    `org_unit_id_source` String,
    `location` String,
    `location_source` String,
    `completeness_score` Float64,
    `conflict_status` String,
    `created_at` DateTime64(3),
    `updated_at` DateTime64(3),
    `is_deleted` UInt8,
    `_version` Int64
)
ENGINE = MergeTree
ORDER BY id
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

