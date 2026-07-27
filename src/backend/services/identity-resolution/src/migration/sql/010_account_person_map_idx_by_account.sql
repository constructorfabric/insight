-- Index for caller resolution from JWT id claims (oid / sub) against
-- account_person_map. The existing idx_current starts with
-- (insight_tenant_id, insight_source_type, insight_source_id,
--  source_account_id, valid_to), so a query that only knows the
-- source_account_id cannot use the leading prefix and falls back to a
-- full table scan. The new index covers the JWT lookup directly.
CREATE INDEX IF NOT EXISTS idx_by_account
    ON account_person_map (insight_tenant_id, source_account_id, valid_to);
