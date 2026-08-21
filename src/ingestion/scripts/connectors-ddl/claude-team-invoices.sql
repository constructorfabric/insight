CREATE DATABASE IF NOT EXISTS `bronze_claude_team_invoices`;

CREATE TABLE IF NOT EXISTS bronze_claude_team_invoices.claude_team_invoice_lines
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `chain_status` Nullable(String),
    `invoice_ref` Nullable(String),
    `invoice_id` Nullable(String),
    `invoice_status` Nullable(String),
    `invoice_created_ts` Nullable(Decimal(38, 9)),
    `invoice_due_date_ts` Nullable(Decimal(38, 9)),
    `invoice_currency` Nullable(String),
    `invoice_total` Nullable(Decimal(38, 9)),
    `invoice_total_excluding_tax` Nullable(Decimal(38, 9)),
    `invoice_num_seats` Nullable(Decimal(38, 9)),
    `invoice_payment_intent` Nullable(String),
    `line_id` Nullable(String),
    `description` Nullable(String),
    `product_name` Nullable(String),
    `tier_label` Nullable(String),
    `category` Nullable(String),
    `is_proration` Nullable(Bool),
    `amount` Nullable(Decimal(38, 9)),
    `currency` Nullable(String),
    `quantity` Nullable(Decimal(38, 9)),
    `unit_amount` Nullable(Decimal(38, 9)),
    `seat_unit_amount` Nullable(Decimal(38, 9)),
    `period_start_ts` Nullable(Decimal(38, 9)),
    `period_end_ts` Nullable(Decimal(38, 9))
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

