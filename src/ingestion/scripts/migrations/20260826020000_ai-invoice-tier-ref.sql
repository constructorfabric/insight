-- Add the vendor's stable identifier for a priced tier to the invoice class.
--
-- Gold resolves a seat's price by binding the tier an invoice line prices to the
-- tier the seat carries, and it reads only class-contract columns to do it. The
-- display name cannot carry that binding — it is localised copy that moves
-- without notice — so the vendor's catalogue identifier becomes a column.
--
-- AFTER tier_label on both statements: dbt-clickhouse inserts positionally and
-- union_by_tag is a positional UNION ALL, so the physical order has to match the
-- staging model's SELECT. The staging half converges in apply-ch-migrations.sh —
-- that relation is absent from the snapshot until a first sync.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy. The class
-- tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_ai_invoice
    ADD COLUMN IF NOT EXISTS tier_ref Nullable(String) AFTER tier_label;

ALTER TABLE silver.class_ai_invoice
    MODIFY COLUMN tier_ref Nullable(String) AFTER tier_label;
