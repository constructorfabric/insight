-- Tenant-agnostic index for the login-bootstrap lookup
-- resolve_person_id_by_email_any_tenant (identity-resolution
-- GET /internal/persons/by-email/{email}). The existing idx_value_id starts
-- with insight_tenant_id, so a query that filters only on value_type + value_id
-- -- at login the tenant is not yet known, so it is deliberately omitted --
-- cannot use that index's leading prefix and falls back to a full table scan.
-- This index covers the tenant-agnostic email lookup directly.
CREATE INDEX IF NOT EXISTS idx_value_id_any_tenant
    ON persons (value_type, value_id);
