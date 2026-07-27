CREATE DATABASE IF NOT EXISTS `bronze_m365`;

CREATE TABLE IF NOT EXISTS bronze_m365.email_activity
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `reportRefreshDate` Nullable(String),
    `userPrincipalName` Nullable(String),
    `displayName` Nullable(String),
    `isDeleted` Nullable(Bool),
    `lastActivityDate` Nullable(String),
    `sendCount` Nullable(Decimal(38, 9)),
    `receiveCount` Nullable(Decimal(38, 9)),
    `readCount` Nullable(Decimal(38, 9)),
    `meetingCreatedCount` Nullable(Decimal(38, 9)),
    `meetingInteractedCount` Nullable(Decimal(38, 9)),
    `assignedProducts` Nullable(String),
    `reportPeriod` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_m365.onedrive_activity
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `reportRefreshDate` Nullable(String),
    `userPrincipalName` Nullable(String),
    `isDeleted` Nullable(Bool),
    `lastActivityDate` Nullable(String),
    `viewedOrEditedFileCount` Nullable(Decimal(38, 9)),
    `syncedFileCount` Nullable(Decimal(38, 9)),
    `sharedInternallyFileCount` Nullable(Decimal(38, 9)),
    `sharedExternallyFileCount` Nullable(Decimal(38, 9)),
    `assignedProducts` Nullable(String),
    `reportPeriod` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_m365.sharepoint_activity
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `reportRefreshDate` Nullable(String),
    `userPrincipalName` Nullable(String),
    `isDeleted` Nullable(Bool),
    `lastActivityDate` Nullable(String),
    `viewedOrEditedFileCount` Nullable(Decimal(38, 9)),
    `syncedFileCount` Nullable(Decimal(38, 9)),
    `sharedInternallyFileCount` Nullable(Decimal(38, 9)),
    `sharedExternallyFileCount` Nullable(Decimal(38, 9)),
    `visitedPageCount` Nullable(Decimal(38, 9)),
    `assignedProducts` Nullable(String),
    `reportPeriod` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_m365.teams_activity
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenantDisplayName` Nullable(String),
    `sharedChannelTenantDisplayNames` Nullable(String),
    `reportRefreshDate` Nullable(String),
    `userId` Nullable(String),
    `userPrincipalName` Nullable(String),
    `lastActivityDate` Nullable(String),
    `isDeleted` Nullable(Bool),
    `isExternal` Nullable(Bool),
    `assignedProducts` Nullable(String),
    `teamChatMessageCount` Nullable(Decimal(38, 9)),
    `privateChatMessageCount` Nullable(Decimal(38, 9)),
    `callCount` Nullable(Decimal(38, 9)),
    `meetingCount` Nullable(Decimal(38, 9)),
    `meetingsOrganizedCount` Nullable(Decimal(38, 9)),
    `meetingsAttendedCount` Nullable(Decimal(38, 9)),
    `adHocMeetingsOrganizedCount` Nullable(Decimal(38, 9)),
    `adHocMeetingsAttendedCount` Nullable(Decimal(38, 9)),
    `scheduledOneTimeMeetingsOrganizedCount` Nullable(Decimal(38, 9)),
    `scheduledOneTimeMeetingsAttendedCount` Nullable(Decimal(38, 9)),
    `scheduledRecurringMeetingsOrganizedCount` Nullable(Decimal(38, 9)),
    `scheduledRecurringMeetingsAttendedCount` Nullable(Decimal(38, 9)),
    `audioDuration` Nullable(String),
    `videoDuration` Nullable(String),
    `screenShareDuration` Nullable(String),
    `hasOtherAction` Nullable(Bool),
    `urgentMessages` Nullable(Decimal(38, 9)),
    `postMessages` Nullable(Decimal(38, 9)),
    `replyMessages` Nullable(Decimal(38, 9)),
    `isLicensed` Nullable(Bool),
    `reportPeriod` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

