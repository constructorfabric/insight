-- =====================================================================
-- wiki_bullet_rows — Gold long-format bullet rows for the wiki class
-- =====================================================================
-- Surfaces the wiki Silver classes (Confluence today; Outline later) in a
-- per-person long-format bullet view, mirroring ai_bullet_rows. Feeds the
-- "Team/IC Bullet Wiki" metric views (analytics-api m20260620_000001).
--
-- Grain: one row per (person, day, metric_key). Person key:
--   person_id = coalesce(lower(author_email), author_id)
-- author_email is resolved upstream via a Jira accountId→email join; when
-- Jira is absent (e.g. a standalone Confluence) it is NULL, so we fall back
-- to the raw author_id (Atlassian accountId) — the row still aggregates and
-- renders (team view), and lights up per-person once identity resolves.
--
-- Sources (the two STABLE wiki Silver classes):
--   • class_wiki_pages       → wiki_pages_created (1/page), wiki_edits
--       (version_count-1), wiki_active_authors (member marker)
--   • class_wiki_engagement  → wiki_comments, attributed to the page author
--       (INNER JOIN to class_wiki_pages on page_id)
-- (class_wiki_activity is intentionally NOT read here — it is the per-day
--  author rollup and proved unstable on dev; pages carries the same signal.)
--
-- Both Silver tables are ReplacingMergeTree → read with FINAL for current
-- state. Counters → sum in the metric query_ref; wiki_active_authors is a
-- 0/1 member marker (max per person, count() as the company/team range).
--
-- Idempotent: CREATE OR REPLACE VIEW. Auto-discovered by the migration
-- runner in filename order (no registry edit needed).
-- =====================================================================

CREATE OR REPLACE VIEW insight.wiki_bullet_rows AS

-- ─── Branch 1: page authorship / edits (from class_wiki_pages) ────────
SELECT
    coalesce(lower(pg.author_email), pg.author_id)          AS person_id,
    p.org_unit_id                                           AS org_unit_id,
    toDate(pg.created_at)                                   AS metric_date,
    kv.1                                                    AS metric_key,
    kv.2                                                    AS metric_value
FROM (SELECT * FROM silver.class_wiki_pages FINAL) AS pg
LEFT JOIN insight.people AS p
    ON coalesce(lower(pg.author_email), pg.author_id) = p.person_id
ARRAY JOIN [
    ('wiki_pages_created',  toFloat64(1)),
    ('wiki_edits',          toFloat64(greatest(toInt64(pg.version_count) - 1, 0))),
    ('wiki_active_authors', toFloat64(1))
] AS kv
WHERE pg.author_id IS NOT NULL AND pg.author_id != ''
  AND pg.created_at IS NOT NULL

UNION ALL

-- ─── Branch 2: engagement comments (from class_wiki_engagement) ───────
-- Attributed to the PAGE AUTHOR (engagement received on their pages).
SELECT
    coalesce(lower(pg.author_email), pg.author_id)          AS person_id,
    p.org_unit_id                                           AS org_unit_id,
    e.day                                                   AS metric_date,
    'wiki_comments'                                         AS metric_key,
    toFloat64(e.total_comments)                             AS metric_value
FROM (SELECT * FROM silver.class_wiki_engagement FINAL) AS e
INNER JOIN (SELECT * FROM silver.class_wiki_pages FINAL) AS pg
    ON e.page_id = pg.page_id AND e.tenant_id = pg.tenant_id
LEFT JOIN insight.people AS p
    ON coalesce(lower(pg.author_email), pg.author_id) = p.person_id
WHERE pg.author_id IS NOT NULL AND pg.author_id != ''
  AND e.day IS NOT NULL;
