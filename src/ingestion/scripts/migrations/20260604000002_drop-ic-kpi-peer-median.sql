-- =====================================================================
-- Drop insight.ic_kpi_peer_median (retired)
-- =====================================================================
-- Department KPI medians are now folded into the IC KPIs query_ref (…0010)
-- by analytics-api m20260604_000006_ic_kpis_peer_median; the KPI tiles read
-- the median off the KPI row. Nothing queries this view any more. Apply
-- after the FE cutover.
-- =====================================================================
DROP VIEW IF EXISTS insight.ic_kpi_peer_median;
