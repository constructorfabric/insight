-- An interval that cannot have happened.
--
-- Three shapes, and the second is why this test exists. ClickHouse answers an
-- ASOF LEFT JOIN that matched nothing with the column TYPE'S DEFAULT rather
-- than NULL, so a link that was never removed reads as removed at the epoch —
-- a date, which every range query believes. The same failure has already been
-- shipped twice in this repository under a different aggregate, so it is
-- guarded here rather than reasoned about.
--
--   1. `valid_to` at or before `valid_from` — the link ended before it began.
--   2. `valid_to` at the epoch — the unmatched-join default leaking through.
--   3. `valid_from` at the epoch while `valid_from_known = 1` — the row claims
--      the vendor stated a moment that is the sentinel for "unknown".

SELECT
    insight_source_id,
    data_source,
    id_readable,
    link_type,
    target_readable,
    valid_from,
    valid_to,
    valid_from_known,
    multiIf(
        valid_to IS NOT NULL AND valid_to <= valid_from, 'ends before it begins',
        valid_to = toDateTime64(0, 3),                   'closed at the epoch: an unmatched join default',
        'valid_from is the unknown sentinel but is marked known'
    ) AS finding
FROM {{ ref('class_task_links') }} FINAL
WHERE (valid_to IS NOT NULL AND valid_to <= valid_from)
   OR valid_to = toDateTime64(0, 3)
   OR (valid_from = toDateTime64(0, 3) AND valid_from_known = 1)
LIMIT 100
