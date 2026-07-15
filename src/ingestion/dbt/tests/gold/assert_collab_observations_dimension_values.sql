-- Build-integrity check (untagged → error severity under `dbt build`).
-- Dimension tuples are a published contract: `tool` is the closed
-- collaboration source set, `scope` the closed recipient-scope set, and
-- every tuple carries a non-empty value and label. A violation means the
-- tool label macro, the scope literals, or a staging data_source
-- discriminator drifted.
SELECT
    measure_key,
    dimensions,
    count() AS row_count
FROM {{ ref('collab_metric_observations') }}
WHERE arrayExists(
        d -> (
            (tupleElement(d, 1) = 'tool'
                AND tupleElement(d, 2) NOT IN ('m365', 'slack', 'zoom', 'zulip_proxy'))
            OR (tupleElement(d, 1) = 'scope'
                AND tupleElement(d, 2) NOT IN ('internal', 'external'))
            OR tupleElement(d, 2) = ''
            OR tupleElement(d, 3) IS NULL
            OR tupleElement(d, 3) = ''
        ),
        dimensions
    )
GROUP BY measure_key, dimensions
