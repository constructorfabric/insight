{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Working hours per day within physical bounds',
        'domain': 'hr',
        'category': 'physical_bound',
        'tier': 'warn',
        'remediation': 'A working day with 0 or more than 24 scheduled hours means the scheduled-hours mapping is missing or wrong for this person, or the default fallback did not apply. Check class_hr_working_hours and its join into class_focus_metrics.'
    }
) }}
-- `working_hours_per_day` is the denominator for focus_time_pct and dev_time_h.
-- It must be a real positive day length: greater than 0 and at most 24. A value
-- of 0 silently zeroes the denominator and almost always means scheduled hours
-- were not resolved for that person. Returns the offending (person, day) rows;
-- a violation is one or more rows.
SELECT
    insight_tenant_id,
    email,
    day,
    working_hours_per_day
FROM silver.class_focus_metrics FINAL
WHERE working_hours_per_day <= 0
   OR working_hours_per_day > 24
