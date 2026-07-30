-- =====================================================================
-- CRM gold views — sales-rep dashboard
-- =====================================================================
--
-- Four `insight.crm_*` views feeding the sales variant of My Dashboard
-- in `insight-front`. All read from silver (`silver.class_crm_*` +
-- `silver.class_people`); no bronze access here — that's the
-- bronze → staging (dbt) → silver (dbt) → gold (this file) contract
-- the rest of the codebase already follows.
--
--   1. `insight.crm_kpis`         — daily wide rollup (hero strip + pacing)
--   2. `insight.crm_chart_flow`   — weekly opened/closed/won (flow chart)
--   3. `insight.crm_bullet_rows`  — long-format key/value (V&Q + Activity bullets)
--   4. `insight.crm_pipeline_now` — date-less open-deal snapshot per rep
--                                   (Pipeline Now hero card)
--
-- Identity resolution
-- -------------------
-- `silver.class_crm_users` exposes two HubSpot identifiers per rep:
--   * `user_id`    — owners.id     (record-owner side; used in
--                                    `properties_hubspot_owner_id` on
--                                    deals/engagements)
--   * `hs_user_id` — owners.userId (who-logged-it side; used in
--                                    `hs_created_by_user_id` on
--                                    deals/engagements)
-- These views resolve activity attribution via `hs_user_id` — what
-- HubSpot's "Activities by user" report attributes on — falling back to
-- `user_id` (owner) only when `hs_user_id` is missing on older rows.
-- Deal facts use `user_id` (owner_id) since deal records' canonical rep
-- IS the owner. `person_id` is the lowercased rep email — same key
-- `silver.class_people` is keyed on, so bamboo `org_unit_id` /
-- `department_name` join cleanly.
--
-- Amount semantics
-- ----------------
-- Deal-amount-summing aggregates use `amount_home` (HubSpot's
-- currency-normalized field, full coverage on Constructor's bronze).
-- `acv`/`tcv`/`arr` are HubSpot-computed contract rollups but sparse
-- (~10% population); they're exposed by silver but not aggregated here.
--
-- Stock vs flow
-- -------------
-- `crm_kpis` / `crm_chart_flow` / `crm_bullet_rows` are flow metrics —
-- attributed to an event date and intersected with the period filter.
-- `crm_pipeline_now` is a stock metric — current count + $ of open deals
-- per rep as of query time. It exposes no `metric_date` column so the
-- analytics-api date-filter injection no-ops, and the FE calls it
-- without a period (`/queryMetric` with `$filter=person_id eq '…'`).
-- Earlier drafts surfaced pipeline-now inside `crm_kpis` via a
-- `range(365)` fan-out; that was dropped because it produced ~365 rows
-- per rep per query for a constant value. Split-out keeps the snapshot
-- O(reps) instead of O(reps × 365).
--
-- ReplacingMergeTree + FINAL
-- --------------------------
-- All `silver.class_crm_*` materializations use
-- ReplacingMergeTree(_version). Background merges aren't synchronous, so
-- a `SELECT` without `FINAL` can see un-merged duplicates. We use
-- `FINAL` on every silver read here — small extra cost on a per-rep
-- dashboard query; precision is worth more than a few ms. Each view
-- hoists `silver.class_crm_deals FINAL` (and where applicable
-- `silver.class_crm_activities FINAL`) into a single `*_dedup` CTE that
-- downstream CTEs read from, so the version-resolve pass runs once per
-- view regardless of how many CTEs reference deals.
--
-- Identity-resolution / `is_active`
-- ---------------------------------
-- Each view's `owners` CTE intentionally does NOT filter `is_active`:
-- a rep who's currently archived may still own historical deals and
-- engagements that need to aggregate correctly under their email key.
-- `person_id` is `lower(email)` on both the owners and people side — the
-- silver model writes `email` as-is (HubSpot/Bamboo preserve case), so
-- the `lower()` on both join sides is what guarantees the key matches.
-- =====================================================================

CREATE DATABASE IF NOT EXISTS insight;

-- ---------------------------------------------------------------------
-- 1. insight.crm_kpis — daily wide rollup
-- ---------------------------------------------------------------------
CREATE OR REPLACE VIEW insight.crm_kpis AS
WITH
  -- Single dedup pass per silver source — referenced by all downstream
  -- CTEs so FINAL runs once regardless of how many CTEs read deals /
  -- activities. See header for full rationale.
  deals_dedup AS (
    SELECT * FROM silver.class_crm_deals FINAL
  ),
  activities_dedup AS (
    SELECT * FROM silver.class_crm_activities FINAL
  ),
  -- Owner adapter: HubSpot ID space → canonical email key.
  -- Two parallel projections so deal-side (`user_id`) and engagement-side
  -- (`hs_user_id`) joins each get a narrow lookup table.
  owners AS (
    SELECT user_id, hs_user_id, lower(assumeNotNull(email)) AS person_id
    FROM silver.class_crm_users FINAL
    WHERE email IS NOT NULL AND email != ''
  ),
  -- Bamboo department for `org_unit_id`. HubSpot Owners API exposes no
  -- title/department; we cross-reference by email into class_people.
  people AS (
    SELECT lower(assumeNotNull(email)) AS person_id,
           coalesce(department_name, 'Unknown') AS org_unit_id
    FROM silver.class_people FINAL
    WHERE email IS NOT NULL AND email != ''
  ),
  -- Flow: deals OPENED in period (attributed to createdAt date).
  opened_by_day AS (
    SELECT
      toDate(d.created_at)          AS metric_date,
      o.person_id                   AS person_id,
      count()                       AS deals_opened,
      toUInt64(0)                   AS deals_closed,
      toUInt64(0)                   AS deals_won,
      toFloat64(0)                  AS deals_value_closed,
      toUInt64(0)                   AS comms_count
    FROM deals_dedup d
    INNER JOIN owners o ON o.user_id = d.owner_id
    WHERE d.created_at IS NOT NULL
    GROUP BY metric_date, person_id
  ),
  -- Flow: deals CLOSED in period (attributed to close_date).
  closed_by_day AS (
    SELECT
      d.close_date                                      AS metric_date,
      o.person_id                                       AS person_id,
      toUInt64(0)                                       AS deals_opened,
      count()                                           AS deals_closed,
      countIf(d.is_won = 1)                             AS deals_won,
      sumIf(coalesce(d.amount_home, 0), d.is_won = 1)   AS deals_value_closed,
      toUInt64(0)                                       AS comms_count
    FROM deals_dedup d
    INNER JOIN owners o ON o.user_id = d.owner_id
    WHERE d.is_closed = 1 AND d.close_date IS NOT NULL
    GROUP BY metric_date, person_id
  ),
  -- Flow: engagements per day. Per-activity-type attribution to match
  -- HubSpot Reports semantics: `calls` use the record owner (HubSpot
  -- never sets `hs_created_by_user_id` on call records); emails /
  -- meetings / tasks use the creator (the rep who actually logged the
  -- activity), with NO owner fallback — records lacking a creator
  -- don't roll up to anyone (mirrors what HubSpot's "Activities by
  -- user" report does, otherwise contact-owned inbound emails inflate
  -- per-rep counts by ~30%).
  comms_by_day AS (
    SELECT
      toDate(a.timestamp)                                            AS metric_date,
      if(a.activity_type = 'call',
         nullIf(by_owner.person_id, ''),
         nullIf(by_user.person_id, ''))                              AS person_id,
      toUInt64(0)                                                     AS deals_opened,
      toUInt64(0)                                                     AS deals_closed,
      toUInt64(0)                                                     AS deals_won,
      toFloat64(0)                                                    AS deals_value_closed,
      count()                                                         AS comms_count
    FROM activities_dedup a
    LEFT JOIN owners by_user  ON by_user.hs_user_id = a.created_by_user_id
    LEFT JOIN owners by_owner ON by_owner.user_id    = a.owner_id
    WHERE a.timestamp IS NOT NULL
      AND if(a.activity_type = 'call',
             nullIf(by_owner.person_id, ''),
             nullIf(by_user.person_id, '')) IS NOT NULL
    GROUP BY metric_date, person_id
  ),
  unioned AS (
    SELECT * FROM opened_by_day
    UNION ALL SELECT * FROM closed_by_day
    UNION ALL SELECT * FROM comms_by_day
  )
