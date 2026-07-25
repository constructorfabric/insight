-- Tighten insight_source_type from VARCHAR(100) to VARCHAR(30) across
-- persons and account_person_map. The 100-char width was inherited
-- from 001_persons.sql with no data-shape justification: every
-- real-world source_type value is short (the longest in production
-- today is `claude_enterprise` = 17 chars; connectors registered so
-- far: bamboohr, zoom, slack, m365, ms-entra, github, cursor,
-- hubspot, claude_admin, claude_enterprise, claude_code, copilot,
-- onedrive, sharepoint, openai, bitwarden, confluence, salesforce).
-- VARCHAR(30) gives ~13 chars of headroom for future composite
-- names like `claude-enterprise-eu1` without leaving 80 unused.
--
-- Note on indexes: `insight_source_type` is part of UNIQUE/PRIMARY
-- keys and secondary indexes on both tables (persons.uq_person_observation,
-- persons.idx_source, account_person_map.PRIMARY KEY,
-- account_person_map.idx_current). MariaDB rebuilds them as part of
-- the ALTER. On InnoDB this is INPLACE when narrowing VARCHAR within
-- the same charset/collation; no DDL hints are pinned here so the
-- engine picks the optimal algorithm per its current capabilities.
--
-- Safety: if any existing row has insight_source_type longer than 30
-- chars, the ALTER will fail with "Data too long for column
-- 'insight_source_type'". That is intentional — the operator should
-- inspect the rogue value rather than have the migration silently
-- truncate.
--
-- Idempotency: re-running MODIFY COLUMN against an already-VARCHAR(30)
-- column is a no-op in MariaDB; the migration is safe to re-apply.
--
-- This migration is the persons+account_person_map half of the
-- tightening. The org_chart table (created in 003_org_chart.sql,
-- still in the same PR #477 and not yet applied anywhere) declares
-- insight_source_type VARCHAR(30) directly in its CREATE TABLE, so
-- there is no ALTER step for it here.

ALTER TABLE persons
    MODIFY COLUMN insight_source_type VARCHAR(30) NOT NULL;

ALTER TABLE account_person_map
    MODIFY COLUMN insight_source_type VARCHAR(30) NOT NULL;
