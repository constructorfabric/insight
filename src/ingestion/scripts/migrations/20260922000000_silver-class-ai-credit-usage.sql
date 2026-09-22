-- A NEW class relation, not a widened one, and gold reads it in the same
-- deploy that introduces it. The deploy hook builds tag:gold and not the
-- tag:silver model that would create this table, so on a warm installation
-- ai_cost_metric_evidence resolves silver.class_ai_credit_usage before anything
-- has created it and the whole hook fails with UNKNOWN_TABLE. A fresh install
-- never shows this: there the snapshot creates every relation first.
--
-- Column order is the contract. union_by_tag is a positional UNION ALL, so this
-- must match chatgpt_team__ai_credit_usage's SELECT order exactly, and any
-- contributor added later must match it too.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
--
CREATE TABLE IF NOT EXISTS silver.class_ai_credit_usage
(
    `insight_tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `email` Nullable(String),
    `day` Date,
    `tool` LowCardinality(String),
    `credits` Decimal(18, 6),
    `credit_kind` LowCardinality(String),
    `source` LowCardinality(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `_version` UInt64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1;
