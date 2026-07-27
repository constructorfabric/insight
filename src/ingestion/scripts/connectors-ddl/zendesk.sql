CREATE DATABASE IF NOT EXISTS `bronze_zendesk`;

CREATE TABLE IF NOT EXISTS bronze_zendesk.support_agents
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String),
    `agent_id` Nullable(String),
    `email` Nullable(String),
    `display_name` Nullable(String),
    `role` Nullable(String),
    `group_id` Nullable(String),
    `group_name` Nullable(String),
    `is_active` Nullable(Int64)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_zendesk.support_ticket_events
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String),
    `audit_id` Nullable(String),
    `ticket_id` Nullable(String),
    `author_id` Nullable(String),
    `created_at` Nullable(String),
    `events` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_zendesk.support_ticket_ids
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String),
    `ticket_id` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_zendesk.support_tickets
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String),
    `ticket_id` Nullable(String),
    `subject` Nullable(String),
    `status` Nullable(String),
    `priority` Nullable(String),
    `ticket_type` Nullable(String),
    `assignee_id` Nullable(String),
    `group_id` Nullable(String),
    `requester_id` Nullable(String),
    `organization_id` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `solved_at` Nullable(String),
    `first_reply_time_seconds` Nullable(Int64),
    `first_reply_time_calendar_seconds` Nullable(Int64),
    `full_resolution_time_seconds` Nullable(Int64),
    `full_resolution_time_calendar_seconds` Nullable(Int64),
    `satisfaction_score` Nullable(String),
    `tags` Nullable(String),
    `metadata` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_zendesk.zendesk_satisfaction_ratings
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String),
    `rating_id` Nullable(String),
    `ticket_id` Nullable(String),
    `requester_id` Nullable(String),
    `assignee_id` Nullable(String),
    `group_id` Nullable(String),
    `score` Nullable(String),
    `comment` Nullable(String),
    `reason` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

