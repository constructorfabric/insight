-- INVARIANT: gold reads this class in the deploy that introduces it, before any sync
-- has built it; column order is m365__collab_calendar_activity's SELECT order.

CREATE TABLE IF NOT EXISTS silver.class_collab_calendar_activity
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(FixedString(16)),
    `email` Nullable(String),
    `person_key` Nullable(String),
    `date` Date,
    `meetings` Int64,
    `meeting_seconds` Int64,
    `collected_at` DateTime,
    `data_source` String,
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1;
