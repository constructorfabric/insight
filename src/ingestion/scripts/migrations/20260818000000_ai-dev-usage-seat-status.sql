-- Add seat_status to the AI dev-usage class contract.
--
-- A pre-existing table got it from nowhere: the DDL snapshot is IF NOT EXISTS,
-- and dbt appends it (append_new_columns) only when the silver model runs —
-- after the deploy whose gold build already reads it (ai_metric_evidence).
--
-- AFTER _version is last, the position the staging projections use and the
-- positional insert requires; MODIFY converges an instance where an
-- out-of-band ALTER placed it elsewhere. Shape follows
-- 20260716000000_class_contract_heal.sql.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- The class tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_ai_dev_usage
    ADD COLUMN IF NOT EXISTS seat_status Nullable(String) AFTER _version;

ALTER TABLE silver.class_ai_dev_usage
    MODIFY COLUMN seat_status Nullable(String) AFTER _version;
