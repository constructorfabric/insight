{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Collab document activity spike vs per-user trailing baseline',
        'domain': 'collab',
        'category': 'anomaly',
        'tier': 'warn',
        'remediation': 'A recent user-day activity count is far above the recent history for that same user, product and metric (z-score > 5 AND > 10x the trailing mean over the last 28 observed days, with at least 14 days of history and a baseline that is itself recent). Not necessarily wrong — it can be a genuine burst — but a sudden 100x-style jump usually means a feeder double-count, a unit change, or a backfill landing on one date. Inspect the stored rows, compare value against base_mean, and trace the m365 feeder. Thresholds are heuristic and advisory (warn only); tune them once observed against real tenant data.'
    }
) }}
-- Statistical spike guard for collaboration document activity (#1321 follow-up).
-- Catches "too high" anomalies the non-negative check cannot: a count that is
-- plausible in isolation but wildly out of line with the same user's own recent
-- history. Deliberately NOT a fixed upper limit — the baseline is derived per
-- (tenant, user, product, metric) from each series' own trailing window, so it
-- adapts to heavy and light users alike instead of one global number.
--
-- Bounded by design, so the cost stays flat as history grows:
--   * input is capped to the last 120 days (the `recent` CTE). A 28-observed-day
--     baseline for any reasonably active user fits well inside 120 calendar days,
--     so the window sort/scan never grows with total history. A user with fewer
--     than 14 active days in that window simply isn't evaluated (too little
--     signal), which the prior_n guard below enforces.
--   * output is capped to rows collected in the last 3 days (collected_at, the
--     row arrival time -- not activity date), so a scheduled daily run reports
--     newly-arrived spikes once, and a late backfill landing today for an older
--     activity date is still caught instead of being silently dropped.
--
-- Method: unpivot the five activity counts, then for each series compute a
-- trailing baseline over the previous 28 observed days (excluding the current
-- day). A day is flagged only when ALL of these hold:
--   * at least 14 prior observations exist   -- cold-start guard, no baseline yet
--   * the baseline has non-zero spread        -- avoids divide-by-noise on flat series
--   * the most recent prior observation is within 35 days  -- baseline is not stale
--   * value > mean + 5 * stddev               -- z-score outlier
--   * value > 10 * mean                        -- massive relative jump
-- The window is ROWS-based (observed days, not calendar days), so without the
-- staleness guard a user returning from a long leave would be compared against a
-- months-old baseline and could false-fire; `date - prev_date <= 35` prevents
-- that. Advisory only (severity=warn): it emits a finding and stores the rows, it
-- never fails the pipeline. Read FINAL so transient ReplacingMergeTree duplicates
-- can't look like spikes.
WITH recent AS (
    SELECT
        tenant_id,
        insight_source_id,
        person_key,
        product,
        data_source,
        date,
        collected_at,
        viewed_or_edited_count,
        synced_count,
        shared_internally_count,
        shared_externally_count,
        visited_page_count
    FROM {{ ref('class_collab_document_activity') }} FINAL
    WHERE date >= today() - 120
),
unpivoted AS (
    SELECT
        tenant_id,
        insight_source_id,
        person_key,
        product,
        data_source,
        date,
        collected_at,
        m.1 AS metric,
        m.2 AS value
    FROM recent
    ARRAY JOIN
        [
            ('viewed_or_edited_count',  CAST(viewed_or_edited_count  AS Nullable(Float64))),
            ('synced_count',            CAST(synced_count            AS Nullable(Float64))),
            ('shared_internally_count', CAST(shared_internally_count AS Nullable(Float64))),
            ('shared_externally_count', CAST(shared_externally_count AS Nullable(Float64))),
            ('visited_page_count',      CAST(visited_page_count      AS Nullable(Float64)))
        ] AS m
    WHERE m.2 IS NOT NULL
),
baselined AS (
    SELECT
        tenant_id,
        insight_source_id,
        person_key,
        product,
        data_source,
        date,
        collected_at,
        metric,
        value,
        count()           OVER w AS prior_n,
        avg(value)        OVER w AS base_mean,
        stddevSamp(value) OVER w AS base_sd,
        max(date)         OVER w AS prev_date
    FROM unpivoted
    WINDOW w AS (
        PARTITION BY tenant_id, insight_source_id, person_key, product, data_source, metric
        ORDER BY date
        ROWS BETWEEN 28 PRECEDING AND 1 PRECEDING
    )
)
SELECT
    tenant_id,
    insight_source_id,
    person_key,
    product,
    data_source,
    date,
    metric,
    value,
    prior_n,
    base_mean,
    base_sd
FROM baselined
WHERE prior_n >= 14
  AND base_sd > 0
  AND (date - prev_date) <= 35
  AND value > base_mean + 5 * base_sd
  AND value > base_mean * 10
  AND collected_at >= today() - 3
