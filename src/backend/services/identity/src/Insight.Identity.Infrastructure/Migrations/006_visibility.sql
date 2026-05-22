-- Visibility grants — explicit scopes that let one person see another
-- (and the org_chart subtree rooted at the target). Combined with the
-- `org_chart` SCD2 cache at query time, this drives the "can A see B?"
-- predicate that gates GET /v1/persons and POST /v1/profiles.
--
-- Semantics (decided in #346 design rev 3.1):
--   viewed_person_id IS NULL  → viewer sees the whole tenant tree
--   viewed_person_id IS NOT NULL → viewer sees subtree(viewed_person_id)
--     unioned with subtree(viewer) and any other active grants
--
-- Roles ≠ visibility. The `admin` role gates CRUD on this table; it
-- does NOT grant visibility — admins still need explicit rows here to
-- see persons outside their own org_chart subtree.
--
-- Append-only for changes: the only mutating column post-INSERT is
-- `valid_to` (set to UTC_TIMESTAMP(6) on soft-delete). New grants =
-- INSERT new row; revoked grants = UPDATE valid_to.
CREATE TABLE IF NOT EXISTS visibility (
    visibility_id     BINARY(16) NOT NULL,
    insight_tenant_id BINARY(16) NOT NULL,
    viewer_person_id  BINARY(16) NOT NULL,
    viewed_person_id  BINARY(16) NULL,
    valid_from        DATETIME(6) NOT NULL,
    valid_to          DATETIME(6) NULL,
    author_person_id  BINARY(16) NOT NULL,
    reason            VARCHAR(500) NULL,
    created_at        DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),

    PRIMARY KEY (visibility_id),

    -- Reject backward intervals at schema level: a row with
    -- valid_to < valid_from would be silently invisible to the
    -- "active = valid_to IS NULL" query, masking caller-side bugs.
    CONSTRAINT chk_visibility_interval
        CHECK (valid_to IS NULL OR valid_from <= valid_to),

    -- Hot path: "what are A's active grants?" — drives the visibility
    -- CTE seed list when checking can_see(A, B). `viewed_person_id` is
    -- a covering column so the index satisfies the SELECT shape from
    -- `Sql.Visibility.cs::ActiveGrantsByViewer` without a back-lookup
    -- into the clustered index. The valid_to suffix lets the index
    -- also serve historical queries (valid_to <= @as_of).
    INDEX idx_viewer_current
        (insight_tenant_id, viewer_person_id, valid_to, viewed_person_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