SELECT
  u.metric_date                                  AS metric_date,
  u.person_id                                    AS person_id,
  coalesce(p.org_unit_id, 'Unknown')             AS org_unit_id,
  coalesce(p.org_unit_id, 'Unknown')             AS org_unit_name,
  sum(u.deals_opened)                            AS deals_opened,
  sum(u.deals_closed)                            AS deals_closed,
  sum(u.deals_won)                               AS deals_won,
  sum(u.deals_value_closed)                      AS deals_value_closed,
  sum(u.comms_count)                             AS comms_count
FROM unioned u
LEFT JOIN people p ON p.person_id = u.person_id
WHERE u.metric_date IS NOT NULL
GROUP BY u.metric_date, u.person_id, p.org_unit_id;


-- ---------------------------------------------------------------------
-- 2. insight.crm_pipeline_now — date-less open-deal snapshot per rep
-- ---------------------------------------------------------------------
-- Powers the "Pipeline Now" hero card. No `metric_date` column — the
-- analytics-api date-filter injection skips views without one, and the
-- FE calls this without a period.
CREATE OR REPLACE VIEW insight.crm_pipeline_now AS
WITH
  deals_dedup AS (
    SELECT * FROM silver.class_crm_deals FINAL
  ),
  owners AS (
    SELECT user_id, lower(assumeNotNull(email)) AS person_id
    FROM silver.class_crm_users FINAL
    WHERE email IS NOT NULL AND email != ''
  ),
  people AS (
    SELECT lower(assumeNotNull(email)) AS person_id,
           coalesce(department_name, 'Unknown') AS org_unit_id
    FROM silver.class_people FINAL
    WHERE email IS NOT NULL AND email != ''
  )
