-- Unit test for `silver_watermark_key_matches`: which watermark group a key falls in.
-- Touches no table.

WITH fixtures AS (
    SELECT
        t.1 AS label,
        t.2 AS candidate_key,
        t.3 AS watermark_key,
        t.4 AS expected
    FROM (
        SELECT arrayJoin([
            ('null meets null',          CAST(NULL AS Nullable(String)), CAST(NULL AS Nullable(String)), 1),
            ('null is not empty',        CAST(NULL AS Nullable(String)), CAST(''   AS Nullable(String)), 0),
            ('empty is not null',        CAST(''   AS Nullable(String)), CAST(NULL AS Nullable(String)), 0),
            ('empty meets empty',        CAST(''   AS Nullable(String)), CAST(''   AS Nullable(String)), 1),
            ('value meets itself',       CAST('a'  AS Nullable(String)), CAST('a'  AS Nullable(String)), 1),
            ('value is not another',     CAST('a'  AS Nullable(String)), CAST('b'  AS Nullable(String)), 0),
            ('value is not null',        CAST('a'  AS Nullable(String)), CAST(NULL AS Nullable(String)), 0)
        ]) AS t
    )
)

SELECT label, candidate_key, watermark_key, expected
FROM fixtures
WHERE toUInt8({{ silver_watermark_key_matches('candidate_key', 'watermark_key') }}) != expected
