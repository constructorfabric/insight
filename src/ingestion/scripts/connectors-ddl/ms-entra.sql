CREATE DATABASE IF NOT EXISTS `bronze_ms_entra`;

CREATE TABLE IF NOT EXISTS bronze_ms_entra.users
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `userPrincipalName` Nullable(String),
    `mail` Nullable(String),
    `proxyAddresses` Nullable(String),
    `otherMails` Nullable(String),
    `displayName` Nullable(String),
    `givenName` Nullable(String),
    `surname` Nullable(String),
    `employeeId` Nullable(String),
    `department` Nullable(String),
    `jobTitle` Nullable(String),
    `accountEnabled` Nullable(Bool),
    `onPremisesSamAccountName` Nullable(String),
    `createdDateTime` Nullable(String),
    `userType` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

