-- Persons schema fix: two independent corrections to the observation log
-- that surfaced while implementing person_parent_map (#348 Phase 1).
--
-- Both changes are corrections of original-design mistakes, not new
-- features. The persons table is the canonical append-only observation
-- log; the original UNIQUE and collation choices accidentally
-- contradicted that role.
--
-- A) Drop UNIQUE on (..., value_hash). Replace with (..., created_at).
--    The original UNIQUE forced same-value observations on the same
--    partition to collapse into a single row, which silently dropped
--    legitimate state transitions of the form Active -> Inactive -> Active
--    (the second 'Active' row collided with the first on value_hash and
--    INSERT IGNORE skipped it). An append-only event log must allow the
--    same value to be observed multiple times at different timestamps.
--    The new UNIQUE uses `created_at` as the natural disambiguator:
--      * Same observation re-emitted at the same created_at -> INSERT IGNORE
--        skip (re-run idempotency preserved).
--      * Different created_at -> both rows kept (transition recorded).
--
-- B) Switch value_id collation from utf8mb4_bin (case-sensitive) to
--    utf8mb4_unicode_ci (case-insensitive), matching value_full_text.
--    The original utf8mb4_bin choice made every value_id comparison
--    case-sensitive: a lookup for `jane.doe@...` would not match
--    a stored `Jane.Doe@...` even though both refer to the same
--    person. The service-side ToLowerInvariant() partially mitigated
--    this for the GET /v1/persons/{email} happy path but did nothing
--    for raw callers, for any other value_type (id / username /
--    parent_email / etc.), or for the rebuild SQL that JOINs by value.
--    With utf8mb4_unicode_ci, all value-column comparisons become
--    case-insensitive automatically; no SQL changes required.
--
--    `value_hash` stays at ascii_bin: it is a SHA-256 hex digest and
--    must remain byte-compared. Hashes themselves are not changed by
--    this migration (value_effective is unchanged byte-for-byte).
--
-- The ALTER COLUMN on value_id rewrites the table and rebuilds
-- idx_value_id under the new collation. On the dev cluster (~12k rows)
-- this is sub-second; production tables may take longer.

-- Idempotency note: each statement is safe to re-run. The first time
-- it executes on a fresh DbUp journal, it drops the old UNIQUE and
-- recreates it with the new shape. On a cluster where the same
-- migration was applied out-of-band (e.g. manual ALTER during a
-- production hot-fix), `DROP INDEX IF EXISTS` and
-- `CREATE UNIQUE INDEX IF NOT EXISTS` keep it from failing. MODIFY
-- COLUMN to the same collation is also a no-op in MariaDB.

ALTER TABLE persons DROP INDEX IF EXISTS uq_person_observation;

CREATE UNIQUE INDEX IF NOT EXISTS uq_person_observation
    ON persons (
        insight_tenant_id, person_id, insight_source_type, insight_source_id,
        value_type, created_at
    );

ALTER TABLE persons
    MODIFY COLUMN value_id
        VARCHAR(320) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL;
