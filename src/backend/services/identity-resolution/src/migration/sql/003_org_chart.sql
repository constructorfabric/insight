-- Materialized SCD2 cache of parent→child relationships derived from
-- `persons` by `seed-persons-from-identity-input.py` step 9. Edges
-- come from two sources UNION-ed in the rebuild SQL:
--
--   Source 1: value_type='parent_person_id' observations (resolved
--             Insight UUIDs written by a future reconciliation service
--             — currently zero rows in the pipeline).
--   Source 2: value_type='parent_email' observations resolved by JOIN
--             to the latest value_type='email' observation per
--             (tenant, email) partition, intersected with the child's
--             active intervals derived from value_type='status'
--             observations (Active / Inactive / Terminated).
--
-- Source 1 wins on partition collision via NOT EXISTS in Source 2.
-- See ADR-0010 for the full algorithm.
--
-- Per-source-instance tree: an edge is scoped to one
-- (tenant, insight_source_type, insight_source_id) — the same person
-- may have different parents in BambooHR vs Zoom vs Slack, and that is
-- intentional.
--
-- Phase 1 invariant: at most one CURRENT parent per
-- (tenant, source_type, source_id, child_person_id), enforced by the
-- PRIMARY KEY on (..., child_person_id, valid_from) together with the
-- "at most one row with valid_to IS NULL per partition" rule the
-- rebuild SQL maintains (no DB-level UNIQUE constraint on valid_to —
-- enforcement is the rebuild's responsibility). Phase 1.5 multi-parent
-- per source would relax this by adding parent_person_id to the PK.
--
-- See ADR-0010 and docs/components/backend/identity-resolution/identity
-- /specs/DESIGN.md "Table: org_chart".
CREATE TABLE IF NOT EXISTS org_chart (
    insight_tenant_id BINARY(16) NOT NULL,
    insight_source_type VARCHAR(30) NOT NULL,
    insight_source_id BINARY(16) NOT NULL,
    child_person_id BINARY(16) NOT NULL,
    parent_person_id BINARY(16) NOT NULL,
    author_person_id BINARY(16) NOT NULL,
    reason VARCHAR(50) NOT NULL,
    valid_from TIMESTAMP(6) NOT NULL,
    valid_to TIMESTAMP(6) NULL,

    -- Phase 1: at most one CURRENT parent per (tenant, source, child).
    -- Including valid_from in the PK lets SCD2 history co-exist:
    -- multiple rows per (tenant, source, child) with different valid_from
    -- represent the chronological sequence; only one of them has
    -- valid_to IS NULL.
    PRIMARY KEY (
        insight_tenant_id, insight_source_type, insight_source_id,
        child_person_id, valid_from
    ),

    -- Self-loops are bad data. A child cannot be its own parent within
    -- the same source. The seeder also skips these (see step 9), but
    -- defence-in-depth at schema level rejects them on insert too.
    CONSTRAINT chk_no_self_loop CHECK (child_person_id <> parent_person_id),

    -- "Find the current parent of X within this source" — used by the
    -- Phase-2 endpoint to project the designated-source supervisor.
    INDEX idx_current_parent (
        insight_tenant_id, insight_source_type, insight_source_id,
        child_person_id, valid_to
    ),

    -- "Find all current children of X within this source" — used by
    -- the Phase-2 endpoint subordinates expansion and by the Phase-3
    -- `/v1/subchart/{person_id}?depth=N` recursive CTE.
    INDEX idx_current_children (
        insight_tenant_id, insight_source_type, insight_source_id,
        parent_person_id, valid_to
    ),

    -- Cross-source views: "everyone X is currently a parent of, across
    -- all source instances" and the inverse. Used to surface the new
    -- per-source detail fields in the Phase-2 response and by future
    -- analytics queries.
    INDEX idx_child_any_source  (insight_tenant_id, child_person_id, valid_to),
    INDEX idx_parent_any_source (insight_tenant_id, parent_person_id, valid_to),

    -- Temporal "as-of" queries: "who was X's parent on date Y" and the
    -- inverse. Filter is `valid_from <= @as_of AND (valid_to IS NULL OR
    -- valid_to > @as_of)`.
    INDEX idx_valid_from (insight_tenant_id, valid_from)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
