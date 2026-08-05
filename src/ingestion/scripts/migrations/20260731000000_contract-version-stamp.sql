-- Contract version stamp: the deployed version of the read-only surface the
-- Engineering layer exposes to presentation
-- (docs/domain/presentation-layer/specs/CONTRACT-SURFACE.md §2.3). The
-- analytics service pins an expected version and verifies this stamp
-- periodically. On a surface change, bump the constant in all three places
-- together: HERE (in place, not a new migration file), the snapshot copy in
-- scripts/connectors-ddl/silver.sql, and the doc's "Current version" line.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
CREATE OR REPLACE VIEW silver.contract_version AS
SELECT toUInt32(1) AS version;
