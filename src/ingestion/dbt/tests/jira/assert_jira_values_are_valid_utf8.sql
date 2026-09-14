-- Every string the journal carries must be valid UTF-8.
--
-- ClickHouse stores arbitrary bytes without complaint, and every other test
-- here passes on a broken string: the arrays stay parallel, the lengths match,
-- nothing is duplicated. The damage appears only in a consumer that decodes
-- strictly — the Rust reader dies with "incomplete utf-8 byte sequence", and an
-- API serializing the row to JSON would too.
--
-- The way to produce one is truncation. ClickHouse's `substring` counts BYTES,
-- so cutting a long-text body at N bytes splits whatever multi-byte character
-- spans the boundary; `substringUTF8` counts characters and cannot. This test
-- is what makes the difference between the two visible.
--
-- Checks the whole journal, not just long text: any future normalizer that
-- slices a string is covered by construction.

SELECT
    insight_source_id,
    id_readable,
    field_id,
    event_kind,
    arrayFilter(x -> NOT isValidUTF8(x), value_ids)      AS invalid_ids,
    arrayFilter(x -> NOT isValidUTF8(x), value_displays) AS invalid_displays
FROM {{ ref('jira__field_history_derived') }} FINAL
WHERE arrayExists(x -> NOT isValidUTF8(x), value_ids)
   OR arrayExists(x -> NOT isValidUTF8(x), value_displays)
   OR NOT isValidUTF8(field_name)
LIMIT 100
