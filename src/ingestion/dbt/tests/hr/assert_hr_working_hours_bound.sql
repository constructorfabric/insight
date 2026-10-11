{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Working hours per day within physical bounds',
        'domain': 'hr',
        'category': 'physical_bound',
        'tier': 'warn',
        'remediation': 'A scheduled day below 0 or above 24 hours means the scheduled-hours mapping is wrong for this person, for example weekly hours stored as daily. Check the HR source behind class_hr_working_hours.'
    }
) }}
-- INVARIANT: gold divides by these hours for focus_time_pct and falls back to 8 only
-- when they are not positive, so a value above 24 reaches the metric unflagged.
SELECT
    insight_tenant_id,
    email,
    working_hours_per_day
FROM silver.class_hr_working_hours FINAL
WHERE working_hours_per_day < 0
   OR working_hours_per_day > 24
