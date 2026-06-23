-- =====================================================================
-- Drop insight.peer_cohort_stats (retired)
-- =====================================================================
-- The per-bullet cohort distribution (p25/p50/p75/min/max/n) is now carried
-- on each bullet row by the analytics-api *_bullet_distribution query_refs
-- (m20260604_00000{1..5}). Coloring + drilldowns read row.peer; nothing
-- queries this view any more. Apply after the FE cutover.
-- =====================================================================
DROP VIEW IF EXISTS insight.peer_cohort_stats;
