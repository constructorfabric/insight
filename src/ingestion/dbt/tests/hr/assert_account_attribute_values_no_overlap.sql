-- Build-integrity guard on the gold interval builder: intervals for one
-- (tenant, source type, source instance, account, field) must be disjoint.
-- Open intervals participate via a far-future sentinel so two open intervals,
-- or an open interval preceding a later valid_from, fail here rather than
-- NULL-propagate to a silent pass.
SELECT *
FROM (
    SELECT
        insight_tenant_id,
        insight_source_type,
        insight_source_id,
        source_account_id,
        field_id,
        valid_from,
        valid_to,
        max(coalesce(valid_to, toDateTime64('2106-01-01 00:00:00', 3))) OVER (
            PARTITION BY
                insight_tenant_id,
                insight_source_type,
                insight_source_id,
                source_account_id,
                field_id
            ORDER BY valid_from
            ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING
        ) AS prev_valid_to,
        ROW_NUMBER() OVER (
            PARTITION BY
                insight_tenant_id,
                insight_source_type,
                insight_source_id,
                source_account_id,
                field_id
            ORDER BY valid_from
        ) AS interval_seq
    FROM {{ ref('account_attribute_values') }}
)
WHERE interval_seq > 1
  AND valid_from < prev_valid_to
