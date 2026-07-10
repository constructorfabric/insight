-- Build-integrity check (untagged → error severity under `dbt build`).
-- Dimension tuples are a published contract: `category` is the closed
-- gold-side taxonomy, `source` the closed git source set, and every tuple
-- carries a non-empty value and label. A violation means the category
-- macro or a staging discriminator drifted.
SELECT
    measure_key,
    dimensions,
    count() AS row_count
FROM {{ ref('git_metric_observations') }}
WHERE arrayExists(
        d -> (
            (tupleElement(d, 1) = 'category'
                AND tupleElement(d, 2) NOT IN ('code', 'test', 'config', 'docs'))
            OR (tupleElement(d, 1) = 'source'
                AND tupleElement(d, 2) NOT IN ('github', 'gitlab', 'bitbucket_cloud'))
            OR tupleElement(d, 2) = ''
            OR tupleElement(d, 3) IS NULL
            OR tupleElement(d, 3) = ''
        ),
        dimensions
    )
GROUP BY measure_key, dimensions