SELECT
  o.person_id                                          AS person_id,
  coalesce(p.org_unit_id, 'Unknown')                   AS org_unit_id,
  countIf(d.is_closed = 0)                             AS pipeline_count,
  round(sumIf(coalesce(d.amount_home, 0), d.is_closed = 0)) AS pipeline_value
FROM deals_dedup d
INNER JOIN owners o ON o.user_id = d.owner_id
LEFT JOIN people p  ON p.person_id = o.person_id
GROUP BY o.person_id, p.org_unit_id;


-- ---------------------------------------------------------------------
-- 3. insight.crm_chart_flow — weekly opened/closed/won
-- ---------------------------------------------------------------------
CREATE OR REPLACE VIEW insight.crm_chart_flow AS
WITH
  deals_dedup AS (
    SELECT * FROM silver.class_crm_deals FINAL
  ),
  owners AS (
    SELECT user_id, lower(assumeNotNull(email)) AS person_id
    FROM silver.class_crm_users FINAL
    WHERE email IS NOT NULL AND email != ''
  ),
  people AS (
    SELECT lower(assumeNotNull(email)) AS person_id,
           coalesce(department_name, 'Unknown') AS org_unit_id
    FROM silver.class_people FINAL
    WHERE email IS NOT NULL AND email != ''
  ),
  opened_w AS (
    SELECT toMonday(toDate(d.created_at))   AS week_start,
           o.person_id                       AS person_id,
           count()                           AS opened,
           toUInt64(0)                       AS closed,
           toUInt64(0)                       AS won
    FROM deals_dedup d
    INNER JOIN owners o ON o.user_id = d.owner_id
    WHERE d.created_at IS NOT NULL
    GROUP BY week_start, person_id
  ),
  closed_w AS (
    SELECT toMonday(d.close_date)            AS week_start,
           o.person_id                        AS person_id,
           toUInt64(0)                        AS opened,
           count()                            AS closed,
           countIf(d.is_won = 1)              AS won
    FROM deals_dedup d
    INNER JOIN owners o ON o.user_id = d.owner_id
    WHERE d.is_closed = 1 AND d.close_date IS NOT NULL
    GROUP BY week_start, person_id
  ),
  unioned AS (
    SELECT * FROM opened_w UNION ALL SELECT * FROM closed_w
  )
SELECT
  u.person_id                                  AS person_id,
  coalesce(p.org_unit_id, 'Unknown')           AS org_unit_id,
  formatDateTime(u.week_start, '%b %d')        AS date_bucket,
  toString(u.week_start)                       AS metric_date,
  toUInt64(sum(u.opened))                       AS opened,
  toUInt64(sum(u.closed))                       AS closed,
  toUInt64(sum(u.won))                          AS won
FROM unioned u
LEFT JOIN people p ON p.person_id = u.person_id
WHERE u.week_start IS NOT NULL
GROUP BY u.person_id, p.org_unit_id, u.week_start;


