{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'The month that just closed was read close to its end',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A row here is a connector instance whose last seat-spend read of the month that just closed happened hours before the month ended. The seat endpoint reports the month in progress and carries no history, so everything spent after that read is lost permanently — a re-sync cannot recover it, because the endpoint now answers for the current month. Check that the sync ran on the last day of the month and succeeded: the schedule is meant to place the final read minutes before the boundary, and a single failed run there costs a day of spend. Past rows do not clear; they record a month that is already short.'
    }
) }}
{#- Only the month that has just closed is judged. An open month is still
    being read, and an older one cannot be acted on any more than it can be
    re-read, so flagging either would produce noise nobody can clear.

    Six hours is the line: the schedule puts the last read ten minutes before
    the boundary, and one failed run on the last day makes it a full day. -#}

WITH toStartOfMonth(today()) AS current_month,
     current_month - INTERVAL 1 MONTH AS closed_month

{# `closed_month` is NOT aliased to `period_month` below: a SELECT alias
   shadows the column of the same name, the WHERE would compare the alias
   with itself, and every month would collapse into one group whose latest
   read is the open month's — silently inverting the check.

   Untrimmed delimiters on purpose: this comment stands between two SQL
   tokens, and a trimming one would glue `closed_month` to `SELECT`. #}
SELECT
    insight_tenant_id,
    source_id,
    source,
    closed_month                                                   AS billing_month,
    max(collected_at)                                              AS last_read,
    dateDiff('hour', max(collected_at), toDateTime(current_month)) AS hours_unread
FROM {{ ref('class_ai_overage') }} FINAL
WHERE period_month = closed_month
GROUP BY
    insight_tenant_id,
    source_id,
    source
HAVING hours_unread > 6
