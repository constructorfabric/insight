CREATE DATABASE IF NOT EXISTS `bronze_cursor`;

CREATE TABLE IF NOT EXISTS bronze_cursor.cursor_audit_logs
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `event_id` Nullable(String),
    `timestamp` Nullable(String),
    `user_email` Nullable(String),
    `event_type` Nullable(String),
    `event_data` Nullable(String),
    `ip_address` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_cursor.cursor_daily_usage
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `userId` Nullable(String),
    `email` Nullable(String),
    `day` Nullable(String),
    `date` Nullable(Decimal(38, 9)),
    `isActive` Nullable(Bool),
    `chatRequests` Nullable(Decimal(38, 9)),
    `cmdkUsages` Nullable(Decimal(38, 9)),
    `composerRequests` Nullable(Decimal(38, 9)),
    `agentRequests` Nullable(Decimal(38, 9)),
    `bugbotUsages` Nullable(Decimal(38, 9)),
    `totalTabsShown` Nullable(Decimal(38, 9)),
    `totalTabsAccepted` Nullable(Decimal(38, 9)),
    `totalAccepts` Nullable(Decimal(38, 9)),
    `totalApplies` Nullable(Decimal(38, 9)),
    `totalRejects` Nullable(Decimal(38, 9)),
    `totalLinesAdded` Nullable(Decimal(38, 9)),
    `totalLinesDeleted` Nullable(Decimal(38, 9)),
    `acceptedLinesAdded` Nullable(Decimal(38, 9)),
    `acceptedLinesDeleted` Nullable(Decimal(38, 9)),
    `mostUsedModel` Nullable(String),
    `tabMostUsedExtension` Nullable(String),
    `applyMostUsedExtension` Nullable(String),
    `clientVersion` Nullable(String),
    `subscriptionIncludedReqs` Nullable(Decimal(38, 9)),
    `usageBasedReqs` Nullable(Decimal(38, 9)),
    `apiKeyReqs` Nullable(Decimal(38, 9))
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_cursor.cursor_members
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `id` Nullable(String),
    `name` Nullable(String),
    `email` Nullable(String),
    `role` Nullable(String),
    `isRemoved` Nullable(Bool)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_cursor.cursor_usage_events
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `userEmail` Nullable(String),
    `timestamp` Nullable(String),
    `kind` Nullable(String),
    `model` Nullable(String),
    `maxMode` Nullable(Bool),
    `requestsCosts` Nullable(Decimal(38, 9)),
    `isTokenBasedCall` Nullable(Bool),
    `isFreeBugbot` Nullable(Bool),
    `cursorTokenFee` Nullable(Decimal(38, 9)),
    `isChargeable` Nullable(Bool),
    `isHeadless` Nullable(Bool),
    `chargedCents` Nullable(Decimal(38, 9)),
    `tokenUsage` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_cursor.cursor_usage_events_daily_resync
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `userEmail` Nullable(String),
    `timestamp` Nullable(String),
    `kind` Nullable(String),
    `model` Nullable(String),
    `maxMode` Nullable(Bool),
    `requestsCosts` Nullable(Decimal(38, 9)),
    `isTokenBasedCall` Nullable(Bool),
    `isFreeBugbot` Nullable(Bool),
    `cursorTokenFee` Nullable(Decimal(38, 9)),
    `isChargeable` Nullable(Bool),
    `isHeadless` Nullable(Bool),
    `chargedCents` Nullable(Decimal(38, 9)),
    `tokenUsage` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

