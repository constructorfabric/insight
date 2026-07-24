-- Path B for /v1/subchart forest endpoint (#344 follow-up):
-- org_chart becomes "every person's CURRENT position in the hierarchy"
-- rather than "edges only". Top-of-tree persons (no parent) and
-- isolated source members (singletons) now get a row with
-- parent_person_id = NULL, which makes "find all tops" a property of
-- a row (`WHERE parent_person_id IS NULL`) instead of a set-difference
-- across the edge table.
--
-- The CHECK relaxation is null-safe: self-loops are still rejected,
-- but a NULL parent is allowed (it can't equal anything, including
-- the child).
--
-- The persons-seed rebuild ports change in tandem (Sql.PersonsSeed.cs
-- InsertOrgChartForTenant adds a UNION ALL for the no-parent rows);
-- the next seed run populates the new rows.
--
-- RUST-PORT DIVERGENCE from the .NET 012 (the one deliberate edit in the
-- copied set): DROP/ADD are IF EXISTS / IF NOT EXISTS guarded. MariaDB DDL
-- is not transactional and the ledger row is written only after the whole
-- script — a crash between the plain DROP and ADD left a re-run failing on
-- "Can't DROP CONSTRAINT". The guards make every interleaving/retry
-- converge (also against the frozen .NET DbUp replaying its own 012 on a
-- fresh side-by-side install).
ALTER TABLE org_chart
    MODIFY COLUMN parent_person_id BINARY(16) NULL;

ALTER TABLE org_chart
    DROP CONSTRAINT IF EXISTS chk_no_self_loop;

ALTER TABLE org_chart
    ADD CONSTRAINT IF NOT EXISTS chk_no_self_loop
    CHECK (parent_person_id IS NULL OR child_person_id <> parent_person_id);
