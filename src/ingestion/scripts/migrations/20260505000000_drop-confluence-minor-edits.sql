-- =====================================================================
-- #260: drop major_edits/minor_edits from silver.class_wiki_activity
-- =====================================================================
--
-- The Confluence Cloud `/wiki/api/v2/pages/{id}/versions` endpoint always
-- returns `minorEdit: false` for every version regardless of how the flag
-- was set on PUT (verified empirically 2026-05-05 against both v2 and v1
-- listing endpoints). As a consequence, in production data:
--   • `class_wiki_activity.minor_edits` is uniformly 0
--   • `class_wiki_activity.major_edits` is identical to the raw version
--     count (i.e. the dead signal turns it into "total versions authored",
--     which after #259 is no longer even what `total_edits` represents)
--
-- Both columns convey no information and were dropped in PR #280.
-- This migration removes them from the existing Silver table on tenants
-- that already have `class_wiki_activity` populated. New tenants get the
-- post-drop schema directly from the dbt run.
--
-- Bronze (`bronze_confluence.wiki_page_versions.minor_edit`) is preserved
-- as-is for forward compatibility — if Atlassian fixes the API in the
-- future, re-introducing the Silver columns becomes a non-breaking
-- additive change.

ALTER TABLE silver.class_wiki_activity
    DROP COLUMN IF EXISTS major_edits;

ALTER TABLE silver.class_wiki_activity
    DROP COLUMN IF EXISTS minor_edits;
