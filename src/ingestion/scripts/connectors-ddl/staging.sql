CREATE DATABASE IF NOT EXISTS `staging`;

CREATE TABLE IF NOT EXISTS staging.m365__collab_email_activity
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(FixedString(16)),
    `user_id` Nullable(String),
    `user_name` Nullable(String),
    `email` Nullable(String),
    `person_key` Nullable(String),
    `date` Nullable(Date),
    `sent_count` Nullable(Decimal(38, 9)),
    `received_count` Nullable(Decimal(38, 9)),
    `read_count` Nullable(Decimal(38, 9)),
    `meetings_created` Nullable(Decimal(38, 9)),
    `meetings_interacted` Nullable(Decimal(38, 9)),
    `report_period` Nullable(String),
    `collected_at` DateTime,
    `data_source` String,
    `_version` Int64
)
ENGINE = MergeTree
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

