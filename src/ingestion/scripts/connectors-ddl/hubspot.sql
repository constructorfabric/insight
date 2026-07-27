CREATE DATABASE IF NOT EXISTS `bronze_hubspot`;

CREATE TABLE IF NOT EXISTS bronze_hubspot.companies
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_annualrevenue` Nullable(String),
    `properties_city` Nullable(String),
    `properties_country` Nullable(String),
    `properties_domain` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `properties_industry` Nullable(String),
    `properties_lifecyclestage` Nullable(String),
    `properties_name` Nullable(String),
    `properties_numberofemployees` Nullable(String),
    `properties_phone` Nullable(String),
    `properties_state` Nullable(String),
    `properties_type` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.companies_archived
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_annualrevenue` Nullable(String),
    `properties_city` Nullable(String),
    `properties_country` Nullable(String),
    `properties_domain` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `properties_industry` Nullable(String),
    `properties_lifecyclestage` Nullable(String),
    `properties_name` Nullable(String),
    `properties_numberofemployees` Nullable(String),
    `properties_phone` Nullable(String),
    `properties_state` Nullable(String),
    `properties_type` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.contacts
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_city` Nullable(String),
    `properties_country` Nullable(String),
    `properties_email` Nullable(String),
    `properties_firstname` Nullable(String),
    `properties_hs_analytics_source` Nullable(String),
    `properties_hs_lead_status` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `properties_jobtitle` Nullable(String),
    `properties_lastname` Nullable(String),
    `properties_lifecyclestage` Nullable(String),
    `properties_phone` Nullable(String),
    `properties_state` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.contacts_archived
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_city` Nullable(String),
    `properties_country` Nullable(String),
    `properties_email` Nullable(String),
    `properties_firstname` Nullable(String),
    `properties_hs_analytics_source` Nullable(String),
    `properties_hs_lead_status` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `properties_jobtitle` Nullable(String),
    `properties_lastname` Nullable(String),
    `properties_lifecyclestage` Nullable(String),
    `properties_phone` Nullable(String),
    `properties_state` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.deals
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_amount` Nullable(String),
    `properties_amount_in_home_currency` Nullable(String),
    `properties_closed_lost_reason` Nullable(String),
    `properties_closedate` Nullable(String),
    `properties_dealname` Nullable(String),
    `properties_dealstage` Nullable(String),
    `properties_dealtype` Nullable(String),
    `properties_description` Nullable(String),
    `properties_hs_acv` Nullable(String),
    `properties_hs_analytics_source` Nullable(String),
    `properties_hs_arr` Nullable(String),
    `properties_hs_created_by_user_id` Nullable(String),
    `properties_hs_deal_stage_probability` Nullable(String),
    `properties_hs_is_closed` Nullable(String),
    `properties_hs_is_closed_won` Nullable(String),
    `properties_hs_manual_forecast_category` Nullable(String),
    `properties_hs_priority` Nullable(String),
    `properties_hs_tcv` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `properties_pipeline` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_contacts` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.deals_archived
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_amount` Nullable(String),
    `properties_amount_in_home_currency` Nullable(String),
    `properties_closed_lost_reason` Nullable(String),
    `properties_closedate` Nullable(String),
    `properties_dealname` Nullable(String),
    `properties_dealstage` Nullable(String),
    `properties_dealtype` Nullable(String),
    `properties_description` Nullable(String),
    `properties_hs_acv` Nullable(String),
    `properties_hs_analytics_source` Nullable(String),
    `properties_hs_arr` Nullable(String),
    `properties_hs_created_by_user_id` Nullable(String),
    `properties_hs_deal_stage_probability` Nullable(String),
    `properties_hs_is_closed` Nullable(String),
    `properties_hs_is_closed_won` Nullable(String),
    `properties_hs_manual_forecast_category` Nullable(String),
    `properties_hs_priority` Nullable(String),
    `properties_hs_tcv` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `properties_pipeline` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_contacts` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.engagements_calls
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_hs_call_direction` Nullable(String),
    `properties_hs_call_disposition` Nullable(String),
    `properties_hs_call_duration` Nullable(String),
    `properties_hs_call_status` Nullable(String),
    `properties_hs_call_title` Nullable(String),
    `properties_hs_created_by_user_id` Nullable(String),
    `properties_hs_timestamp` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.engagements_calls_archived
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_hs_call_direction` Nullable(String),
    `properties_hs_call_disposition` Nullable(String),
    `properties_hs_call_duration` Nullable(String),
    `properties_hs_call_status` Nullable(String),
    `properties_hs_call_title` Nullable(String),
    `properties_hs_created_by_user_id` Nullable(String),
    `properties_hs_timestamp` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.engagements_emails
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_hs_created_by_user_id` Nullable(String),
    `properties_hs_email_direction` Nullable(String),
    `properties_hs_email_status` Nullable(String),
    `properties_hs_email_subject` Nullable(String),
    `properties_hs_timestamp` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.engagements_emails_archived
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_hs_created_by_user_id` Nullable(String),
    `properties_hs_email_direction` Nullable(String),
    `properties_hs_email_status` Nullable(String),
    `properties_hs_email_subject` Nullable(String),
    `properties_hs_timestamp` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.engagements_meetings
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_hs_created_by_user_id` Nullable(String),
    `properties_hs_internal_meeting_notes` Nullable(String),
    `properties_hs_meeting_end_time` Nullable(String),
    `properties_hs_meeting_external_url` Nullable(String),
    `properties_hs_meeting_location` Nullable(String),
    `properties_hs_meeting_outcome` Nullable(String),
    `properties_hs_meeting_start_time` Nullable(String),
    `properties_hs_meeting_title` Nullable(String),
    `properties_hs_timestamp` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.engagements_tasks
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_hs_created_by_user_id` Nullable(String),
    `properties_hs_task_completion_date` Nullable(String),
    `properties_hs_task_priority` Nullable(String),
    `properties_hs_task_status` Nullable(String),
    `properties_hs_task_subject` Nullable(String),
    `properties_hs_task_type` Nullable(String),
    `properties_hs_timestamp` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.engagements_tasks_archived
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `properties_hs_created_by_user_id` Nullable(String),
    `properties_hs_task_completion_date` Nullable(String),
    `properties_hs_task_priority` Nullable(String),
    `properties_hs_task_status` Nullable(String),
    `properties_hs_task_subject` Nullable(String),
    `properties_hs_task_type` Nullable(String),
    `properties_hs_timestamp` Nullable(String),
    `properties_hubspot_owner_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.leads
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.leads_archived
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.owners
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `email` Nullable(String),
    `firstName` Nullable(String),
    `lastName` Nullable(String),
    `userId` Nullable(Int64),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.owners_archived
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `email` Nullable(String),
    `firstName` Nullable(String),
    `lastName` Nullable(String),
    `userId` Nullable(Int64),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.tickets
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_hubspot.tickets_archived
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `createdAt` Nullable(DateTime64(3)),
    `updatedAt` Nullable(DateTime64(3)),
    `archived` Nullable(Bool),
    `archivedAt` Nullable(DateTime64(3)),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `custom_fields` Nullable(String),
    `associations_contacts` Nullable(String),
    `associations_companies` Nullable(String),
    `associations_deals` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

