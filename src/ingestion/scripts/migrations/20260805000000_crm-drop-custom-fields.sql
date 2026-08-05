-- Drop the CRM overflow blob from the class contract.
--
-- dbt-clickhouse incremental inserts are positional and union_by_tag is a
-- positional SELECT * UNION ALL, so physical column order must equal the
-- model's SELECT order. The CRM staging models no longer project
-- custom_fields — the connectors carry the unabridged record in raw_data
-- instead — so the column leaves the contract here in the same change.
-- DROP preserves the order of the remaining columns.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- The class tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_crm_accounts DROP COLUMN IF EXISTS custom_fields;

ALTER TABLE silver.class_crm_activities DROP COLUMN IF EXISTS custom_fields;

ALTER TABLE silver.class_crm_contacts DROP COLUMN IF EXISTS custom_fields;

ALTER TABLE silver.class_crm_deals DROP COLUMN IF EXISTS custom_fields;

ALTER TABLE silver.class_crm_users DROP COLUMN IF EXISTS custom_fields;