-- ---------------------------------------------------------------------
-- 4. insight.crm_bullet_rows — long-format key/value
-- ---------------------------------------------------------------------
-- One row per source event with `metric_key` discriminator. Catalog
-- `query_ref`s (CRM_BULLET_QUALITY, CRM_BULLET_ACTIVITY) ARRAY-JOIN this
-- into the bullet metrics the FE renders.
CREATE OR REPLACE VIEW insight.crm_bullet_rows AS
WITH
  deals_dedup AS (
    SELECT * FROM silver.class_crm_deals FINAL
  ),
  activities_dedup AS (
    SELECT * FROM silver.class_crm_activities FINAL
  ),
  owners AS (
    SELECT user_id, hs_user_id, lower(assumeNotNull(email)) AS person_id
    FROM silver.class_crm_users FINAL
    WHERE email IS NOT NULL AND email != ''
  ),
  people AS (
    SELECT lower(assumeNotNull(email)) AS person_id,
           coalesce(department_name, 'Unknown') AS org_unit_id
    FROM silver.class_people FINAL
    WHERE email IS NOT NULL AND email != ''
  ),
  -- Deal-side rows (one per deal event):
  opened_rows AS (
    SELECT
      toDate(d.created_at)                AS metric_date,
      o.person_id                          AS person_id,
      coalesce(p.org_unit_id, 'Unknown')   AS org_unit_id,
      'deals_opened'                       AS metric_key,
      toFloat64(1)                         AS metric_value
    FROM deals_dedup d
    INNER JOIN owners o ON o.user_id = d.owner_id
    LEFT JOIN people p ON p.person_id = o.person_id
    WHERE d.created_at IS NOT NULL
  ),
  closed_rows AS (
    SELECT
      d.close_date                         AS metric_date,
      o.person_id                          AS person_id,
      coalesce(p.org_unit_id, 'Unknown')   AS org_unit_id,
      'deals_closed'                       AS metric_key,
      toFloat64(1)                         AS metric_value
    FROM deals_dedup d
    INNER JOIN owners o ON o.user_id = d.owner_id
    LEFT JOIN people p ON p.person_id = o.person_id
    WHERE d.is_closed = 1 AND d.close_date IS NOT NULL
  ),
  won_rows AS (
    SELECT
      d.close_date                         AS metric_date,
      o.person_id                          AS person_id,
      coalesce(p.org_unit_id, 'Unknown')   AS org_unit_id,
      'deals_won'                          AS metric_key,
      toFloat64(1)                         AS metric_value
    FROM deals_dedup d
    INNER JOIN owners o ON o.user_id = d.owner_id
    LEFT JOIN people p ON p.person_id = o.person_id
    WHERE d.is_won = 1 AND d.close_date IS NOT NULL
  ),
  -- `cycle_days` per won deal. Floored at 0: HubSpot's close_date is UTC
  -- ISO while created_at carries an offset, producing -1 for some
  -- same-day closes. Same-day close (0) is legitimate.
  cycle_rows AS (
    SELECT
      d.close_date                         AS metric_date,
      o.person_id                          AS person_id,
      coalesce(p.org_unit_id, 'Unknown')   AS org_unit_id,
      'cycle_days'                         AS metric_key,
      toFloat64(greatest(0, dateDiff('day', toDate(d.created_at), d.close_date)))  AS metric_value
    FROM deals_dedup d
    INNER JOIN owners o ON o.user_id = d.owner_id
    LEFT JOIN people p ON p.person_id = o.person_id
    WHERE d.is_won = 1 AND d.close_date IS NOT NULL AND d.created_at IS NOT NULL
  ),
  size_rows AS (
    SELECT
      d.close_date                         AS metric_date,
      o.person_id                          AS person_id,
      coalesce(p.org_unit_id, 'Unknown')   AS org_unit_id,
      'deal_size'                          AS metric_key,
      -- toFloat64 to match the other UNION branches; amount_home is Decimal (#1708)
      toFloat64(coalesce(d.amount_home, 0)) AS metric_value
    FROM deals_dedup d
    INNER JOIN owners o ON o.user_id = d.owner_id
    LEFT JOIN people p ON p.person_id = o.person_id
    WHERE d.is_won = 1 AND d.close_date IS NOT NULL AND d.amount_home IS NOT NULL
  ),
  -- Engagement rows. Per-activity-type attribution (see kpis view for
  -- full rationale): calls via owner (HubSpot never sets a creator on
  -- call records); emails/meetings/tasks via creator with no owner
  -- fallback (matches HubSpot's "Activities by user" report).
  activity_rows AS (
    SELECT
      toDate(a.timestamp)                                     AS metric_date,
      if(a.activity_type = 'call',
         nullIf(by_owner.person_id, ''),
         nullIf(by_user.person_id, ''))                       AS person_id,
      coalesce(p.org_unit_id, 'Unknown')                       AS org_unit_id,
      multiIf(
        a.activity_type = 'call',     'calls',
        a.activity_type = 'email',    'emails',
        a.activity_type = 'meeting',  'meetings',
        a.activity_type = 'task',     'tasks',
        a.activity_type
      )                                                        AS metric_key,
      toFloat64(1)                                             AS metric_value
    FROM activities_dedup a
    LEFT JOIN owners by_user  ON by_user.hs_user_id = a.created_by_user_id
    LEFT JOIN owners by_owner ON by_owner.user_id    = a.owner_id
    LEFT JOIN people p
           ON p.person_id = if(a.activity_type = 'call',
                               nullIf(by_owner.person_id, ''),
                               nullIf(by_user.person_id, ''))
    WHERE a.timestamp IS NOT NULL
      AND if(a.activity_type = 'call',
             nullIf(by_owner.person_id, ''),
             nullIf(by_user.person_id, '')) IS NOT NULL
  )
SELECT * FROM opened_rows
UNION ALL SELECT * FROM closed_rows
UNION ALL SELECT * FROM won_rows
UNION ALL SELECT * FROM cycle_rows
UNION ALL SELECT * FROM size_rows
UNION ALL SELECT * FROM activity_rows;


