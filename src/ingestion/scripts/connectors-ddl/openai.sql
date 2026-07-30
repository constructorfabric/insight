CREATE DATABASE IF NOT EXISTS `bronze_openai`;

CREATE TABLE IF NOT EXISTS bronze_openai.costs
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `bucket_start_time` Nullable(Decimal(38, 9)),
    `bucket_end_time` Nullable(Decimal(38, 9)),
    `amount_value` Nullable(Decimal(38, 9)),
    `amount_currency` Nullable(String),
    `line_item` Nullable(String),
    `project_id` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_openai.usage_audio_speeches
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `bucket_start_time` Nullable(Decimal(38, 9)),
    `bucket_end_time` Nullable(Decimal(38, 9)),
    `characters` Nullable(Decimal(38, 9)),
    `num_model_requests` Nullable(Decimal(38, 9)),
    `project_id` Nullable(String),
    `user_id` Nullable(String),
    `model` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_openai.usage_audio_transcriptions
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `bucket_start_time` Nullable(Decimal(38, 9)),
    `bucket_end_time` Nullable(Decimal(38, 9)),
    `seconds` Nullable(Decimal(38, 9)),
    `num_model_requests` Nullable(Decimal(38, 9)),
    `project_id` Nullable(String),
    `user_id` Nullable(String),
    `model` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_openai.usage_code_interpreter
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `bucket_start_time` Nullable(Decimal(38, 9)),
    `bucket_end_time` Nullable(Decimal(38, 9)),
    `num_sessions` Nullable(Decimal(38, 9)),
    `project_id` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_openai.usage_completions
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `bucket_start_time` Nullable(Decimal(38, 9)),
    `bucket_end_time` Nullable(Decimal(38, 9)),
    `input_tokens` Nullable(Decimal(38, 9)),
    `output_tokens` Nullable(Decimal(38, 9)),
    `input_cached_tokens` Nullable(Decimal(38, 9)),
    `input_audio_tokens` Nullable(Decimal(38, 9)),
    `output_audio_tokens` Nullable(Decimal(38, 9)),
    `num_model_requests` Nullable(Decimal(38, 9)),
    `project_id` Nullable(String),
    `user_id` Nullable(String),
    `model` Nullable(String),
    `batch` Nullable(Bool),
    `service_tier` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_openai.usage_embeddings
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `bucket_start_time` Nullable(Decimal(38, 9)),
    `bucket_end_time` Nullable(Decimal(38, 9)),
    `input_tokens` Nullable(Decimal(38, 9)),
    `num_model_requests` Nullable(Decimal(38, 9)),
    `project_id` Nullable(String),
    `user_id` Nullable(String),
    `model` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_openai.usage_images
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `bucket_start_time` Nullable(Decimal(38, 9)),
    `bucket_end_time` Nullable(Decimal(38, 9)),
    `images` Nullable(Decimal(38, 9)),
    `num_model_requests` Nullable(Decimal(38, 9)),
    `project_id` Nullable(String),
    `user_id` Nullable(String),
    `model` Nullable(String),
    `size` Nullable(String),
    `source` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_openai.usage_moderations
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `bucket_start_time` Nullable(Decimal(38, 9)),
    `bucket_end_time` Nullable(Decimal(38, 9)),
    `input_tokens` Nullable(Decimal(38, 9)),
    `num_model_requests` Nullable(Decimal(38, 9)),
    `project_id` Nullable(String),
    `user_id` Nullable(String),
    `model` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_openai.usage_vector_stores
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `bucket_start_time` Nullable(Decimal(38, 9)),
    `bucket_end_time` Nullable(Decimal(38, 9)),
    `usage_bytes` Nullable(Decimal(38, 9)),
    `project_id` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_openai.users
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `id` Nullable(String),
    `object` Nullable(String),
    `email` Nullable(String),
    `name` Nullable(String),
    `role` Nullable(String),
    `added_at` Nullable(Decimal(38, 9))
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

